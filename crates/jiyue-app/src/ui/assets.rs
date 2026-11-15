//! Embedded asset source: Tabler outline icons compiled into the binary.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

/// Every icon the UI draws. Paths are the names GPUI's `svg().path(..)` asks for.
const ICONS: &[(&str, &[u8])] = &[
    (
        "icons/affiliate.svg",
        include_bytes!("../../assets/icons/affiliate.svg"),
    ),
    (
        "icons/arrow-down.svg",
        include_bytes!("../../assets/icons/arrow-down.svg"),
    ),
    (
        "icons/arrow-up.svg",
        include_bytes!("../../assets/icons/arrow-up.svg"),
    ),
    (
        "icons/arrows-sort.svg",
        include_bytes!("../../assets/icons/arrows-sort.svg"),
    ),
    (
        "icons/bolt.svg",
        include_bytes!("../../assets/icons/bolt.svg"),
    ),
    (
        "icons/book-2.svg",
        include_bytes!("../../assets/icons/book-2.svg"),
    ),
    (
        "icons/bookmark.svg",
        include_bytes!("../../assets/icons/bookmark.svg"),
    ),
    (
        "icons/bookmark-filled.svg",
        include_bytes!("../../assets/icons/bookmark-filled.svg"),
    ),
    (
        "icons/alert-triangle.svg",
        include_bytes!("../../assets/icons/alert-triangle.svg"),
    ),
    (
        "icons/bug.svg",
        include_bytes!("../../assets/icons/bug.svg"),
    ),
    (
        "icons/bulb.svg",
        include_bytes!("../../assets/icons/bulb.svg"),
    ),
    (
        "icons/checkbox.svg",
        include_bytes!("../../assets/icons/checkbox.svg"),
    ),
    (
        "icons/hierarchy-2.svg",
        include_bytes!("../../assets/icons/hierarchy-2.svg"),
    ),
    (
        "icons/help-circle.svg",
        include_bytes!("../../assets/icons/help-circle.svg"),
    ),
    (
        "icons/list-numbers.svg",
        include_bytes!("../../assets/icons/list-numbers.svg"),
    ),
    (
        "icons/list.svg",
        include_bytes!("../../assets/icons/list.svg"),
    ),
    (
        "icons/math.svg",
        include_bytes!("../../assets/icons/math.svg"),
    ),
    (
        "icons/photo.svg",
        include_bytes!("../../assets/icons/photo.svg"),
    ),
    (
        "icons/quote.svg",
        include_bytes!("../../assets/icons/quote.svg"),
    ),
    (
        "icons/table.svg",
        include_bytes!("../../assets/icons/table.svg"),
    ),
    (
        "icons/calendar-event.svg",
        include_bytes!("../../assets/icons/calendar-event.svg"),
    ),
    (
        "icons/calendar.svg",
        include_bytes!("../../assets/icons/calendar.svg"),
    ),
    (
        "icons/camera-rotate.svg",
        include_bytes!("../../assets/icons/camera-rotate.svg"),
    ),
    (
        "icons/check.svg",
        include_bytes!("../../assets/icons/check.svg"),
    ),
    (
        "icons/chevron-down.svg",
        include_bytes!("../../assets/icons/chevron-down.svg"),
    ),
    (
        "icons/chevron-left.svg",
        include_bytes!("../../assets/icons/chevron-left.svg"),
    ),
    (
        "icons/chevron-right.svg",
        include_bytes!("../../assets/icons/chevron-right.svg"),
    ),
    (
        "icons/clock.svg",
        include_bytes!("../../assets/icons/clock.svg"),
    ),
    (
        "icons/code.svg",
        include_bytes!("../../assets/icons/code.svg"),
    ),
    (
        "icons/copy.svg",
        include_bytes!("../../assets/icons/copy.svg"),
    ),
    (
        "icons/corner-down-left.svg",
        include_bytes!("../../assets/icons/corner-down-left.svg"),
    ),
    (
        "icons/dots.svg",
        include_bytes!("../../assets/icons/dots.svg"),
    ),
    (
        "icons/download.svg",
        include_bytes!("../../assets/icons/download.svg"),
    ),
    (
        "icons/eye.svg",
        include_bytes!("../../assets/icons/eye.svg"),
    ),
    (
        "icons/file-plus.svg",
        include_bytes!("../../assets/icons/file-plus.svg"),
    ),
    (
        "icons/file-text.svg",
        include_bytes!("../../assets/icons/file-text.svg"),
    ),
    (
        "icons/focus-2.svg",
        include_bytes!("../../assets/icons/focus-2.svg"),
    ),
    (
        "icons/folder-open.svg",
        include_bytes!("../../assets/icons/folder-open.svg"),
    ),
    (
        "icons/folder-plus.svg",
        include_bytes!("../../assets/icons/folder-plus.svg"),
    ),
    (
        "icons/folder.svg",
        include_bytes!("../../assets/icons/folder.svg"),
    ),
    (
        "icons/hash.svg",
        include_bytes!("../../assets/icons/hash.svg"),
    ),
    (
        "icons/info-circle.svg",
        include_bytes!("../../assets/icons/info-circle.svg"),
    ),
    (
        "icons/keyboard.svg",
        include_bytes!("../../assets/icons/keyboard.svg"),
    ),
    (
        "icons/layout-columns.svg",
        include_bytes!("../../assets/icons/layout-columns.svg"),
    ),
    (
        "icons/layout-sidebar.svg",
        include_bytes!("../../assets/icons/layout-sidebar.svg"),
    ),
    (
        "icons/layout-sidebar-right.svg",
        include_bytes!("../../assets/icons/layout-sidebar-right.svg"),
    ),
    (
        "icons/link.svg",
        include_bytes!("../../assets/icons/link.svg"),
    ),
    (
        "icons/minus.svg",
        include_bytes!("../../assets/icons/minus.svg"),
    ),
    (
        "icons/palette.svg",
        include_bytes!("../../assets/icons/palette.svg"),
    ),
    (
        "icons/pencil.svg",
        include_bytes!("../../assets/icons/pencil.svg"),
    ),
    (
        "icons/plus.svg",
        include_bytes!("../../assets/icons/plus.svg"),
    ),
    (
        "icons/qrcode.svg",
        include_bytes!("../../assets/icons/qrcode.svg"),
    ),
    (
        "icons/refresh.svg",
        include_bytes!("../../assets/icons/refresh.svg"),
    ),
    (
        "icons/search.svg",
        include_bytes!("../../assets/icons/search.svg"),
    ),
    (
        "icons/selector.svg",
        include_bytes!("../../assets/icons/selector.svg"),
    ),
    (
        "icons/settings.svg",
        include_bytes!("../../assets/icons/settings.svg"),
    ),
    (
        "icons/sparkles.svg",
        include_bytes!("../../assets/icons/sparkles.svg"),
    ),
    (
        "icons/square-rounded-plus.svg",
        include_bytes!("../../assets/icons/square-rounded-plus.svg"),
    ),
    (
        "icons/square.svg",
        include_bytes!("../../assets/icons/square.svg"),
    ),
    (
        "icons/tag.svg",
        include_bytes!("../../assets/icons/tag.svg"),
    ),
    (
        "icons/text-size.svg",
        include_bytes!("../../assets/icons/text-size.svg"),
    ),
    (
        "icons/trash.svg",
        include_bytes!("../../assets/icons/trash.svg"),
    ),
    ("icons/x.svg", include_bytes!("../../assets/icons/x.svg")),
    // The app mark (rounded corners in the alpha channel): sidebar and About page.
    (
        "logo/icon-64.png",
        include_bytes!("../../assets/logo/icon-64.png"),
    ),
];

