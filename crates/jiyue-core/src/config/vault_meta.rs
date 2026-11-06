//! The name of a vault as the user sees it: a display name stored in the vault's own
//! `.jiyue/config.toml` (`[vault] name`), and the rules for the names typed in the 笔记库 screens.
//! The display name defaults to the folder name. Renaming the folder itself is a separate action
//! (see `folder_rename_target`); the display name survives it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{VAULT_CONFIG_FILE, load_section, save_section, templates, vault_file};

/// Longest display name, in characters.
pub const MAX_DISPLAY_NAME_CHARS: usize = 60;
/// Longest folder name, in characters.
pub const MAX_FOLDER_NAME_CHARS: usize = 120;
/// Section of the vault config that holds the name.
pub const VAULT_SECTION: &str = "vault";

/// `[vault]` section of `<vault>/.jiyue/config.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VaultMeta {
    /// Display name. Empty means the folder name is used.
    pub name: String,
}

/// The display name stored for the vault at `root`, trimmed. Empty when none is set (or the file
/// is missing or broken).
pub fn read_name(root: &Path) -> String {
    let loaded: super::Loaded<VaultMeta> =
        load_section(&vault_file(root, VAULT_CONFIG_FILE), VAULT_SECTION);
    loaded.value.name.trim().to_string()
}

/// Stores `name` as the display name of the vault at `root`, keeping the rest of its config.
pub fn write_name(root: &Path, name: &str) -> Result<(), String> {
    let name = check_display_name(name).map_err(str::to_string)?;
    let meta = VaultMeta { name };
    save_section(
        &vault_file(root, VAULT_CONFIG_FILE),
        VAULT_SECTION,
        &meta,
        templates::VAULT_CONFIG,
        &[],
    )
}

/// The name shown for a vault: its display name when set, otherwise the folder name.
pub fn shown_name(display_name: &str, folder_name: &str) -> String {
    let display = display_name.trim();
    if display.is_empty() {
        folder_name.to_string()
    } else {
        display.to_string()
    }
}

/// Checks a display name typed by the user. Returns the trimmed name or a hint in Chinese.
pub fn check_display_name(raw: &str) -> Result<String, &'static str> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("名称不能为空");
    }
    if name.chars().count() > MAX_DISPLAY_NAME_CHARS {
        return Err("名称最多 60 个字");
    }
    if name.chars().any(char::is_control) {
        return Err("名称不能包含换行或控制字符");
    }
    Ok(name.to_string())
}

/// Characters Windows does not allow in a file name.
const WINDOWS_INVALID: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
/// Device names Windows reserves, with or without an extension.
const WINDOWS_RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Checks a new folder name against the rules of Windows (the app runs there too). Returns the
/// trimmed name or a hint in Chinese.
pub fn check_folder_name(raw: &str) -> Result<String, &'static str> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("文件夹名不能为空");
    }
    if name == "." || name == ".." {
        return Err("文件夹名不能是 . 或 ..");
    }
    if name
        .chars()
        .any(|c| WINDOWS_INVALID.contains(&c) || c.is_control())
    {
        return Err("文件夹名不能包含 < > : \" / \\ | ? * 或控制字符");
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Err("文件夹名不能以点或空格结尾");
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if WINDOWS_RESERVED.contains(&stem.as_str()) {
        return Err("这是 Windows 保留名称，请换一个");
    }
    if name.chars().count() > MAX_FOLDER_NAME_CHARS {
        return Err("文件夹名最多 120 个字");
    }
    Ok(name.to_string())
}

