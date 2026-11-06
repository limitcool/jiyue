//! Importing files (pasted images, dropped files) into a vault.
//!
//! [`plan_import`] decides the destination path, the link text and whether an identical file
//! already exists. [`write_import`] writes the bytes atomically and never overwrites a
//! different file. Nothing here touches the filesystem except `write_import`.

use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Where imported files are stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttachFolder {
    /// The vault root.
    VaultRoot,
    /// A fixed vault-relative folder, for example `附件`.
    Fixed(String),
    /// The folder of the note the file is imported into.
    SameAsNote,
    /// A folder named below the note's folder.
    SubfolderOfNote(String),
}

/// How the inserted link is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkStyle {
    /// `![[name.png]]`, the Obsidian style.
    WikiEmbed,
    /// `![alt](relative/path.png)` for images, `[name](relative/path.pdf)` otherwise.
    Markdown,
}

/// Attachment settings for a vault. Missing fields take their defaults when deserialized.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AttachSettings {
    pub folder: AttachFolder,
    pub link_style: LinkStyle,
    /// Name prefix for files without a name, such as clipboard images. The timestamp follows.
    pub default_pasted_prefix: String,
}

impl Default for AttachSettings {
    fn default() -> Self {
        Self {
            folder: AttachFolder::Fixed("附件".to_string()),
            link_style: LinkStyle::WikiEmbed,
            default_pasted_prefix: "Pasted image".to_string(),
        }
    }
}

/// A local date and time used for generated names. Formatted as `YYYYMMDD-HHMMSS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp {
    pub y: u32,
    pub m: u32,
    pub d: u32,
    pub hh: u32,
    pub mm: u32,
    pub ss: u32,
}

/// The outcome of planning an import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportPlan {
    /// Vault-relative destination with `/` separators.
    pub dest_rel: String,
    /// The Markdown or wiki snippet to insert into the note.
    pub link_text: String,
    /// True when an identical file already exists at `dest_rel`. Nothing needs writing.
    pub reuse_existing: bool,
}

const MAX_STEM_BYTES: usize = 120;
const MAX_SUFFIX: u32 = 10_000;

// ---------------------------------------------------------------------------
// Names
// ---------------------------------------------------------------------------

fn is_reserved(s: &str) -> bool {
    let base = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    if matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    let bytes = base.as_bytes();
    bytes.len() == 4
        && (base.starts_with("COM") || base.starts_with("LPT"))
        && (b'1'..=b'9').contains(&bytes[3])
}

fn truncate_bytes(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Makes one path component safe on Windows and in Markdown links.
///
/// Forbidden characters (`<>:"/\|?*`, plus `#[]^` which break wiki link targets) become `_`.
/// Control characters and all whitespace become single spaces. Leading and trailing dots and
/// spaces are removed. Device names (`CON`, `COM1`, ...) get a `_` prefix. The result is at
/// most 120 bytes and never splits a character. CJK and emoji are kept.
pub fn sanitize_component(s: &str) -> String {
    let mut mapped = String::with_capacity(s.len());
    for c in s.chars() {
        let out = if c.is_control() || c.is_whitespace() {
            ' '
        } else if matches!(
            c,
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | '#' | '[' | ']' | '^'
        ) {
            '_'
        } else {
            c
        };
        if out == ' ' && mapped.ends_with(' ') {
            continue;
        }
        mapped.push(out);
    }
    let trimmed = mapped.trim_matches(|c: char| c == ' ' || c == '.');
    let mut out = truncate_bytes(trimmed, MAX_STEM_BYTES)
        .trim_end_matches([' ', '.'])
        .to_string();
    if is_reserved(&out) {
        out.insert(0, '_');
    }
    out
}

/// Splits a relative folder path into sanitized components. `.`, `..`, empty and
/// unusable components are dropped, so `../../x` becomes `["x"]`.
fn sanitize_rel_dir(p: &str) -> Vec<String> {
    p.split(['/', '\\'])
        .filter(|c| !c.is_empty() && *c != "." && *c != "..")
        .map(sanitize_component)
        .filter(|c| !c.is_empty())
        .collect()
}

/// The folder components of a note path (its own folder, not the file name).
fn note_dir(note_rel_path: &str) -> Vec<String> {
    let mut parts: Vec<&str> = note_rel_path.split(['/', '\\']).collect();
    parts.pop();
    parts
        .into_iter()
        .filter(|c| !c.is_empty() && *c != "." && *c != "..")
        .map(sanitize_component)
        .filter(|c| !c.is_empty())
        .collect()
}

/// Splits an original file name into a sanitized stem and a lower-case extension.
/// The extension is kept only when it is 1 to 10 ASCII letters or digits.
fn split_name(name: &str) -> (String, Option<String>) {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("").trim();
    match base.rfind('.') {
        Some(dot) if dot > 0 => {
            let ext = &base[dot + 1..];
            if (1..=10).contains(&ext.len()) && ext.bytes().all(|c| c.is_ascii_alphanumeric()) {
                (
                    sanitize_component(&base[..dot]),
                    Some(ext.to_ascii_lowercase()),
                )
            } else {
                (sanitize_component(base), None)
            }
        }
        _ => (sanitize_component(base), None),
    }
}

/// Extension family: formats that share an extension group compare equal.
fn family(ext: &str) -> &str {
    match ext {
        "jpeg" | "jpe" => "jpg",
        "tif" => "tiff",
        "m4v" | "mov" => "mp4",
        "m4b" => "m4a",
        "mkv" => "webm",
        "oga" | "opus" => "ogg",
        other => other,
    }
}

/// Recognises a file type from its leading bytes. Returns the canonical extension.
pub fn sniff_ext(bytes: &[u8]) -> Option<&'static str> {
    let len = bytes.len();
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some("png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("jpg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("gif");
    }
    if bytes.starts_with(b"%PDF-") {
        return Some("pdf");
    }
    if bytes.starts_with(b"OggS") {
        return Some("ogg");
    }
    if bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Some("webm");
    }
    if len >= 12 && bytes.starts_with(b"RIFF") {
        return match &bytes[8..12] {
            b"WEBP" => Some("webp"),
            b"WAVE" => Some("wav"),
            _ => None,
        };
    }
    if len >= 12 && &bytes[4..8] == b"ftyp" {
        return match &bytes[8..12] {
            b"M4A " | b"M4B " => Some("m4a"),
            b"qt  " => Some("mov"),
            _ => Some("mp4"),
        };
    }
    if len >= 6 && bytes.starts_with(b"BM") {
        let declared = u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]) as usize;
        if declared == len {
            return Some("bmp");
        }
    }
    if bytes.starts_with(b"ID3") {
        return Some("mp3");
    }
    if len >= 2 && bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0 && ((bytes[1] >> 1) & 0x03) != 0 {
        return Some("mp3");
    }
    if is_svg(bytes) {
        return Some("svg");
    }
    None
}

