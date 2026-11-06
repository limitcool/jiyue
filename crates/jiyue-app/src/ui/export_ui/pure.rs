//! Pure helpers of the export: themes, base64 for inlined images, file names and the Markdown copy.
//! No I/O.

/// Look of an exported page. Both are the stylesheet of `jiyue-export`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Theme {
    /// Light page (纸张).
    #[default]
    Paper,
    /// Dark page (墨色).
    Ink,
}

impl Theme {
    /// The choices in the dialog, in order.
    pub const ALL: [Theme; 2] = [Theme::Paper, Theme::Ink];

    pub fn label(self) -> &'static str {
        match self {
            Theme::Paper => "纸张",
            Theme::Ink => "墨色",
        }
    }

    /// Value of `data-theme` on the page's root element.
    pub fn attr(self) -> &'static str {
        match self {
            Theme::Paper => "light",
            Theme::Ink => "dark",
        }
    }

    /// Colour of formulas drawn into the page.
    pub fn math_color(self) -> &'static str {
        match self {
            Theme::Paper => "#1f2328",
            Theme::Ink => "#e6e6e3",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
pub fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// MIME type of an image file, from its extension. `None` for anything else.
pub fn image_mime(name: &str) -> Option<&'static str> {
    let ext = name.rsplit('.').next()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        _ => return None,
    })
}

/// `data:` URI holding the bytes inline.
pub fn data_uri(mime: &str, bytes: &[u8]) -> String {
    format!("data:{mime};base64,{}", base64(bytes))
}

/// Sets `data-theme` on the page's root element. The page is the output of `wrap_document`,
/// whose root tag is fixed; anything else is returned unchanged.
pub fn with_theme(html: &str, theme: Theme) -> String {
    const ROOT: &str = "<html lang=\"zh-CN\">";
    match html.find(ROOT) {
        Some(at) => {
            let mut out = String::with_capacity(html.len() + 32);
            out.push_str(&html[..at]);
            out.push_str(&format!(
                "<html lang=\"zh-CN\" data-theme=\"{}\">",
                theme.attr()
            ));
            out.push_str(&html[at + ROOT.len()..]);
            out
        }
        None => html.to_owned(),
    }
}

/// Replaces every `src="href"` of the page with the inline `data` URI.
pub fn inline_src(html: &str, href: &str, data: &str) -> String {
    html.replace(&format!("src=\"{href}\""), &format!("src=\"{data}\""))
}

/// Note text without its frontmatter (the Markdown copy).
pub fn markdown_body(text: &str) -> &str {
    let (_, start) = jiyue_md::frontmatter::split(text);
    text.get(start..).unwrap_or(text)
}

/// A file name from a note title: characters Windows rejects become `_`, blanks fall back to
/// `笔记`. The extension is added by the caller.
pub fn file_stem(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches('.').trim();
    if trimmed.is_empty() {
        "笔记".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Name of the file the single-note export writes, for a note title.
pub fn html_file_name(title: &str) -> String {
    format!("{}.html", file_stem(title))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn image_mime_by_extension() {
        assert_eq!(image_mime("a.PNG"), Some("image/png"));
        assert_eq!(image_mime("photo.jpeg"), Some("image/jpeg"));
        assert_eq!(image_mime("doc.pdf"), None);
        assert_eq!(image_mime("noext"), None);
    }

    #[test]
    fn data_uri_has_the_mime_and_payload() {
        assert_eq!(data_uri("image/png", b"f"), "data:image/png;base64,Zg==");
    }

    #[test]
    fn theme_attribute_is_set_on_the_root_tag_only() {
        let page = "<!doctype html>\n<html lang=\"zh-CN\">\n<body></body>";
        let ink = with_theme(page, Theme::Ink);
        assert!(ink.contains("<html lang=\"zh-CN\" data-theme=\"dark\">"));
        let paper = with_theme(page, Theme::Paper);
        assert!(paper.contains("data-theme=\"light\""));
        assert_eq!(with_theme("<p>x</p>", Theme::Ink), "<p>x</p>");
    }

    #[test]
    fn inline_src_replaces_only_matching_attributes() {
        let html = "<img src=\"a/b.png\"><img src=\"a/b.png.bak\"><a href=\"a/b.png\">";
        let out = inline_src(html, "a/b.png", "data:x");
        assert_eq!(
            out,
            "<img src=\"data:x\"><img src=\"a/b.png.bak\"><a href=\"a/b.png\">"
        );
    }

    #[test]
    fn markdown_copy_drops_the_frontmatter() {
        let text = "---\ntags: [a]\n---\n# 标题\n正文\n";
        assert_eq!(markdown_body(text), "# 标题\n正文\n");
        assert_eq!(markdown_body("# 无属性\n"), "# 无属性\n");
    }

    #[test]
    fn file_names_are_safe_on_windows() {
        assert_eq!(file_stem("a/b:c?"), "a_b_c_");
        assert_eq!(file_stem("  "), "笔记");
        assert_eq!(file_stem("末尾点."), "末尾点");
        assert_eq!(html_file_name("沉积的想法"), "沉积的想法.html");
    }

    #[test]
    fn theme_labels_and_indexes() {
        assert_eq!(Theme::Paper.label(), "纸张");
        assert_eq!(Theme::Ink.index(), 1);
        assert_eq!(Theme::default(), Theme::Paper);
    }
}
