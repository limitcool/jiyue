//! Self-updating module for Jiyue (霁月).
//!
//! Provides clean, lightweight auto-update checks via GitHub Releases API, asset matching
//! for the current operating system and architecture, stream downloading with progress reports,
//! SHA-256 verification against `SHA256SUMS`, and silent in-place installation on Windows.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const DEFAULT_REPO_OWNER: &str = "limitcool";
pub const DEFAULT_REPO_NAME: &str = "jiyue";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Release asset descriptor returned by GitHub API.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
    pub size: u64,
}

/// Release metadata returned by GitHub API.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GithubRelease {
    pub tag_name: String,
    pub name: Option<String>,
    pub body: Option<String>,
    pub published_at: Option<String>,
    #[serde(default)]
    pub assets: Vec<ReleaseAsset>,
    #[serde(default)]
    pub prerelease: bool,
}

/// Information about an available update targeted for the current platform.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateInfo {
    pub version: String,
    pub release_notes: String,
    pub published_at: Option<String>,
    pub download_url: String,
    pub file_name: String,
    pub file_size: u64,
    pub sha256_url: Option<String>,
}

/// Reactive update state tracked in UI.
#[derive(Clone, Debug, PartialEq)]
pub enum UpdateState {
    /// Idle, no check performed yet.
    Idle,
    /// Currently querying GitHub for new releases.
    Checking,
    /// Up to date. Holds unix epoch seconds of last check.
    UpToDate { checked_at: u64 },
    /// New version available to download.
    Available(UpdateInfo),
    /// Currently downloading installer.
    Downloading {
        version: String,
        progress: f32,
        downloaded: u64,
        total: u64,
    },
    /// Download complete and verified, ready to restart.
    Ready {
        version: String,
        installer_path: PathBuf,
    },
    /// Error encountered during check or download.
    Failed(String),
}

// ---------------------------------------------------------------------------
// Pure logic: SemVer comparison & Asset matching
// ---------------------------------------------------------------------------

/// Parses a SemVer triple `(major, minor, patch)` from a version string (e.g. "0.1.1", "v0.1.2").
pub fn parse_semver(s: &str) -> Option<(u64, u64, u64)> {
    let clean = s.trim().trim_start_matches(['v', 'V']);
    let mut parts = clean.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch_str = parts.next()?.split(['-', '+']).next()?;
    let patch = patch_str.parse::<u64>().ok()?;
    Some((major, minor, patch))
}

/// Returns true if `candidate_tag` represents a newer version than `current_version`.
pub fn is_newer(candidate_tag: &str, current_version: &str) -> bool {
    let Some(candidate) = parse_semver(candidate_tag) else {
        return false;
    };
    let Some(current) = parse_semver(current_version) else {
        return false;
    };
    candidate > current
}

/// Selects the best release asset matching `os` and `arch`.
pub fn pick_asset<'a>(
    assets: &'a [ReleaseAsset],
    os: &str,
    arch: &str,
) -> Option<&'a ReleaseAsset> {
    match os {
        "windows" => {
            let is_arm = arch == "aarch64" || arch == "arm64";
            assets.iter().find(|a| {
                let lower = a.name.to_lowercase();
                lower.ends_with(".msi")
                    && if is_arm {
                        lower.contains("arm64")
                    } else {
                        lower.contains("x64") || lower.contains("x86_64")
                    }
            })
        }
        "macos" => {
            let is_arm = arch == "aarch64" || arch == "arm64";
            assets.iter().find(|a| {
                let lower = a.name.to_lowercase();
                lower.ends_with(".dmg")
                    && if is_arm {
                        lower.contains("aarch64") || lower.contains("arm64")
                    } else {
                        lower.contains("x64") || lower.contains("x86_64")
                    }
            })
        }
        "linux" => assets.iter().find(|a| {
            let lower = a.name.to_lowercase();
            lower.ends_with(".appimage") || lower.ends_with(".deb")
        }),
        _ => None,
    }
}

/// Finds the `SHA256SUMS` checksums asset in the release if present.
pub fn find_checksums_asset(assets: &[ReleaseAsset]) -> Option<&ReleaseAsset> {
    assets.iter().find(|a| {
        let lower = a.name.to_lowercase();
        lower == "sha256sums" || lower == "sha256sum" || lower.ends_with(".sha256")
    })
}

/// Extracts the expected 64-character SHA-256 hex string for `file_name` from `SHA256SUMS` text.
pub fn parse_checksum(sha256sums_content: &str, file_name: &str) -> Option<String> {
    for line in sha256sums_content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            let hash = parts[0];
            let name = parts[1].trim_start_matches('*');
            if name.eq_ignore_ascii_case(file_name) && hash.len() == 64 {
                return Some(hash.to_lowercase());
            }
        }
    }
    None
}