fn is_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(2048)];
    if head.contains(&0) {
        return false;
    }
    let text = String::from_utf8_lossy(head);
    let t = text.trim_start_matches('\u{feff}').trim_start();
    (t.starts_with("<svg") || t.starts_with("<?xml") || t.starts_with("<!DOCTYPE svg"))
        && t.contains("<svg")
}

fn decide_ext(orig: Option<String>, sniffed: Option<&str>) -> String {
    match (orig, sniffed) {
        (Some(o), Some(s)) if family(&o) == family(s) => o,
        (_, Some(s)) => s.to_string(),
        (Some(o), None) => o,
        (None, None) => "bin".to_string(),
    }
}

fn stamp(now: Timestamp) -> String {
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        now.y, now.m, now.d, now.hh, now.mm, now.ss
    )
}

fn is_image_name(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "jpe" | "gif" | "webp" | "bmp" | "svg" | "avif"
    )
}

fn encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Relative, percent-encoded path from a note's folder to a file in `folder`.
fn relative_link(from: &[String], target_dir: &[String], file: &str) -> String {
    let mut target: Vec<&str> = target_dir.iter().map(String::as_str).collect();
    target.push(file);
    let common = from
        .iter()
        .zip(target.iter())
        .take_while(|(a, b)| a.as_str() == **b)
        .count();
    let mut parts: Vec<String> = Vec::new();
    for _ in common..from.len() {
        parts.push("..".to_string());
    }
    for seg in &target[common..] {
        parts.push(encode_segment(seg));
    }
    parts.join("/")
}

// ---------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------

/// Plans the import of `bytes` into the vault.
///
/// - `note_rel_path`: the note the file is inserted into, vault-relative.
/// - `original_name`: the source file name, or `None` for clipboard data.
/// - `existing_files(path)`: returns the content of the vault file at `path`, or `None` when
///   there is no such file. Used for name collisions and identical-content reuse.
///
/// Name collisions get ` 1`, ` 2`, ... before the extension. If a file with identical content
/// already exists under one of those names, it is reused. Identical files under other names
/// are not detected; that would need a vault-wide hash index.
///
/// This function assumes the file name is unique in the vault, so the wiki link uses the name
/// alone. Use [`plan_import_checked`] when the caller knows about name clashes.
pub fn plan_import(
    settings: &AttachSettings,
    note_rel_path: &str,
    original_name: Option<&str>,
    bytes: &[u8],
    now: Timestamp,
    existing_files: &dyn Fn(&str) -> Option<Vec<u8>>,
) -> ImportPlan {
    plan_import_checked(
        settings,
        note_rel_path,
        original_name,
        bytes,
        now,
        existing_files,
        &|_| false,
    )
}

/// Like [`plan_import`], with `name_taken(file_name)`: true when another file in the vault,
/// other than the destination, has that file name. Wiki links then use the full vault path.
pub fn plan_import_checked(
    settings: &AttachSettings,
    note_rel_path: &str,
    original_name: Option<&str>,
    bytes: &[u8],
    now: Timestamp,
    existing_files: &dyn Fn(&str) -> Option<Vec<u8>>,
    name_taken: &dyn Fn(&str) -> bool,
) -> ImportPlan {
    let note = note_dir(note_rel_path);
    let folder: Vec<String> = match &settings.folder {
        AttachFolder::VaultRoot => Vec::new(),
        AttachFolder::Fixed(p) => sanitize_rel_dir(p),
        AttachFolder::SameAsNote => note.clone(),
        AttachFolder::SubfolderOfNote(sub) => {
            let mut v = note.clone();
            v.extend(sanitize_rel_dir(sub));
            v
        }
    };

    let (mut stem, orig_ext) = match original_name.map(split_name) {
        Some((stem, ext)) => (stem, ext),
        None => (String::new(), None),
    };
    if stem.is_empty() {
        let prefix = sanitize_component(&settings.default_pasted_prefix);
        let prefix = if prefix.is_empty() {
            "Pasted image".to_string()
        } else {
            prefix
        };
        stem = format!("{prefix} {}", stamp(now));
    }
    let ext = decide_ext(orig_ext, sniff_ext(bytes));
    let hash = blake3::hash(bytes);

    let join = |file: &str| -> String {
        let mut parts: Vec<&str> = folder.iter().map(String::as_str).collect();
        parts.push(file);
        parts.join("/")
    };

    let mut chosen: Option<(String, String, bool)> = None;
    for n in 0..MAX_SUFFIX {
        let file = if n == 0 {
            format!("{stem}.{ext}")
        } else {
            format!("{stem} {n}.{ext}")
        };
        let dest = join(&file);
        match existing_files(&dest) {
            None => {
                chosen = Some((dest, file, false));
                break;
            }
            Some(content) => {
                if blake3::hash(&content) == hash {
                    chosen = Some((dest, file, true));
                    break;
                }
            }
        }
    }
    let (dest_rel, file, reuse_existing) = chosen.unwrap_or_else(|| {
        let hex = hash.to_hex().to_string();
        let short = hex.get(..8).unwrap_or("hash");
        let file = format!("{stem} {short}.{ext}");
        (join(&file), file, false)
    });

    let link_text = match settings.link_style {
        LinkStyle::WikiEmbed => {
            let target = if name_taken(&file) {
                dest_rel.clone()
            } else {
                file.clone()
            };
            format!("![[{target}]]")
        }
        LinkStyle::Markdown => {
            let rel = relative_link(&note, &folder, &file);
            if is_image_name(&file) {
                format!("![{stem}]({rel})")
            } else {
                format!("[{file}]({rel})")
            }
        }
    };

    ImportPlan {
        dest_rel,
        link_text,
        reuse_existing,
    }
}