/// Serves the embedded icons to GPUI.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| SharedString::from(*name))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

    fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    collect_rs_files(&p, out);
                } else if p.extension().and_then(|s| s.to_str()) == Some("rs") {
                    out.push(p);
                }
            }
        }
    }

    fn extract_nth_arg(src: &str, open_paren_idx: usize, arg_idx: usize) -> Option<&str> {
        let bytes = src.as_bytes();
        let mut depth = 0usize;
        let mut in_str = false;
        let mut escape = false;
        let mut cur_arg = 0usize;
        let mut cur_start = open_paren_idx + 1;
        for (offset, &b) in bytes[open_paren_idx + 1..].iter().enumerate() {
            let abs = open_paren_idx + 1 + offset;
            if in_str {
                if escape {
                    escape = false;
                } else if b == b'\\' {
                    escape = true;
                } else if b == b'"' {
                    in_str = false;
                }
                continue;
            }
            match b {
                b'"' => in_str = true,
                b'(' | b'{' | b'[' => depth += 1,
                b')' | b'}' | b']' => {
                    if depth == 0 {
                        if cur_arg == arg_idx {
                            return Some(&src[cur_start..abs]);
                        }
                        return None;
                    }
                    depth -= 1;
                }
                b',' if depth == 0 => {
                    if cur_arg == arg_idx {
                        return Some(&src[cur_start..abs]);
                    }
                    cur_arg += 1;
                    cur_start = abs + 1;
                }
                _ => {}
            }
        }
        None
    }

    fn extract_string_literals(expr: &str) -> Vec<&str> {
        let mut out = Vec::new();
        let bytes = expr.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'"' {
                let start = i + 1;
                i = start;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == b'"' {
                        out.push(&expr[start..i]);
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            } else {
                i += 1;
            }
        }
        out
    }

    #[test]
    fn registered_svg_icons_are_non_empty() {
        let mut has_layout_sidebar = false;
        for (path, bytes) in ICONS {
            assert!(!bytes.is_empty(), "asset {path} must not be empty");
            if *path == "icons/layout-sidebar.svg" {
                has_layout_sidebar = true;
            }
            if path.ends_with(".svg") {
                let text = std::str::from_utf8(bytes).expect("svg must be utf8");
                assert!(text.contains("<svg"), "{path} missing <svg root");
            }
        }
        assert!(
            has_layout_sidebar,
            "icons/layout-sidebar.svg must be registered"
        );
    }

    #[test]
    fn no_missing_svg_icon_references_in_ui() {
        let registered: HashSet<&str> = ICONS
            .iter()
            .filter_map(|(k, _)| k.strip_prefix("icons/")?.strip_suffix(".svg"))
            .collect();

        let ui_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/ui");
        let mut files = Vec::new();
        collect_rs_files(&ui_dir, &mut files);

        let call_slots: [(&str, usize); 9] = [
            ("icon(", 0),
            ("icon_button(", 1),
            ("icon_button_sized(", 1),
            ("icon_toggle_button(", 1),
            ("small_icon_btn(", 1),
            ("tip_card(", 1),
            ("empty_state(", 1),
            ("button(", 3),
            ("button_sized(", 4),
        ];

        let mut missing = Vec::new();
        for file in files {
            if file.file_name().and_then(|s| s.to_str()) == Some("assets.rs") {
                continue;
            }
            let content = std::fs::read_to_string(&file).expect("read ui rs");
            // Exclude fn definitions like `fn icon(`
            for (needle, arg_idx) in call_slots {
                let mut pos = 0;
                while let Some(idx) = content[pos..].find(needle) {
                    let abs_idx = pos + idx;
                    let prev = abs_idx
                        .checked_sub(1)
                        .and_then(|i| content.as_bytes().get(i).copied());
                    let is_boundary = matches!(
                        prev,
                        None | Some(b' ' | b'\n' | b'\r' | b'\t' | b'(' | b'.' | b':' | b'!')
                    );
                    let before_line = content[..abs_idx].rsplit('\n').next().unwrap_or("");
                    let is_fn_def = before_line.trim_start().starts_with("pub fn ")
                        || before_line.trim_start().starts_with("fn ");
                    if is_boundary && !is_fn_def {
                        let paren_idx = abs_idx + needle.len() - 1;
                        if let Some(arg) = extract_nth_arg(&content, paren_idx, arg_idx) {
                            for name in extract_string_literals(arg) {
                                if !registered.contains(name) {
                                    missing.push(format!(
                                        "{}: `{name}` in `{needle}`",
                                        file.display()
                                    ));
                                }
                            }
                        }
                    }
                    pos = abs_idx + needle.len();
                }
            }
        }

        assert!(
            missing.is_empty(),
            "Found unregistered SVG icon references:\n{}",
            missing.join("\n")
        );
    }
}
