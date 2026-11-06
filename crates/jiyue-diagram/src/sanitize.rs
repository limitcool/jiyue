//! Rewrites renderer output into a self-contained SVG.
//!
//! The output keeps only SVG-namespace elements and drops anything that could run
//! code or fetch a resource: `script`, `foreignObject`, animation elements, external
//! `href`/`src`, external `url(...)` references, event-handler attributes and
//! `javascript:` URLs. Links (`<a>`) are unwrapped so their label stays visible.
//! Only fragment (`#id`) references and inline `data:image/` rasters survive.

use crate::error::DiagramError;

const SVG_NS: &str = "http://www.w3.org/2000/svg";
const XLINK_NS: &str = "http://www.w3.org/1999/xlink";
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
const MAX_DEPTH: usize = 256;

/// Elements removed together with their subtree.
const DROP_ELEMENTS: &[&str] = &[
    "script",
    "foreignObject",
    "iframe",
    "object",
    "embed",
    "audio",
    "video",
    "animate",
    "animateMotion",
    "animateTransform",
    "set",
    "handler",
    "listener",
    "feImage",
    "use",
];

/// Attributes that may carry a URL and must be fragment-only.
const URL_ATTRIBUTES: &[&str] = &[
    "href",
    "src",
    "action",
    "formaction",
    "data",
    "poster",
    "background",
];

/// Returns a self-contained copy of `input`, or an error when `input` is not an SVG document.
pub(crate) fn sanitize_svg(input: &str) -> Result<String, DiagramError> {
    let doc = roxmltree::Document::parse(input)
        .map_err(|e| DiagramError::Render(format!("renderer produced invalid XML: {e}")))?;
    let root = doc.root_element();
    if root.tag_name().name() != "svg" || root.tag_name().namespace() != Some(SVG_NS) {
        return Err(DiagramError::Render(
            "renderer did not produce an SVG root".into(),
        ));
    }

    let mut prefixes: Vec<(String, String)> = Vec::new();
    for ns in root.namespaces() {
        if let Some(name) = ns.name() {
            prefixes.push((ns.uri().to_string(), name.to_string()));
        }
    }

    let mut out = String::with_capacity(input.len());
    out.push_str("<svg xmlns=\"");
    out.push_str(SVG_NS);
    out.push_str("\" xmlns:xlink=\"");
    out.push_str(XLINK_NS);
    out.push('"');
    for (uri, prefix) in &prefixes {
        if prefix == "xlink" {
            continue;
        }
        out.push_str(" xmlns:");
        out.push_str(prefix);
        out.push_str("=\"");
        out.push_str(&escape_attr(uri));
        out.push('"');
    }
    let ctx = Ctx {
        prefixes: &prefixes,
    };
    write_attributes(root, &ctx, &mut out, true);
    out.push('>');
    for child in root.children() {
        write_node(child, &ctx, &mut out, 1)?;
    }
    out.push_str("</svg>");
    Ok(out)
}

struct Ctx<'a> {
    prefixes: &'a [(String, String)],
}

fn write_node(
    node: roxmltree::Node,
    ctx: &Ctx,
    out: &mut String,
    depth: usize,
) -> Result<(), DiagramError> {
    if depth > MAX_DEPTH {
        return Err(DiagramError::Render("SVG nesting is too deep".into()));
    }
    if node.is_text() {
        if let Some(text) = node.text() {
            out.push_str(&escape_text(text));
        }
        return Ok(());
    }
    if !node.is_element() {
        return Ok(());
    }
    let tag = node.tag_name();
    if tag.namespace() != Some(SVG_NS) {
        return Ok(());
    }
    let local = tag.name();
    if DROP_ELEMENTS.contains(&local) {
        return Ok(());
    }
    match local {
        "a" => {
            // Unwrap links: keep the visible content, drop the navigation target.
            for child in node.children() {
                write_node(child, ctx, out, depth + 1)?;
            }
            return Ok(());
        }
        "image" => {
            let href = node
                .attribute("href")
                .or_else(|| node.attribute((XLINK_NS, "href")));
            match href {
                Some(value) if is_inline_raster(value) => {}
                _ => return Ok(()),
            }
        }
        "style" => {
            let text: String = node
                .children()
                .filter_map(|c| c.text())
                .collect::<Vec<_>>()
                .join("");
            if !css_is_safe(&text) {
                return Ok(());
            }
        }
        _ => {}
    }

    out.push('<');
    out.push_str(local);
    write_attributes(node, ctx, out, false);
    if node.has_children() {
        out.push('>');
        for child in node.children() {
            write_node(child, ctx, out, depth + 1)?;
        }
        out.push_str("</");
        out.push_str(local);
        out.push('>');
    } else {
        out.push_str("/>");
    }
    Ok(())
}