/// The folder a vault moves to when its folder is renamed to `new_name` (a sibling of `root`).
/// Refused for the filesystem root, an invalid name, the same name, and a folder that already
/// exists. A change of letter case alone is allowed (Windows keeps one folder for both).
pub fn folder_rename_target(root: &Path, new_name: &str) -> Result<PathBuf, String> {
    let name = check_folder_name(new_name).map_err(str::to_string)?;
    let Some(parent) = root.parent() else {
        return Err("笔记库位于磁盘根目录，不能重命名文件夹".to_string());
    };
    let current = root.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if current == name {
        return Err("名称没有改变".to_string());
    }
    let target = parent.join(&name);
    if target.exists() && !current.eq_ignore_ascii_case(&name) {
        return Err(format!("同名文件夹已存在：{}", target.display()));
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jiyue-vmeta-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".jiyue")).expect("mkdir");
        dir
    }

    #[test]
    fn display_name_rules() {
        assert_eq!(
            check_display_name("  霁月的笔记  "),
            Ok("霁月的笔记".to_string())
        );
        assert!(check_display_name("   ").is_err());
        assert!(check_display_name("a\tb").is_err());
        assert!(check_display_name(&"字".repeat(60)).is_ok());
        assert_eq!(
            check_display_name(&"字".repeat(61)),
            Err("名称最多 60 个字")
        );
    }

    #[test]
    fn shown_name_falls_back_to_the_folder() {
        assert_eq!(shown_name("", "notes"), "notes");
        assert_eq!(shown_name("  ", "notes"), "notes");
        assert_eq!(shown_name(" 工作 ", "notes"), "工作");
    }

    #[test]
    fn folder_name_rules_follow_windows() {
        assert_eq!(check_folder_name(" 我的笔记 "), Ok("我的笔记".to_string()));
        assert!(check_folder_name("").is_err());
        assert!(check_folder_name("..").is_err());
        for bad in ["a/b", "a\\b", "a:b", "a*b", "a?b", "a\"b", "a<b", "a|b"] {
            assert!(check_folder_name(bad).is_err(), "{bad}");
        }
        assert!(check_folder_name("name.").is_err());
        assert!(check_folder_name("CON").is_err());
        assert!(check_folder_name("lpt9.txt").is_err());
        assert!(
            check_folder_name("console").is_ok(),
            "only the exact device name is reserved"
        );
    }

    #[test]
    fn display_name_round_trips_and_keeps_other_settings() {
        let root = scratch("roundtrip");
        let cfg = vault_file(&root, VAULT_CONFIG_FILE);
        std::fs::write(
            &cfg,
            "version = 1\n# 我的备注\n[editor]\nauto_name_from_heading = false\n",
        )
        .expect("write");
        assert_eq!(read_name(&root), "");
        write_name(&root, "  新名字 ").expect("save");
        assert_eq!(read_name(&root), "新名字");
        let text = std::fs::read_to_string(&cfg).expect("read");
        assert!(text.contains("# 我的备注"), "{text}");
        assert!(text.contains("auto_name_from_heading = false"), "{text}");
        assert!(text.contains("[vault]"), "{text}");
        assert!(write_name(&root, "").is_err());
        assert_eq!(read_name(&root), "新名字", "a refused name changes nothing");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn broken_vault_config_gives_no_name_and_is_not_overwritten() {
        let root = scratch("broken");
        let cfg = vault_file(&root, VAULT_CONFIG_FILE);
        std::fs::write(&cfg, "version = 1\n[vault\nname = \"x\"\n").expect("write");
        assert_eq!(read_name(&root), "");
        assert!(write_name(&root, "新").is_err());
        assert_eq!(
            std::fs::read_to_string(&cfg).expect("read"),
            "version = 1\n[vault\nname = \"x\"\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn folder_rename_target_refuses_bad_cases() {
        let base = scratch("folder");
        let root = base.join("笔记");
        std::fs::create_dir_all(&root).expect("mkdir");
        std::fs::create_dir_all(base.join("已有")).expect("mkdir");
        assert_eq!(
            folder_rename_target(&root, "新笔记"),
            Ok(base.join("新笔记"))
        );
        assert!(folder_rename_target(&root, "笔记").is_err(), "same name");
        assert!(folder_rename_target(&root, "已有").is_err(), "exists");
        assert!(folder_rename_target(&root, "a:b").is_err(), "invalid");
        assert!(
            folder_rename_target(Path::new("C:\\"), "x").is_err(),
            "root"
        );
        // A change of case alone is a rename of the same folder.
        std::fs::create_dir_all(base.join("Case")).expect("mkdir");
        assert!(folder_rename_target(&base.join("Case"), "case").is_ok());
        let _ = std::fs::remove_dir_all(&base);
    }
}