/// Pixel size of a PNG, JPEG, GIF, WebP or BMP image, read from its header only.
pub fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let size = imagesize::blob_size(bytes).ok()?;
    let w = u32::try_from(size.width).ok()?;
    let h = u32::try_from(size.height).ok()?;
    Some((w, h))
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

fn validate_rel(rel: &str) -> io::Result<PathBuf> {
    let bad = |msg: &str| io::Error::new(io::ErrorKind::InvalidInput, msg.to_string());
    if rel.is_empty() || rel.contains(':') || rel.contains('\\') {
        return Err(bad("unsafe destination path"));
    }
    // Path::components silently drops interior "." segments, so check the raw text too.
    if rel
        .split('/')
        .any(|seg| seg.is_empty() || seg == "." || seg == "..")
    {
        return Err(bad("destination escapes the vault"));
    }
    let mut out = PathBuf::new();
    for c in Path::new(rel).components() {
        match c {
            Component::Normal(s) => {
                if s.to_str().is_none() {
                    return Err(bad("destination is not valid UTF-8"));
                }
                out.push(s);
            }
            _ => return Err(bad("destination escapes the vault")),
        }
    }
    if out.as_os_str().is_empty() {
        return Err(bad("empty destination"));
    }
    Ok(out)
}

fn same_content(path: &Path, bytes: &[u8]) -> io::Result<bool> {
    Ok(fs::read(path)? == bytes)
}

/// Writes `plan.dest_rel` under `root` with `bytes`.
///
/// - Rejects destinations that are absolute, contain `..`, drive prefixes or `:`.
/// - Creates missing folders.
/// - Writes to a temporary file in the same folder and moves it into place.
/// - If the destination exists with identical content, succeeds without writing.
/// - If it exists with different content, returns `AlreadyExists` and leaves it alone.
pub fn write_import(root: &Path, plan: &ImportPlan, bytes: &[u8]) -> io::Result<()> {
    let rel = validate_rel(&plan.dest_rel)?;
    let dest = root.join(&rel);
    let parent = dest
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "destination has no parent"))?;
    fs::create_dir_all(&parent)?;

    match fs::metadata(&dest) {
        Ok(meta) => {
            if !meta.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "destination is not a file",
                ));
            }
            return if same_content(&dest, bytes)? {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "a different file already exists",
                ))
            };
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }

    let file_name = rel
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("import")
        .to_string();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = parent.join(format!(".{file_name}.{}.{nanos}.tmp", std::process::id()));
    {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }

    let placed = match fs::hard_link(&tmp, &dest) {
        Ok(()) => {
            let _ = fs::remove_file(&tmp);
            return Ok(());
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            let _ = fs::remove_file(&tmp);
            return if same_content(&dest, bytes)? {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "a different file already exists",
                ))
            };
        }
        // Hard links are not supported on every filesystem. Fall back to a check and a rename.
        Err(_) => {
            if dest.exists() {
                let _ = fs::remove_file(&tmp);
                return if same_content(&dest, bytes)? {
                    Ok(())
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "a different file already exists",
                    ))
                };
            }
            fs::rename(&tmp, &dest)
        }
    };
    if placed.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    placed
}

// ---------------------------------------------------------------------------
// File types
// ---------------------------------------------------------------------------

fn ext_of(path: &str) -> Option<String> {
    let base = path.rsplit(['/', '\\']).next().unwrap_or("");
    let dot = base.rfind('.')?;
    if dot == 0 {
        return None;
    }
    Some(base[dot + 1..].to_ascii_lowercase())
}

/// True for files that are attachments rather than notes: images, PDFs, audio, video and
/// common office or archive files. Markdown, text, JSON and canvas files are not attachments.
pub fn is_attachment(path: &str) -> bool {
    match ext_of(path) {
        Some(e) => matches!(
            e.as_str(),
            "png"
                | "jpg"
                | "jpeg"
                | "jpe"
                | "gif"
                | "webp"
                | "bmp"
                | "svg"
                | "avif"
                | "tif"
                | "tiff"
                | "heic"
                | "ico"
                | "pdf"
                | "mp3"
                | "wav"
                | "ogg"
                | "oga"
                | "opus"
                | "m4a"
                | "m4b"
                | "flac"
                | "aac"
                | "mp4"
                | "m4v"
                | "webm"
                | "mov"
                | "mkv"
                | "avi"
                | "ogv"
                | "docx"
                | "xlsx"
                | "pptx"
                | "zip"
                | "7z"
                | "rar"
                | "tar"
                | "gz"
        ),
        None => false,
    }
}