/// Computes the SHA-256 hash of a local file and compares it with `expected_hex`.
pub fn verify_file_sha256(path: &Path, expected_hex: &str) -> Result<bool, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("打开文件失败: {e}"))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| format!("计算哈希失败: {e}"))?;
    let result = hasher.finalize();
    let actual_hex = format!("{:x}", result);
    Ok(actual_hex.eq_ignore_ascii_case(expected_hex))
}

// ---------------------------------------------------------------------------
// Network & Update Actions
// ---------------------------------------------------------------------------

/// Queries GitHub Releases API for the latest version.
pub fn check_for_update(
    owner: &str,
    repo: &str,
    current_version: &str,
) -> Result<Option<UpdateInfo>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent(format!("Jiyue-App/{}", current_version))
        .build()
        .map_err(|e| format!("创建网络客户端失败: {e}"))?;

    let url = format!("https://api.github.com/repos/{owner}/{repo}/releases/latest");
    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .map_err(|e| format!("检查更新连接失败: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        // If GitHub API rate limit is exceeded (HTTP 403 or 429), fallback to web redirection check
        if status.as_u16() == 403 || status.as_u16() == 429 {
            return check_latest_tag_via_web(owner, repo, current_version);
        }
        return Err(format!("检查更新请求失败 (HTTP {})", status));
    }

    let release: GithubRelease = resp.json().map_err(|e| format!("解析版本信息失败: {e}"))?;

    if !is_newer(&release.tag_name, current_version) {
        return Ok(None);
    }

    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let Some(asset) = pick_asset(&release.assets, os, arch) else {
        return Err(format!(
            "发现新版本 {}，但未找到适用于当前平台 ({}-{}) 的安装包",
            release.tag_name, os, arch
        ));
    };

    let sha256_url = find_checksums_asset(&release.assets).map(|a| a.browser_download_url.clone());
    let clean_version = release.tag_name.trim_start_matches(['v', 'V']).to_string();

    Ok(Some(UpdateInfo {
        version: clean_version,
        release_notes: release.body.unwrap_or_default(),
        published_at: release.published_at,
        download_url: asset.browser_download_url.clone(),
        file_name: asset.name.clone(),
        file_size: asset.size,
        sha256_url,
    }))
}

/// Fallback update check querying GitHub Releases Web endpoint (bypasses GitHub REST API 60 req/hr rate limit).
pub fn check_latest_tag_via_web(
    owner: &str,
    repo: &str,
    current_version: &str,
) -> Result<Option<UpdateInfo>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(format!("Jiyue-App/{}", current_version))
        .build()
        .map_err(|e| format!("创建网络客户端失败: {e}"))?;

    let url = format!("https://github.com/{owner}/{repo}/releases/latest");
    let resp = client
        .get(&url)
        .send()
        .map_err(|e| format!("检查更新连接失败: {e}"))?;

    let status = resp.status();
    if status.is_redirection()
        && let Some(loc) = resp.headers().get("Location").and_then(|v| v.to_str().ok())
        && let Some(tag) = loc.split("/tag/").nth(1)
    {
        let tag = tag.trim_matches('/');
        if !is_newer(tag, current_version) {
            return Ok(None);
        }

        let clean_version = tag.trim_start_matches(['v', 'V']).to_string();
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        let (file_name, _) = default_asset_name_for(os, arch, &clean_version);
        let download_url =
            format!("https://github.com/{owner}/{repo}/releases/download/{tag}/{file_name}");
        let sha256_url = Some(format!(
            "https://github.com/{owner}/{repo}/releases/download/{tag}/SHA256SUMS"
        ));

        return Ok(Some(UpdateInfo {
            version: clean_version,
            release_notes: format!(
                "请前往 https://github.com/{owner}/{repo}/releases/tag/{tag} 查看更新日志。"
            ),
            published_at: None,
            download_url,
            file_name,
            file_size: 0,
            sha256_url,
        }));
    }

    Err(format!(
        "GitHub API 访问频次受限 (HTTP 403 Rate Limit)，且网页备用检查失败 (HTTP {})",
        status
    ))
}