fn write_attributes(node: roxmltree::Node, ctx: &Ctx, out: &mut String, is_root: bool) {
    let element = node.tag_name().name();
    for attr in node.attributes() {
        let name = match attr.namespace() {
            None => attr.name().to_string(),
            Some(XLINK_NS) => format!("xlink:{}", attr.name()),
            Some(XML_NS) => format!("xml:{}", attr.name()),
            Some(SVG_NS) => attr.name().to_string(),
            Some(uri) => match ctx.prefixes.iter().find(|(u, _)| u == uri) {
                Some((_, prefix)) => format!("{prefix}:{}", attr.name()),
                None => continue,
            },
        };
        let value = attr.value();
        if !attribute_is_safe(attr.name(), element, value) {
            continue;
        }
        if is_root && (name == "xmlns" || name.starts_with("xmlns:")) {
            continue;
        }
        out.push(' ');
        out.push_str(&name);
        out.push_str("=\"");
        out.push_str(&escape_attr(value));
        out.push('"');
    }
}

fn attribute_is_safe(name: &str, element: &str, value: &str) -> bool {
    let lname = name.to_ascii_lowercase();
    if lname.starts_with("on") {
        return false;
    }
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("javascript:")
        || lower.contains("vbscript:")
        || lower.contains("data:text/html")
    {
        return false;
    }
    if URL_ATTRIBUTES.contains(&lname.as_str()) {
        if trimmed.starts_with('#') {
            return true;
        }
        return element == "image" && lname == "href" && is_inline_raster(trimmed);
    }
    if lower.contains("@import") || lower.contains("expression(") {
        return false;
    }
    !has_external_url_ref(&lower)
}

/// True when a CSS/attribute value contains `url(...)` pointing anywhere except a fragment.
fn has_external_url_ref(lower: &str) -> bool {
    let mut rest = lower;
    while let Some(pos) = rest.find("url(") {
        let after = &rest[pos + 4..];
        let end = after.find(')').unwrap_or(after.len());
        let target = after[..end]
            .trim()
            .trim_matches(|c| c == '\'' || c == '"')
            .trim();
        if !target.starts_with('#') {
            return true;
        }
        rest = &after[end..];
    }
    false
}

fn css_is_safe(css: &str) -> bool {
    let lower = css.to_ascii_lowercase();
    !(lower.contains("@import")
        || lower.contains("javascript:")
        || lower.contains("expression(")
        || has_external_url_ref(&lower))
}

fn is_inline_raster(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    lower.starts_with("data:image/")
        && !lower.starts_with("data:image/svg")
        && (lower.starts_with("data:image/png")
            || lower.starts_with("data:image/jpeg")
            || lower.starts_with("data:image/gif")
            || lower.starts_with("data:image/webp"))
}

fn escape_text(value: &str) -> String {
    let mut s = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => s.push_str("&amp;"),
            '<' => s.push_str("&lt;"),
            '>' => s.push_str("&gt;"),
            other => s.push(other),
        }
    }
    s
}