/// MIME type for a path, from its extension. `application/octet-stream` when unknown.
pub fn mime_for(path: &str) -> &'static str {
    match ext_of(path).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg" | "jpe") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        Some("svg") => "image/svg+xml",
        Some("avif") => "image/avif",
        Some("tif" | "tiff") => "image/tiff",
        Some("heic") => "image/heic",
        Some("ico") => "image/x-icon",
        Some("pdf") => "application/pdf",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("ogg" | "oga") => "audio/ogg",
        Some("opus") => "audio/opus",
        Some("m4a" | "m4b") => "audio/mp4",
        Some("flac") => "audio/flac",
        Some("aac") => "audio/aac",
        Some("mp4" | "m4v") => "video/mp4",
        Some("webm") => "video/webm",
        Some("ogv") => "video/ogg",
        Some("mov") => "video/quicktime",
        Some("mkv") => "video/x-matroska",
        Some("avi") => "video/x-msvideo",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("pptx") => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        Some("zip") => "application/zip",
        Some("7z") => "application/x-7z-compressed",
        Some("rar") => "application/vnd.rar",
        Some("tar") => "application/x-tar",
        Some("gz") => "application/gzip",
        Some("md" | "markdown") => "text/markdown",
        Some("txt") => "text/plain",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Timestamp = Timestamp {
        y: 2026,
        m: 11,
        d: 4,
        hh: 15,
        mm: 30,
        ss: 12,
    };

    fn png_bytes(w: u32, h: u32) -> Vec<u8> {
        let mut v = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        v.extend_from_slice(&13u32.to_be_bytes());
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&[8, 6, 0, 0, 0]);
        v.extend_from_slice(&[0, 0, 0, 0]);
        v
    }

    fn no_files(_: &str) -> Option<Vec<u8>> {
        None
    }

    fn fixed(folder: &str, style: LinkStyle) -> AttachSettings {
        AttachSettings {
            folder: AttachFolder::Fixed(folder.to_string()),
            link_style: style,
            default_pasted_prefix: "Pasted image".to_string(),
        }
    }

    #[test]
    fn defaults_match_spec() {
        let d = AttachSettings::default();
        assert_eq!(d.folder, AttachFolder::Fixed("附件".to_string()));
        assert_eq!(d.link_style, LinkStyle::WikiEmbed);
        assert_eq!(d.default_pasted_prefix, "Pasted image");
    }

    #[test]
    fn settings_serde_round_trip() {
        let s = AttachSettings {
            folder: AttachFolder::SubfolderOfNote("assets".into()),
            link_style: LinkStyle::Markdown,
            default_pasted_prefix: "截图".into(),
        };
        let json = serde_json::to_string(&s).unwrap_or_default();
        let back: AttachSettings =
            serde_json::from_str(&json).unwrap_or_else(|_| AttachSettings::default());
        assert_eq!(back, s);
    }

    #[test]
    fn settings_partial_json_uses_defaults() {
        let back: AttachSettings = serde_json::from_str("{}").unwrap_or_else(|_| AttachSettings {
            folder: AttachFolder::VaultRoot,
            link_style: LinkStyle::Markdown,
            default_pasted_prefix: String::new(),
        });
        assert_eq!(back, AttachSettings::default());
    }

    #[test]
    fn sanitize_table() {
        let cases: &[(&str, &str)] = &[
            ("plain", "plain"),
            ("a<b>c:d", "a_b_c_d"),
            ("q\"u?o*t|e", "q_u_o_t_e"),
            ("slash/back\\", "slash_back_"),
            ("  many   spaces  ", "many spaces"),
            ("tab\tnew\nline", "tab new line"),
            ("...dots...", "dots"),
            ("trailing. ", "trailing"),
            ("中文 名称", "中文 名称"),
            ("🙂 emoji", "🙂 emoji"),
            ("#tag[x]^y", "_tag_x__y"),
            ("CON", "_CON"),
            ("nul.txt", "_nul.txt"),
            ("com1", "_com1"),
            ("LPT9", "_LPT9"),
            ("COM10", "COM10"),
            ("", ""),
            ("...", ""),
        ];
        for (input, want) in cases {
            assert_eq!(sanitize_component(input), *want, "input {input:?}");
        }
    }

    #[test]
    fn sanitize_caps_length_on_char_boundary() {
        let long: String = "中".repeat(100);
        let s = sanitize_component(&long);
        assert!(s.len() <= MAX_STEM_BYTES);
        assert!(s.chars().all(|c| c == '中'));
        let emoji: String = "🙂".repeat(80);
        let e = sanitize_component(&emoji);
        assert!(e.len() <= MAX_STEM_BYTES);
        assert!(!e.is_empty());
    }

    #[test]
    fn sanitize_never_yields_traversal() {
        for input in ["..", "../..", "..\\..", "a/../b", "./x"] {
            for part in sanitize_rel_dir(input) {
                assert!(!part.contains('/') && !part.contains('\\'));
                assert_ne!(part, "..");
                assert_ne!(part, ".");
            }
        }
    }

    #[test]
    fn rel_dir_drops_traversal() {
        assert_eq!(sanitize_rel_dir("../../x"), vec!["x".to_string()]);
        assert_eq!(sanitize_rel_dir("a\\b/../c"), vec!["a", "b", "c"]);
        assert!(sanitize_rel_dir("").is_empty());
        assert!(sanitize_rel_dir("/").is_empty());
    }

    #[test]
    fn note_dir_extraction() {
        assert!(note_dir("note.md").is_empty());
        assert_eq!(note_dir("projects/a.md"), vec!["projects"]);
        assert_eq!(note_dir("日记\\2026\\x.md"), vec!["日记", "2026"]);
        assert_eq!(note_dir("../../evil/n.md"), vec!["evil"]);
    }

    #[test]
    fn split_name_cases() {
        assert_eq!(
            split_name("photo.PNG"),
            ("photo".into(), Some("png".into()))
        );
        assert_eq!(split_name("noext"), ("noext".into(), None));
        assert_eq!(split_name("a b.c d"), ("a b.c d".into(), None));
        assert_eq!(
            split_name("dir/sub/file.jpg"),
            ("file".into(), Some("jpg".into()))
        );
        assert_eq!(split_name(".hidden"), ("hidden".into(), None));
        assert_eq!(split_name("中文.png"), ("中文".into(), Some("png".into())));
    }

    #[test]
    fn sniff_table() {
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&[1, 0, 1, 0]);
        let mut webp = b"RIFF".to_vec();
        webp.extend_from_slice(&[0, 0, 0, 0]);
        webp.extend_from_slice(b"WEBPVP8 ");
        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&[0, 0, 0, 0]);
        wav.extend_from_slice(b"WAVEfmt ");
        let mut m4a = vec![0, 0, 0, 0x20];
        m4a.extend_from_slice(b"ftypM4A ");
        let mut mov = vec![0, 0, 0, 0x14];
        mov.extend_from_slice(b"ftypqt  ");
        let mut mp4 = vec![0, 0, 0, 0x18];
        mp4.extend_from_slice(b"ftypisom");
        let mut bmp = b"BM".to_vec();
        bmp.extend_from_slice(&20u32.to_le_bytes());
        bmp.extend_from_slice(&[0; 14]);
        let cases: Vec<(Vec<u8>, Option<&str>)> = vec![
            (png_bytes(1, 1), Some("png")),
            (vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10], Some("jpg")),
            (gif, Some("gif")),
            (webp, Some("webp")),
            (wav, Some("wav")),
            (b"%PDF-1.7\n".to_vec(), Some("pdf")),
            (b"OggS\0\x02".to_vec(), Some("ogg")),
            (b"ID3\x03\0".to_vec(), Some("mp3")),
            (vec![0xFF, 0xFB, 0x90, 0x00], Some("mp3")),
            (vec![0x1A, 0x45, 0xDF, 0xA3, 0x01], Some("webm")),
            (m4a, Some("m4a")),
            (mov, Some("mov")),
            (mp4, Some("mp4")),
            (bmp, Some("bmp")),
            (
                b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>".to_vec(),
                Some("svg"),
            ),
            (b"<?xml version=\"1.0\"?><svg></svg>".to_vec(), Some("svg")),
            (b"<html><body></body></html>".to_vec(), None),
            (b"plain text".to_vec(), None),
            (vec![], None),
            (vec![0x89, b'P'], None),
            (b"RIFF".to_vec(), None),
            (vec![0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06], None),
        ];
        for (bytes, want) in cases {
            assert_eq!(
                sniff_ext(&bytes),
                want,
                "bytes {:?}",
                &bytes[..bytes.len().min(12)]
            );
        }
    }

    #[test]
    fn bmp_with_wrong_size_is_not_bmp() {
        let mut bmp = b"BM".to_vec();
        bmp.extend_from_slice(&999u32.to_le_bytes());
        bmp.extend_from_slice(&[0; 14]);
        assert_eq!(sniff_ext(&bmp), None);
    }

    #[test]
    fn clipboard_image_gets_timestamp_name() {
        let plan = plan_import(
            &AttachSettings::default(),
            "daily/2026-11-04.md",
            None,
            &png_bytes(10, 10),
            NOW,
            &no_files,
        );
        assert_eq!(plan.dest_rel, "附件/Pasted image 20261104-153012.png");
        assert_eq!(plan.link_text, "![[Pasted image 20261104-153012.png]]");
        assert!(!plan.reuse_existing);
    }

    #[test]
    fn wrong_extension_is_corrected_from_magic() {
        let plan = plan_import(
            &fixed("img", LinkStyle::WikiEmbed),
            "n.md",
            Some("shot.jpg"),
            &png_bytes(2, 2),
            NOW,
            &no_files,
        );
        assert_eq!(plan.dest_rel, "img/shot.png");
    }

    #[test]
    fn matching_jpeg_alias_keeps_original_extension() {
        let jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0];
        let plan = plan_import(
            &fixed("img", LinkStyle::WikiEmbed),
            "n.md",
            Some("pic.jpeg"),
            &jpeg,
            NOW,
            &no_files,
        );
        assert_eq!(plan.dest_rel, "img/pic.jpeg");
    }

    #[test]
    fn unknown_bytes_without_extension_become_bin() {
        let plan = plan_import(
            &fixed("f", LinkStyle::WikiEmbed),
            "n.md",
            None,
            &[1, 2, 3, 4],
            NOW,
            &no_files,
        );
        assert!(plan.dest_rel.ends_with(".bin"), "{}", plan.dest_rel);
    }

    #[test]
    fn unknown_bytes_keep_original_extension() {
        let plan = plan_import(
            &fixed("f", LinkStyle::WikiEmbed),
            "n.md",
            Some("data.csv"),
            b"a,b",
            NOW,
            &no_files,
        );
        assert_eq!(plan.dest_rel, "f/data.csv");
    }

    #[test]
    fn name_with_chinese_and_spaces_is_normalised() {
        let plan = plan_import(
            &fixed("附件", LinkStyle::WikiEmbed),
            "n.md",
            Some("  我的   截图 (1).png "),
            &png_bytes(1, 1),
            NOW,
            &no_files,
        );
        assert_eq!(plan.dest_rel, "附件/我的 截图 (1).png");
    }

    #[test]
    fn emoji_name_kept() {
        let plan = plan_import(
            &fixed("a", LinkStyle::WikiEmbed),
            "n.md",
            Some("🙂 face.png"),
            &png_bytes(1, 1),
            NOW,
            &no_files,
        );
        assert_eq!(plan.dest_rel, "a/🙂 face.png");
    }

    #[test]
    fn windows_reserved_name_is_prefixed() {
        let plan = plan_import(
            &fixed("a", LinkStyle::WikiEmbed),
            "n.md",
            Some("CON.png"),
            &png_bytes(1, 1),
            NOW,
            &no_files,
        );
        assert_eq!(plan.dest_rel, "a/_CON.png");
    }

    #[test]
    fn empty_name_after_sanitize_falls_back_to_timestamp() {
        let plan = plan_import(
            &fixed("a", LinkStyle::WikiEmbed),
            "n.md",
            Some("...png"),
            &png_bytes(1, 1),
            NOW,
            &no_files,
        );
        assert!(
            plan.dest_rel.starts_with("a/Pasted image 20261104-153012"),
            "{}",
            plan.dest_rel
        );
    }

    #[test]
    fn malicious_name_stays_inside_folder() {
        let plan = plan_import(
            &fixed("a", LinkStyle::WikiEmbed),
            "n.md",
            Some("../../../etc/passwd.png"),
            &png_bytes(1, 1),
            NOW,
            &no_files,
        );
        assert_eq!(plan.dest_rel, "a/passwd.png");
    }

    #[test]
    fn malicious_fixed_folder_is_sanitized() {
        let s = fixed("../../x", LinkStyle::WikiEmbed);
        let plan = plan_import(&s, "n.md", Some("a.png"), &png_bytes(1, 1), NOW, &no_files);
        assert_eq!(plan.dest_rel, "x/a.png");
    }

    #[test]
    fn absolute_style_folder_is_relative() {
        let s = fixed("C:\\Windows\\evil", LinkStyle::WikiEmbed);
        let plan = plan_import(&s, "n.md", Some("a.png"), &png_bytes(1, 1), NOW, &no_files);
        assert!(!plan.dest_rel.contains(':'));
        assert!(!plan.dest_rel.starts_with('/'));
        assert!(plan.dest_rel.ends_with("/a.png"));
    }

    #[test]
    fn vault_root_folder() {
        let s = AttachSettings {
            folder: AttachFolder::VaultRoot,
            ..AttachSettings::default()
        };
        let plan = plan_import(
            &s,
            "deep/n.md",
            Some("a.png"),
            &png_bytes(1, 1),
            NOW,
            &no_files,
        );
        assert_eq!(plan.dest_rel, "a.png");
        assert_eq!(plan.link_text, "![[a.png]]");
    }

    #[test]
    fn same_as_note_folder() {
        let s = AttachSettings {
            folder: AttachFolder::SameAsNote,
            ..AttachSettings::default()
        };
        let plan = plan_import(
            &s,
            "proj/x/n.md",
            Some("a.png"),
            &png_bytes(1, 1),
            NOW,
            &no_files,
        );
        assert_eq!(plan.dest_rel, "proj/x/a.png");
    }

    #[test]
    fn subfolder_of_note_folder() {
        let s = AttachSettings {
            folder: AttachFolder::SubfolderOfNote("assets".into()),
            ..AttachSettings::default()
        };
        let plan = plan_import(
            &s,
            "proj/n.md",
            Some("a.png"),
            &png_bytes(1, 1),
            NOW,
            &no_files,
        );
        assert_eq!(plan.dest_rel, "proj/assets/a.png");
        let root_note = plan_import(&s, "n.md", Some("a.png"), &png_bytes(1, 1), NOW, &no_files);
        assert_eq!(root_note.dest_rel, "assets/a.png");
    }

    #[test]
    fn collision_appends_numbers() {
        let taken: Vec<&str> = vec!["附件/a.png", "附件/a 1.png"];
        let exists = |p: &str| -> Option<Vec<u8>> {
            if taken.contains(&p) {
                Some(b"different".to_vec())
            } else {
                None
            }
        };
        let plan = plan_import(
            &fixed("附件", LinkStyle::WikiEmbed),
            "n.md",
            Some("a.png"),
            &png_bytes(1, 1),
            NOW,
            &exists,
        );
        assert_eq!(plan.dest_rel, "附件/a 2.png");
        assert!(!plan.reuse_existing);
    }

    #[test]
    fn identical_content_is_reused() {
        let bytes = png_bytes(3, 3);
        let stored = bytes.clone();
        let exists = move |p: &str| -> Option<Vec<u8>> {
            if p == "附件/a.png" {
                Some(stored.clone())
            } else {
                None
            }
        };
        let plan = plan_import(
            &fixed("附件", LinkStyle::WikiEmbed),
            "n.md",
            Some("a.png"),
            &bytes,
            NOW,
            &exists,
        );
        assert_eq!(plan.dest_rel, "附件/a.png");
        assert!(plan.reuse_existing);
    }

    #[test]
    fn identical_content_under_suffix_is_reused() {
        let bytes = png_bytes(4, 4);
        let stored = bytes.clone();
        let exists = move |p: &str| -> Option<Vec<u8>> {
            match p {
                "f/a.png" => Some(b"other".to_vec()),
                "f/a 1.png" => Some(stored.clone()),
                _ => None,
            }
        };
        let plan = plan_import(
            &fixed("f", LinkStyle::WikiEmbed),
            "n.md",
            Some("a.png"),
            &bytes,
            NOW,
            &exists,
        );
        assert_eq!(plan.dest_rel, "f/a 1.png");
        assert!(plan.reuse_existing);
    }

    #[test]
    fn stuck_collisions_fall_back_to_hash_suffix() {
        let exists = |_: &str| -> Option<Vec<u8>> { Some(b"always different".to_vec()) };
        let bytes = png_bytes(5, 5);
        let plan = plan_import(
            &fixed("f", LinkStyle::WikiEmbed),
            "n.md",
            Some("a.png"),
            &bytes,
            NOW,
            &exists,
        );
        assert!(plan.dest_rel.starts_with("f/a "), "{}", plan.dest_rel);
        assert!(plan.dest_rel.ends_with(".png"));
        assert!(!plan.reuse_existing);
    }

    #[test]
    fn wiki_link_uses_name_only_by_default() {
        let plan = plan_import(
            &fixed("sub", LinkStyle::WikiEmbed),
            "n.md",
            Some("a.pdf"),
            b"%PDF-1",
            NOW,
            &no_files,
        );
        assert_eq!(plan.link_text, "![[a.pdf]]");
    }

    #[test]
    fn wiki_link_qualifies_ambiguous_name() {
        let plan = plan_import_checked(
            &fixed("sub", LinkStyle::WikiEmbed),
            "n.md",
            Some("a.pdf"),
            b"%PDF-1",
            NOW,
            &no_files,
            &|name| name == "a.pdf",
        );
        assert_eq!(plan.link_text, "![[sub/a.pdf]]");
    }

    #[test]
    fn markdown_link_relative_from_note_folder() {
        let plan = plan_import(
            &fixed("附件", LinkStyle::Markdown),
            "日记/2026.md",
            Some("截图.png"),
            &png_bytes(1, 1),
            NOW,
            &no_files,
        );
        assert_eq!(
            plan.link_text,
            format!(
                "![截图](../{}/{})",
                encode_segment("附件"),
                encode_segment("截图.png")
            )
        );
    }

    #[test]
    fn markdown_link_same_folder_no_up() {
        let plan = plan_import(
            &fixed("", LinkStyle::Markdown),
            "n.md",
            Some("a.png"),
            &png_bytes(1, 1),
            NOW,
            &no_files,
        );
        assert_eq!(plan.link_text, "![a](a.png)");
    }

    #[test]
    fn markdown_link_spaces_encoded_and_parseable() {
        let plan = plan_import(
            &fixed("my files", LinkStyle::Markdown),
            "n.md",
            Some("a b.pdf"),
            b"%PDF-1",
            NOW,
            &no_files,
        );
        assert_eq!(plan.link_text, "[a b.pdf](my%20files/a%20b.pdf)");
        let links = crate::embeds::parse_links(&plan.link_text);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "my files/a b.pdf");
    }

    #[test]
    fn markdown_link_round_trips_through_parser() {
        let plan = plan_import(
            &fixed("附件", LinkStyle::Markdown),
            "d/x.md",
            Some("图 (1).png"),
            &png_bytes(1, 1),
            NOW,
            &no_files,
        );
        let links = crate::embeds::parse_links(&plan.link_text);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "../附件/图 (1).png");
    }

    #[test]
    fn markdown_link_deeper_note() {
        let plan = plan_import(
            &fixed("assets", LinkStyle::Markdown),
            "a/b/c/n.md",
            Some("x.png"),
            &png_bytes(1, 1),
            NOW,
            &no_files,
        );
        assert_eq!(plan.link_text, "![x](../../../assets/x.png)");
    }

    #[test]
    fn pdf_markdown_link_is_not_image() {
        let plan = plan_import(
            &fixed("f", LinkStyle::Markdown),
            "n.md",
            Some("doc.pdf"),
            b"%PDF-1",
            NOW,
            &no_files,
        );
        assert_eq!(plan.link_text, "[doc.pdf](f/doc.pdf)");
    }

    #[test]
    fn relative_link_table() {
        let from = vec!["a".to_string(), "b".to_string()];
        assert_eq!(relative_link(&from, &from, "x.png"), "x.png");
        assert_eq!(
            relative_link(&from, &["a".to_string()], "x.png"),
            "../x.png"
        );
        assert_eq!(relative_link(&[], &["c".to_string()], "x.png"), "c/x.png");
        assert_eq!(
            relative_link(&from, &["q".to_string()], "x.png"),
            "../../q/x.png"
        );
    }

    #[test]
    fn encode_segment_table() {
        assert_eq!(encode_segment("abc-_.~"), "abc-_.~");
        assert_eq!(encode_segment("a b"), "a%20b");
        assert_eq!(encode_segment("(x)"), "%28x%29");
        assert_eq!(encode_segment("中"), "%E4%B8%AD");
    }

    #[test]
    fn plan_is_deterministic() {
        let a = plan_import(
            &AttachSettings::default(),
            "n.md",
            None,
            &png_bytes(7, 7),
            NOW,
            &no_files,
        );
        let b = plan_import(
            &AttachSettings::default(),
            "n.md",
            None,
            &png_bytes(7, 7),
            NOW,
            &no_files,
        );
        assert_eq!(a, b);
    }

    #[test]
    fn stamp_format() {
        assert_eq!(stamp(NOW), "20261104-153012");
        let early = Timestamp {
            y: 999,
            m: 1,
            d: 2,
            hh: 3,
            mm: 4,
            ss: 5,
        };
        assert_eq!(stamp(early), "09990102-030405");
    }

    #[test]
    fn image_dimensions_png_and_gif() {
        assert_eq!(image_dimensions(&png_bytes(640, 480)), Some((640, 480)));
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&300u16.to_le_bytes());
        gif.extend_from_slice(&200u16.to_le_bytes());
        gif.extend_from_slice(&[0x00, 0x00, 0x00]);
        assert_eq!(image_dimensions(&gif), Some((300, 200)));
    }

    #[test]
    fn image_dimensions_rejects_garbage() {
        assert_eq!(image_dimensions(b"not an image at all"), None);
        assert_eq!(image_dimensions(&[]), None);
        assert_eq!(image_dimensions(&[0x89, b'P', b'N', b'G']), None);
    }

    #[test]
    fn image_dimensions_truncated_png_is_none() {
        let full = png_bytes(10, 10);
        assert_eq!(image_dimensions(&full[..10]), None);
    }

    #[test]
    fn image_dimensions_svg_not_supported() {
        assert_eq!(
            image_dimensions(b"<svg width=\"10\" height=\"10\"></svg>"),
            None
        );
    }

    #[test]
    fn validate_rel_table() {
        assert!(validate_rel("a/b.png").is_ok());
        assert!(validate_rel("附件/图.png").is_ok());
        for bad in [
            "", "../x", "a/../b", "/abs/x", "C:/x", "a\\b", "a:b", "./x", "a/./b",
        ] {
            assert!(validate_rel(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn is_attachment_table() {
        let yes = [
            "a.png",
            "a.JPG",
            "doc.pdf",
            "clip.MP4",
            "song.flac",
            "x.docx",
            "z.zip",
            "pic.jpeg",
        ];
        let no = [
            "note.md",
            "notes.txt",
            "data.json",
            "canvas.canvas",
            "noext",
            ".hidden",
            "a.unknownext",
        ];
        for p in yes {
            assert!(is_attachment(p), "{p}");
        }
        for p in no {
            assert!(!is_attachment(p), "{p}");
        }
    }

    #[test]
    fn mime_table() {
        let cases: &[(&str, &str)] = &[
            ("a.png", "image/png"),
            ("a.JPG", "image/jpeg"),
            ("a.svg", "image/svg+xml"),
            ("a.pdf", "application/pdf"),
            ("a.mp3", "audio/mpeg"),
            ("a.webm", "video/webm"),
            ("a.mov", "video/quicktime"),
            ("a.md", "text/markdown"),
            ("a.unknown", "application/octet-stream"),
            ("noext", "application/octet-stream"),
        ];
        for (p, want) in cases {
            assert_eq!(mime_for(p), *want, "{p}");
        }
    }

    #[test]
    fn write_import_creates_folders_and_content() {
        let dir = test_dir("create");
        let plan = ImportPlan {
            dest_rel: "附件/深 层/x.png".into(),
            link_text: String::new(),
            reuse_existing: false,
        };
        let bytes = png_bytes(2, 2);
        assert!(write_import(&dir, &plan, &bytes).is_ok());
        assert_eq!(
            fs::read(dir.join("附件").join("深 层").join("x.png")).ok(),
            Some(bytes)
        );
        cleanup(&dir);
    }

    #[test]
    fn write_import_identical_is_ok_different_is_refused() {
        let dir = test_dir("conflict");
        let plan = ImportPlan {
            dest_rel: "a.png".into(),
            link_text: String::new(),
            reuse_existing: false,
        };
        assert!(write_import(&dir, &plan, b"first").is_ok());
        assert!(write_import(&dir, &plan, b"first").is_ok());
        let err = write_import(&dir, &plan, b"second");
        assert_eq!(
            err.as_ref().map_err(io::Error::kind).err(),
            Some(io::ErrorKind::AlreadyExists)
        );
        assert_eq!(fs::read(dir.join("a.png")).ok(), Some(b"first".to_vec()));
        cleanup(&dir);
    }

    #[test]
    fn write_import_rejects_escape() {
        let dir = test_dir("escape");
        for bad in ["../outside.png", "/abs.png", "a/../../b.png", "C:/x.png"] {
            let plan = ImportPlan {
                dest_rel: bad.into(),
                link_text: String::new(),
                reuse_existing: false,
            };
            let err = write_import(&dir, &plan, b"x");
            assert_eq!(
                err.as_ref().map_err(io::Error::kind).err(),
                Some(io::ErrorKind::InvalidInput),
                "{bad}"
            );
        }
        assert!(
            !dir.parent()
                .map(|p| p.join("outside.png"))
                .is_some_and(|p| p.exists())
        );
        cleanup(&dir);
    }

    #[test]
    fn write_import_leaves_no_temp_files() {
        let dir = test_dir("tmp");
        let plan = ImportPlan {
            dest_rel: "n/x.bin".into(),
            link_text: String::new(),
            reuse_existing: false,
        };
        assert!(write_import(&dir, &plan, b"data").is_ok());
        let leftovers: Vec<_> = fs::read_dir(dir.join("n"))
            .map(|rd| rd.filter_map(Result::ok).map(|e| e.file_name()).collect())
            .unwrap_or_default();
        assert_eq!(leftovers.len(), 1, "{leftovers:?}");
        cleanup(&dir);
    }

    #[test]
    fn write_import_reuse_flag_with_missing_file_still_writes() {
        let dir = test_dir("reuse-missing");
        let plan = ImportPlan {
            dest_rel: "r.png".into(),
            link_text: String::new(),
            reuse_existing: true,
        };
        assert!(write_import(&dir, &plan, b"abc").is_ok());
        assert_eq!(fs::read(dir.join("r.png")).ok(), Some(b"abc".to_vec()));
        cleanup(&dir);
    }

    #[test]
    fn write_import_destination_is_directory_is_refused() {
        let dir = test_dir("dirdest");
        assert!(fs::create_dir_all(dir.join("folder.png")).is_ok());
        let plan = ImportPlan {
            dest_rel: "folder.png".into(),
            link_text: String::new(),
            reuse_existing: false,
        };
        let err = write_import(&dir, &plan, b"x");
        assert_eq!(
            err.as_ref().map_err(io::Error::kind).err(),
            Some(io::ErrorKind::AlreadyExists)
        );
        cleanup(&dir);
    }

    #[test]
    fn plan_then_write_end_to_end_with_real_directory() {
        let dir = test_dir("e2e");
        let settings = AttachSettings::default();
        let bytes = png_bytes(8, 6);
        let root = dir.clone();
        let exists = move |p: &str| fs::read(root.join(p)).ok();
        let first = plan_import(&settings, "日记/今天.md", None, &bytes, NOW, &exists);
        assert!(write_import(&dir, &first, &bytes).is_ok());
        let second = plan_import(&settings, "日记/今天.md", None, &bytes, NOW, &exists);
        assert!(second.reuse_existing);
        assert_eq!(second.dest_rel, first.dest_rel);
        assert_eq!(image_dimensions(&bytes), Some((8, 6)));
        cleanup(&dir);
    }

    #[test]
    fn plan_import_with_huge_name_is_bounded() {
        let name = format!("{}.png", "长".repeat(1000));
        let plan = plan_import(
            &fixed("f", LinkStyle::WikiEmbed),
            "n.md",
            Some(&name),
            &png_bytes(1, 1),
            NOW,
            &no_files,
        );
        let file = plan.dest_rel.trim_start_matches("f/");
        assert!(file.len() <= MAX_STEM_BYTES + 5, "{}", file.len());
    }

    #[test]
    fn robustness_random_names_and_bytes() {
        let alphabet: Vec<&str> = vec![
            "a", "Z", "9", " ", "  ", "中", "🙂", ".", "..", "/", "\\", ":", "*", "?", "\"", "<",
            ">", "|", "#", "[", "]", "^", "\t", "\n", "CON", "COM1", "png", "jpg", "\u{0}", "é",
            "%",
        ];
        let mut state: u64 = 0x0bad_c0de_1234_5678;
        let mut next = move || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as usize
        };
        let folders = [
            AttachFolder::VaultRoot,
            AttachFolder::Fixed("../x/附件".into()),
            AttachFolder::SameAsNote,
            AttachFolder::SubfolderOfNote("../../s".into()),
        ];
        for _ in 0..2000 {
            let mut name = String::new();
            for _ in 0..(next() % 12) {
                name.push_str(alphabet[next() % alphabet.len()]);
            }
            let mut bytes = vec![0u8; next() % 16];
            for b in &mut bytes {
                *b = (next() % 256) as u8;
            }
            let settings = AttachSettings {
                folder: folders[next() % folders.len()].clone(),
                link_style: if next() % 2 == 0 {
                    LinkStyle::WikiEmbed
                } else {
                    LinkStyle::Markdown
                },
                default_pasted_prefix: name.clone(),
            };
            let note = format!("{}/n.md", alphabet[next() % alphabet.len()]);
            let plan = plan_import(&settings, &note, Some(&name), &bytes, NOW, &no_files);
            assert!(!plan.dest_rel.is_empty());
            assert!(
                validate_rel(&plan.dest_rel).is_ok(),
                "bad dest {:?} from {name:?}",
                plan.dest_rel
            );
            assert!(!plan.link_text.is_empty());
            let _ = sniff_ext(&bytes);
            let _ = image_dimensions(&bytes);
            let _ = is_attachment(&name);
            let _ = mime_for(&name);
        }
    }

    fn test_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("jiyue-md-{tag}-{}-{nanos}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }
}