/// Default asset filename and package format for a target platform.
pub fn default_asset_name_for(os: &str, arch: &str, version: &str) -> (String, &'static str) {
    match (os, arch) {
        ("windows", "x86_64") => (format!("jiyue-{version}-x64-setup.msi"), "msi"),
        ("windows", "aarch64") => (format!("jiyue-{version}-arm64-setup.msi"), "msi"),
        ("macos", "aarch64") => (format!("jiyue-{version}-arm64.dmg"), "dmg"),
        ("macos", "x86_64") => (format!("jiyue-{version}-x64.dmg"), "dmg"),
        ("linux", "x86_64") => (format!("jiyue_{version}_amd64.AppImage"), "appimage"),
        _ => (format!("jiyue-{version}-{os}-{arch}.zip"), "zip"),
    }
}

/// Returns the local cache folder where update installers are saved.
pub fn update_cache_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            return PathBuf::from(local_app_data)
                .join("Jiyue")
                .join("cache")
                .join("updates");
        }
    }
    std::env::temp_dir().join("jiyue-updates")
}

/// Downloads the installer described by `info` into `cache_dir`, streaming progress via callback.
pub fn download_installer<F>(info: &UpdateInfo, progress: F) -> Result<PathBuf, String>
where
    F: Fn(f32, u64, u64),
{
    let cache_dir = update_cache_dir();
    std::fs::create_dir_all(&cache_dir).map_err(|e| format!("创建更新目录失败: {e}"))?;

    let final_path = cache_dir.join(&info.file_name);
    let part_path = cache_dir.join(format!("{}.part", info.file_name));

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(300))
        .user_agent(format!("Jiyue-App/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("创建下载客户端失败: {e}"))?;

    // Download checksums if available
    let expected_hash = if let Some(sums_url) = &info.sha256_url {
        match client.get(sums_url).send() {
            Ok(resp) if resp.status().is_success() => {
                let content = resp.text().unwrap_or_default();
                parse_checksum(&content, &info.file_name)
            }
            _ => None,
        }
    } else {
        None
    };

    // If file already exists and passes checksum, reuse it directly!
    if final_path.exists()
        && let Some(ref hash) = expected_hash
        && let Ok(true) = verify_file_sha256(&final_path, hash)
    {
        progress(1.0, info.file_size, info.file_size);
        return Ok(final_path);
    }

    let mut resp = client
        .get(&info.download_url)
        .send()
        .map_err(|e| format!("下载安装包失败: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("下载请求失败 (HTTP {})", resp.status()));
    }

    let total_size = resp.content_length().unwrap_or(info.file_size);
    let mut file =
        std::fs::File::create(&part_path).map_err(|e| format!("创建临时文件失败: {e}"))?;

    use std::io::Read;
    let mut buffer = [0u8; 16384]; // 16KB buffer
    let mut downloaded: u64 = 0;

    loop {
        let n = resp
            .read(&mut buffer)
            .map_err(|e| format!("读取下载数据失败: {e}"))?;
        if n == 0 {
            break;
        }
        use std::io::Write;
        file.write_all(&buffer[..n])
            .map_err(|e| format!("写入文件失败: {e}"))?;
        downloaded += n as u64;
        let p = if total_size > 0 {
            (downloaded as f32 / total_size as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };
        progress(p, downloaded, total_size);
    }

    file.sync_all().map_err(|e| format!("同步文件失败: {e}"))?;
    drop(file);

    // Verify hash if available
    if let Some(ref hash) = expected_hash {
        let valid =
            verify_file_sha256(&part_path, hash).map_err(|e| format!("校验哈希错误: {e}"))?;
        if !valid {
            let _ = std::fs::remove_file(&part_path);
            return Err("下载文件的 SHA-256 哈希校验不通过，可能文件已损坏或不完整".to_string());
        }
    }

    // Rename part file to final destination
    if final_path.exists() {
        let _ = std::fs::remove_file(&final_path);
    }
    std::fs::rename(&part_path, &final_path).map_err(|e| format!("保存最终文件失败: {e}"))?;

    Ok(final_path)
}

/// Applies the update and restarts Jiyue.
/// On Windows, triggers silent MSI installation and relaunches the app.
pub fn apply_update_and_restart(installer_path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        let current_exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let pid = std::process::id();
        let script = format!(
            "Start-Sleep -Milliseconds 600; \
             Wait-Process -Id {pid} -Timeout 5 -ErrorAction SilentlyContinue; \
             Start-Process -FilePath 'msiexec.exe' -ArgumentList '/i', '\"{msi}\"', '/qn', '/norestart' -Wait; \
             Start-Process -FilePath '\"{exe}\"'",
            pid = pid,
            msi = installer_path.display(),
            exe = current_exe.display()
        );
        std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &script])
            .spawn()
            .map_err(|e| format!("启动静默升级进程失败: {e}"))?;
        std::process::exit(0);
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(installer_path)
            .spawn()
            .map_err(|e| format!("打开安装镜像失败: {e}"))?;
        std::process::exit(0);
    }

    #[cfg(target_os = "linux")]
    {
        // For AppImage or debian packages
        if installer_path.extension().and_then(|s| s.to_str()) == Some("AppImage") {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(installer_path)
                    .map_err(|e| e.to_string())?
                    .permissions();
                perms.set_mode(0o755);
                let _ = std::fs::set_permissions(installer_path, perms);
            }
            std::process::Command::new(installer_path)
                .spawn()
                .map_err(|e| format!("启动新版本失败: {e}"))?;
            std::process::exit(0);
        } else {
            return Err(format!(
                "安装包已下载至 {}，请在终端中使用包管理器安装",
                installer_path.display()
            ));
        }
    }

    #[allow(unreachable_code)]
    Ok(())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_parsing_and_comparison() {
        assert_eq!(parse_semver("0.1.0"), Some((0, 1, 0)));
        assert_eq!(parse_semver("v0.1.2"), Some((0, 1, 2)));
        assert_eq!(parse_semver("v1.20.3-beta"), Some((1, 20, 3)));
        assert_eq!(parse_semver("invalid"), None);

        assert!(is_newer("v0.1.2", "0.1.1"));
        assert!(is_newer("v0.2.0", "0.1.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("v0.1.1", "0.1.1"));
        assert!(!is_newer("v0.1.0", "0.1.1"));
    }

    #[test]
    fn asset_picking_by_os_and_arch() {
        let assets = vec![
            ReleaseAsset {
                name: "Jiyue-0.1.2-x64-zh-CN.msi".into(),
                browser_download_url: "https://example.com/win-x64.msi".into(),
                size: 15_000_000,
            },
            ReleaseAsset {
                name: "Jiyue-0.1.2-arm64-zh-CN.msi".into(),
                browser_download_url: "https://example.com/win-arm64.msi".into(),
                size: 14_000_000,
            },
            ReleaseAsset {
                name: "Jiyue-0.1.2-aarch64.dmg".into(),
                browser_download_url: "https://example.com/mac-arm.dmg".into(),
                size: 20_000_000,
            },
            ReleaseAsset {
                name: "Jiyue-0.1.2-x86_64.AppImage".into(),
                browser_download_url: "https://example.com/linux.AppImage".into(),
                size: 25_000_000,
            },
            ReleaseAsset {
                name: "SHA256SUMS".into(),
                browser_download_url: "https://example.com/SHA256SUMS".into(),
                size: 512,
            },
        ];

        let win_x64 = pick_asset(&assets, "windows", "x86_64");
        assert_eq!(
            win_x64.map(|a| a.name.as_str()),
            Some("Jiyue-0.1.2-x64-zh-CN.msi")
        );

        let win_arm = pick_asset(&assets, "windows", "aarch64");
        assert_eq!(
            win_arm.map(|a| a.name.as_str()),
            Some("Jiyue-0.1.2-arm64-zh-CN.msi")
        );

        let mac = pick_asset(&assets, "macos", "aarch64");
        assert_eq!(
            mac.map(|a| a.name.as_str()),
            Some("Jiyue-0.1.2-aarch64.dmg")
        );

        let linux = pick_asset(&assets, "linux", "x86_64");
        assert_eq!(
            linux.map(|a| a.name.as_str()),
            Some("Jiyue-0.1.2-x86_64.AppImage")
        );

        let sums = find_checksums_asset(&assets);
        assert_eq!(sums.map(|a| a.name.as_str()), Some("SHA256SUMS"));
    }

    #[test]
    fn parse_checksums_content() {
        let content = r#"
# Release checksums
e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  Jiyue-0.1.2-x64-zh-CN.msi
ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad *Jiyue-0.1.2-aarch64.dmg
"#;
        assert_eq!(
            parse_checksum(content, "Jiyue-0.1.2-x64-zh-CN.msi"),
            Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into())
        );
        assert_eq!(
            parse_checksum(content, "Jiyue-0.1.2-aarch64.dmg"),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into())
        );
        assert_eq!(parse_checksum(content, "nonexistent.file"), None);
    }

    #[test]
    fn online_check_with_rate_limit_fallback() {
        // Even when GitHub REST API returns 403 rate limit, web fallback succeeds and finds current version is up to date
        let res = check_for_update("limitcool", "jiyue", env!("CARGO_PKG_VERSION"));
        assert!(matches!(res, Ok(None)), "Got result: {:?}", res);
    }
}