fn escape_attr(value: &str) -> String {
    let mut s = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => s.push_str("&amp;"),
            '<' => s.push_str("&lt;"),
            '>' => s.push_str("&gt;"),
            '"' => s.push_str("&quot;"),
            '\n' => s.push_str("&#10;"),
            '\r' => s.push_str("&#13;"),
            '\t' => s.push_str("&#9;"),
            other => s.push(other),
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wrap(body: &str) -> String {
        format!(
            "<svg xmlns=\"{SVG_NS}\" xmlns:xlink=\"{XLINK_NS}\" width=\"10\" height=\"10\">{body}</svg>"
        )
    }

    fn clean(body: &str) -> String {
        let out = sanitize_svg(&wrap(body)).unwrap_or_default();
        assert!(
            roxmltree::Document::parse(&out).is_ok(),
            "output is not XML: {out}"
        );
        out
    }

    #[test]
    fn drops_scripts_and_foreign_objects() {
        let out = clean(
            "<script>alert(1)</script><foreignObject><div>x</div></foreignObject><rect width=\"1\" height=\"1\"/>",
        );
        assert!(!out.contains("script"));
        assert!(!out.contains("foreignObject"));
        assert!(out.contains("<rect"));
    }

    #[test]
    fn drops_event_handlers() {
        let out = clean("<rect onclick=\"steal()\" onload=\"x\" width=\"1\" height=\"1\"/>");
        assert!(!out.to_ascii_lowercase().contains("onclick"));
        assert!(!out.to_ascii_lowercase().contains("onload"));
    }

    #[test]
    fn unwraps_links_and_drops_href() {
        let out = clean("<a href=\"https://evil.example/\"><text x=\"1\" y=\"1\">Click</text></a>");
        assert!(!out.contains("evil.example"));
        assert!(!out.contains("<a"));
        assert!(out.contains("Click"));
    }

    #[test]
    fn keeps_fragment_hrefs_and_markers() {
        let out = clean(
            "<defs><marker id=\"m\"/></defs><path d=\"M0 0\" marker-end=\"url(#m)\"/><use href=\"#m\"/>",
        );
        assert!(out.contains("marker-end=\"url(#m)\""));
    }

    #[test]
    fn strips_external_url_references() {
        let out =
            clean("<rect fill=\"url(https://evil.example/x.svg#a)\" width=\"1\" height=\"1\"/>");
        assert!(!out.contains("evil.example"));
        let out2 = clean("<rect style=\"fill:url('http://x/y')\" width=\"1\" height=\"1\"/>");
        assert!(!out2.contains("http://x"));
    }

    #[test]
    fn drops_external_images_keeps_inline_png() {
        let out = clean(
            "<image href=\"http://x/y.png\" width=\"1\" height=\"1\"/><image href=\"data:image/png;base64,AAAA\" width=\"1\" height=\"1\"/>",
        );
        assert!(!out.contains("http://x"));
        assert!(out.contains("data:image/png"));
        let svg_data =
            clean("<image href=\"data:image/svg+xml;base64,PHN2Zy8+\" width=\"1\" height=\"1\"/>");
        assert!(!svg_data.contains("svg+xml"));
    }

    #[test]
    fn drops_javascript_urls_and_animation() {
        let out = clean(
            "<a href=\"javascript:alert(1)\">x</a><set attributeName=\"href\" to=\"javascript:x\"/><animate attributeName=\"fill\"/>",
        );
        assert!(!out.to_ascii_lowercase().contains("javascript"));
        assert!(!out.contains("<set"));
        assert!(!out.contains("<animate"));
    }

    #[test]
    fn drops_unsafe_style_element_and_keeps_safe_one() {
        let bad = clean(
            "<style>@import url(https://x/css); text{fill:red}</style><rect width=\"1\" height=\"1\"/>",
        );
        assert!(!bad.contains("<style"));
        let good = clean("<style>.a{fill:#fff}</style><rect width=\"1\" height=\"1\"/>");
        assert!(good.contains("<style>"));
    }

    #[test]
    fn escapes_text_and_attributes() {
        let out = clean("<text x=\"1\" y=\"1\" data-x=\"a&quot;b\">a &lt; b &amp; c</text>");
        assert!(out.contains("a &lt; b &amp; c"));
    }

    #[test]
    fn rejects_non_svg_roots_and_garbage() {
        assert!(sanitize_svg("<html></html>").is_err());
        assert!(sanitize_svg("not xml at all").is_err());
        assert!(sanitize_svg("").is_err());
        assert!(
            sanitize_svg(
                "<!DOCTYPE svg [<!ENTITY x \"y\">]><svg xmlns=\"http://www.w3.org/2000/svg\"/>"
            )
            .is_err()
        );
    }

    #[test]
    fn drops_non_svg_namespace_elements() {
        let out = clean(
            "<x:div xmlns:x=\"http://www.w3.org/1999/xhtml\">hi</x:div><rect width=\"1\" height=\"1\"/>",
        );
        assert!(!out.contains("hi"));
        assert!(out.contains("<rect"));
    }

    #[test]
    fn deep_nesting_is_rejected_not_overflowed() {
        let depth = 400;
        let body = format!("{}{}", "<g>".repeat(depth), "</g>".repeat(depth));
        let result = sanitize_svg(&wrap(&body));
        assert!(matches!(result, Err(DiagramError::Render(_))));
    }

    #[test]
    fn output_is_idempotent() {
        let once = clean("<g><rect fill=\"#abc\" width=\"1\" height=\"1\"/></g>");
        let twice = sanitize_svg(&once).unwrap_or_default();
        assert_eq!(once, twice);
    }
}
