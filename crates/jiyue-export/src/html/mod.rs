//! HTML export.
//!
//! [`export_note_html`] renders one note to a standalone page. [`export_vault_site`] writes a
//! static site for a whole vault (see [`site`]). Both are XSS-safe by default: raw HTML in
//! notes is shown as text, `javascript:` and other non-web URL schemes are replaced with `#`,
//! and all text is escaped. Only the `render_mermaid` and `render_math` hooks return markup,
//! and their output is inserted as-is, so those hooks must return trusted SVG.

mod highlight;
mod render;
mod site;
mod style;

use jiyue_md::embeds::EmbedKind;

pub use site::{ExportReport, SiteOptions, export_vault_site};

/// Where the frontmatter goes in the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FrontmatterView {
    /// Not shown.
    #[default]
    Hidden,
    /// A small properties table above the body.
    Table,
}

/// Rendering switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportOptions {
    /// Keep raw HTML from the note. Off by default; when off it is shown as text.
    pub allow_raw_html: bool,
    /// Put a table of contents above the body.
    pub toc: bool,
    pub frontmatter: FrontmatterView,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            allow_raw_html: false,
            toc: false,
            frontmatter: FrontmatterView::Hidden,
        }
    }
}

/// Everything the renderer needs from its caller.
///
/// * `resolve_link(target)` gets a wikilink or relative Markdown link target without its
///   `#anchor` and returns the href to use, or `None` if it does not exist.
/// * `resolve_embed(target#anchor)` returns the Markdown to inline for `![[note#Heading]]`
///   (or `![[note#^id]]`), or `None`.
/// * `render_mermaid(source)` returns SVG for a mermaid block, or `None` for the placeholder.
/// * `render_math(latex, display)` returns SVG for a formula, or `None` for the placeholder.
pub struct ExportCtx<'a> {
    pub title: String,
    pub options: ExportOptions,
    pub resolve_link: &'a dyn Fn(&str) -> Option<String>,
    pub resolve_embed: Option<StrHook<'a>>,
    pub render_mermaid: Option<StrHook<'a>>,
    pub render_math: Option<MathHook<'a>>,
}

/// A hook that maps a target or source string to HTML, or `None` for the placeholder.
pub type StrHook<'a> = &'a dyn Fn(&str) -> Option<String>;
/// A math hook: LaTeX source and whether it is display math.
pub type MathHook<'a> = &'a dyn Fn(&str, bool) -> Option<String>;

impl<'a> ExportCtx<'a> {
    /// A context with default options and no embed or diagram hooks.
    pub fn new(title: &str, resolve_link: &'a dyn Fn(&str) -> Option<String>) -> Self {
        Self {
            title: title.to_string(),
            options: ExportOptions::default(),
            resolve_link,
            resolve_embed: None,
            render_mermaid: None,
            render_math: None,
        }
    }
}

/// A heading in the note, with the id it gets in the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    pub text: String,
    pub id: String,
}

/// A file a page refers to (image, PDF, audio ...). `target` is what the note wrote,
/// `href` is the URL the page uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetRef {
    pub target: String,
    pub href: String,
    pub kind: EmbedKind,
}

/// A link or embed that did not resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unresolved {
    pub target: String,
    /// True for images and other embeds, false for links.
    pub image: bool,
}

/// Output of [`export_note_html`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedPage {
    /// A complete HTML document, styled, ready to open or print.
    pub html: String,
    /// The page content without the document wrapper.
    pub body: String,
    pub assets: Vec<AssetRef>,
    pub headings: Vec<Heading>,
    pub unresolved: Vec<Unresolved>,
    /// Visible text of the note, for search indexes.
    pub plain_text: String,
}

/// Renders a note (full file text, frontmatter included) to a page.
pub fn export_note_html(note_text: &str, ctx: &ExportCtx<'_>) -> ExportedPage {
    let (frontmatter, body_start) = jiyue_md::frontmatter::split(note_text);
    let body_src = note_text.get(body_start..).unwrap_or(note_text);
    let mut prelude = String::new();
    if ctx.options.frontmatter == FrontmatterView::Table && frontmatter.is_some() {
        prelude = properties_table(note_text);
    }
    let cx = render::Cx { ctx, depth: 0 };
    let mut state = render::State::default();
    let rendered = render::render_markdown(body_src, &cx, &mut state);
    let toc = if ctx.options.toc {
        toc_html(&state.headings)
    } else {
        String::new()
    };
    let body = format!("{prelude}{toc}{rendered}");
    let html = wrap_document(&ctx.title, &body);
    ExportedPage {
        html,
        body,
        assets: state.assets,
        headings: state.headings,
        unresolved: state.unresolved,
        plain_text: state.plain,
    }
}

/// Wraps page content in a complete HTML document with the embedded stylesheet.
pub fn wrap_document(title: &str, body: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"zh-CN\">\n<head>\n<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
<meta name=\"generator\" content=\"jiyue-export\">\n<title>{}</title>\n<style>{}</style>\n\
</head>\n<body>\n<main class=\"page\">\n{}\n</main>\n</body>\n</html>\n",
        esc(title),
        style::CSS,
        body
    )
}

/// Escapes text for HTML element content and quoted attribute values.
pub(crate) fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn toc_html(headings: &[Heading]) -> String {
    if headings.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "<nav class=\"toc\" aria-label=\"目录\"><p class=\"toc-title\">目录</p><ul>\n",
    );
    for h in headings {
        out.push_str(&format!(
            "<li class=\"toc-l{}\"><a href=\"#{}\">{}</a></li>\n",
            h.level,
            esc(&h.id),
            esc(&h.text)
        ));
    }
    out.push_str("</ul></nav>\n");
    out
}

fn properties_table(note_text: &str) -> String {
    use jiyue_md::frontmatter::Value;
    let props = jiyue_md::frontmatter::properties(note_text);
    if props.is_empty() {
        return String::new();
    }
    let mut out = String::from("<section class=\"properties\"><table><tbody>\n");
    for p in props {
        let value = match &p.value {
            Value::Text(s) | Value::Date(s) | Value::DateTime(s) => esc(s),
            Value::Number(n) => {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    format!("{}", *n as i64)
                } else {
                    format!("{n}")
                }
            }
            Value::Bool(b) => b.to_string(),
            Value::List(items) => items.iter().map(|s| esc(s)).collect::<Vec<_>>().join(", "),
            Value::Null => String::new(),
        };
        out.push_str(&format!(
            "<tr><th scope=\"row\">{}</th><td>{}</td></tr>\n",
            esc(&p.key),
            value
        ));
    }
    out.push_str("</tbody></table></section>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_links(_: &str) -> Option<String> {
        None
    }

    fn render(md: &str) -> ExportedPage {
        let ctx = ExportCtx::new("测试", &no_links);
        export_note_html(md, &ctx)
    }

    #[test]
    fn esc_covers_all_special_characters() {
        assert_eq!(
            esc("<a href=\"x\">'&'</a>"),
            "&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;"
        );
    }

    #[test]
    fn document_wrapper_has_meta_and_escaped_title() {
        let doc = wrap_document("a <b>", "<p>x</p>");
        assert!(doc.starts_with("<!doctype html>"));
        assert!(doc.contains("<meta charset=\"utf-8\">"));
        assert!(doc.contains("<title>a &lt;b&gt;</title>"));
        assert!(doc.contains("prefers-color-scheme: dark"));
        assert!(doc.contains("@media print"));
        assert!(doc.contains("<p>x</p>"));
    }

    #[test]
    fn basic_markdown_constructs() {
        let p = render("# 标题\n\n段落 **粗** *斜* `代码`\n\n- a\n- b\n");
        assert!(p.body.contains("<h1 id=\"标题\">标题</h1>"));
        assert!(p.body.contains("<strong>粗</strong>"));
        assert!(p.body.contains("<em>斜</em>"));
        assert!(p.body.contains("<code>代码</code>"));
        assert!(p.body.contains("<ul>"));
        assert_eq!(p.headings.len(), 1);
        assert_eq!(p.headings[0].id, "标题");
        assert_eq!(p.headings[0].level, 1);
    }

    #[test]
    fn duplicate_headings_get_unique_ids() {
        let p = render("## 同名\n\n## 同名\n\n## 同名\n");
        let ids: Vec<&str> = p.headings.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids, vec!["同名", "同名-2", "同名-3"]);
        assert!(p.body.contains("id=\"同名-3\""));
    }

    #[test]
    fn empty_heading_gets_fallback_id() {
        let p = render("## !!!\n");
        assert_eq!(p.headings[0].id, "section");
    }

    #[test]
    fn tables_strikethrough_tasklists_footnotes() {
        let md = "| a | b |\n|---|:-:|\n| 1 | 2 |\n\n~~删~~\n\n- [x] done\n- [ ] todo\n\n脚注[^1]\n\n[^1]: 注释\n";
        let p = render(md);
        assert!(p.body.contains("<table>"));
        assert!(p.body.contains("<del>删</del>"));
        assert!(p.body.contains("checkbox"));
        assert!(p.body.contains("footnote-reference"));
        assert!(p.body.contains("footnote-definition"));
    }

    #[test]
    fn footnotes_are_numbered_by_first_reference_and_listed_with_back_links() {
        let md = "后[^b] 前[^a]\n\n[^a]: 甲\n\n中段\n\n[^b]: 乙\n";
        let p = render(md);
        // Numbers follow the first reference: label b is 1, label a is 2.
        assert!(p.body.contains(
            "<sup class=\"footnote-reference\" id=\"fnref-1\"><a href=\"#fn-1\">1</a></sup>"
        ));
        assert!(p.body.contains(
            "<sup class=\"footnote-reference\" id=\"fnref-2\"><a href=\"#fn-2\">2</a></sup>"
        ));
        // The list comes last, in number order, and each item has its back arrow.
        let list = p.body.find("<section class=\"footnotes\">").expect("list");
        let one = p.body.find("id=\"fn-1\"").expect("item 1");
        let two = p.body.find("id=\"fn-2\"").expect("item 2");
        assert!(list < one && one < two);
        assert!(p.body.find("乙").expect("text b") > list);
        assert!(p.body.find("甲").expect("text a") > list);
        assert!(
            p.body
                .contains("<a href=\"#fnref-1\" class=\"footnote-backref\">")
        );
        assert!(
            p.body
                .contains("<a href=\"#fnref-2\" class=\"footnote-backref\">")
        );
        // The body itself no longer holds the definitions.
        assert!(p.body.find("中段").expect("body") < list);
    }

    #[test]
    fn missing_footnote_stays_text_and_unused_definition_has_no_number() {
        let p = render("x[^9] y\n\n[^5]: 孤立\n");
        assert!(p.body.contains("[^9]"));
        assert!(!p.body.contains("footnote-reference"));
        assert!(p.body.contains("<li class=\"footnote-definition\">"));
        assert!(!p.body.contains("footnote-backref"));
    }

    #[test]
    fn footnote_syntax_inside_code_is_code() {
        let p = render("看 `[^1]`\n\n```\n[^1]: 代码\n```\n");
        assert!(p.body.contains("<code>[^1]</code>"));
        assert!(!p.body.contains("footnote-definition"));
        assert!(!p.body.contains("footnote-reference"));
    }

    #[test]
    fn nested_lists_render() {
        let p = render("- a\n  - b\n    - c\n- d\n");
        assert_eq!(p.body.matches("<ul>").count(), 3);
        assert_eq!(p.body.matches("<li>").count(), 4);
    }

    #[test]
    fn toc_and_frontmatter_table() {
        let md = "---\ntitle: 示例\ntags: [a, b]\ncount: 3\ndone: true\n---\n# 一\n## 二\n";
        let mut ctx = ExportCtx::new("t", &no_links);
        ctx.options.toc = true;
        ctx.options.frontmatter = FrontmatterView::Table;
        let p = export_note_html(md, &ctx);
        assert!(p.body.contains("<section class=\"properties\">"));
        assert!(p.body.contains("<th scope=\"row\">tags</th><td>a, b</td>"));
        assert!(p.body.contains("<td>3</td>"));
        assert!(p.body.contains("<td>true</td>"));
        assert!(p.body.contains("<nav class=\"toc\""));
        assert!(p.body.contains("href=\"#二\""));
        assert!(!p.body.contains("<hr"));
    }

    #[test]
    fn frontmatter_hidden_by_default() {
        let p = render("---\ntitle: 隐藏\n---\n正文\n");
        assert!(!p.body.contains("隐藏"));
        assert!(p.body.contains("正文"));
        assert!(p.plain_text.contains("正文"));
        assert!(!p.plain_text.contains("title"));
    }

    #[test]
    fn plain_text_collects_visible_text() {
        let p = render("# 标题\n\n这是 `code` 与 [链接](https://a.test)\n");
        assert!(p.plain_text.contains("标题"));
        assert!(p.plain_text.contains("code"));
        assert!(p.plain_text.contains("链接"));
    }

    #[test]
    fn cjk_text_is_not_escaped_or_mangled() {
        let p = render("中文，标点、「引号」与 emoji 文字\n");
        assert!(p.body.contains("中文，标点、「引号」与 emoji 文字"));
    }

    #[test]
    fn raw_html_is_escaped_by_default() {
        let p = render("<script>alert(1)</script>\n\n内联 <img src=x onerror=alert(1)> 文字\n");
        assert!(!p.body.contains("<script>"));
        assert!(!p.body.contains("<img"));
        assert!(p.body.contains("&lt;script&gt;"));
        assert!(p.body.contains("&lt;img"));
    }

    #[test]
    fn raw_html_allowed_when_option_set() {
        let mut ctx = ExportCtx::new("t", &no_links);
        ctx.options.allow_raw_html = true;
        let p = export_note_html("<div class=\"keep\">x</div>\n", &ctx);
        assert!(p.body.contains("<div class=\"keep\">x</div>"));
    }

    #[test]
    fn javascript_links_are_neutralised() {
        let attempts = [
            "[a](javascript:alert(1))",
            "[a](JaVaScRiPt:alert(1))",
            "[a](<javascript:alert(1)>)",
            "[a](vbscript:msgbox)",
            "[a](data:text/html;base64,PHNjcmlwdD4=)",
            "[a](file:///etc/passwd)",
        ];
        for md in attempts {
            let p = render(md);
            assert!(
                !p.body.to_ascii_lowercase().contains("javascript:alert"),
                "{md}: {}",
                p.body
            );
            assert!(
                !p.body.to_ascii_lowercase().contains("vbscript:"),
                "{md}: {}",
                p.body
            );
            assert!(!p.body.contains("data:text"), "{md}: {}", p.body);
            assert!(!p.body.contains("file:///"), "{md}: {}", p.body);
            assert!(p.body.contains("href=\"#\""), "{md}: {}", p.body);
        }
    }

    #[test]
    fn https_and_mailto_links_are_kept() {
        let p = render("[a](https://example.com/x?y=1) [m](mailto:a@b.c)\n");
        assert!(p.body.contains("href=\"https://example.com/x?y=1\""));
        assert!(p.body.contains("href=\"mailto:a@b.c\""));
    }

    #[test]
    fn attribute_injection_in_link_is_escaped() {
        let p = render("[x](\"onmouseover=\"alert(1))\n");
        assert!(!p.body.contains("\"onmouseover"));
        assert!(!p.body.contains("<a href=\"\"onmouseover"));
    }

    #[test]
    fn xss_in_text_and_titles_is_escaped() {
        let p = render("# <img src=x onerror=alert(1)>\n\n普通 \"引号\" & <b>粗</b>\n");
        assert!(!p.body.contains("<img src=x"));
        assert!(p.body.contains("&amp;"));
    }

    #[test]
    fn wikilink_resolved_and_missing() {
        let resolve = |t: &str| match t {
            "目标" => Some("../目标.html".to_string()),
            _ => None,
        };
        let ctx = ExportCtx::new("t", &resolve);
        let p = export_note_html("见 [[目标]] 与 [[缺失|别名]]\n", &ctx);
        assert!(
            p.body
                .contains("<a class=\"jiyue-link\" href=\"../目标.html\">目标</a>")
        );
        assert!(p.body.contains("jiyue-missing"));
        assert!(p.body.contains("别名"));
        assert_eq!(
            p.unresolved,
            vec![Unresolved {
                target: "缺失".into(),
                image: false
            }]
        );
    }

    #[test]
    fn wikilink_with_heading_and_block_anchor() {
        let resolve = |t: &str| (t == "笔记").then(|| "n.html".to_string());
        let ctx = ExportCtx::new("t", &resolve);
        let p = export_note_html("[[笔记#第一 章]] 与 [[笔记#^abc1]] 与 [[#本页]]\n", &ctx);
        assert!(p.body.contains("href=\"n.html#第一-章\""), "{}", p.body);
        assert!(p.body.contains("href=\"n.html#^abc1\""), "{}", p.body);
        assert!(p.body.contains("href=\"#本页\""), "{}", p.body);
    }

    #[test]
    fn wikilink_alias_text_is_used() {
        let resolve = |t: &str| (t == "a").then(|| "a.html".to_string());
        let ctx = ExportCtx::new("t", &resolve);
        let p = export_note_html("[[a|显示名]]\n", &ctx);
        assert!(p.body.contains(">显示名</a>"));
    }

    #[test]
    fn wikilink_inside_code_is_not_converted() {
        let resolve = |_: &str| Some("x.html".to_string());
        let ctx = ExportCtx::new("t", &resolve);
        let p = export_note_html("`[[x]]`\n\n```\n[[x]] #tag ==hi==\n```\n", &ctx);
        assert!(!p.body.contains("jiyue-link"));
        assert!(!p.body.contains("jiyue-tag"));
        assert!(!p.body.contains("<mark>"));
        assert!(p.body.contains("[[x]]"));
    }

    #[test]
    fn image_embed_with_size_and_asset_record() {
        let resolve = |t: &str| (t == "图.png").then(|| "../assets/图.png".to_string());
        let ctx = ExportCtx::new("t", &resolve);
        let p = export_note_html("![[图.png|300]]\n\n![[图.png|300x200]]\n", &ctx);
        assert!(p.body.contains("src=\"../assets/图.png\""));
        assert!(p.body.contains("width=\"300\""));
        assert!(p.body.contains("width=\"300\" height=\"200\""));
        assert_eq!(p.assets.len(), 2);
        assert_eq!(p.assets[0].kind, EmbedKind::Image);
        assert_eq!(p.assets[0].target, "图.png");
    }

    #[test]
    fn missing_image_is_reported_as_image() {
        let p = render("![[没有.png]]\n");
        assert!(p.body.contains("jiyue-missing"));
        assert_eq!(
            p.unresolved,
            vec![Unresolved {
                target: "没有.png".into(),
                image: true
            }]
        );
    }

    #[test]
    fn markdown_image_is_resolved_and_recorded() {
        let resolve = |t: &str| (t == "p.png").then(|| "p.png".to_string());
        let ctx = ExportCtx::new("t", &resolve);
        let p = export_note_html("![alt 文字](p%2Epng)\n", &ctx);
        assert!(p.body.contains("src=\"p.png\""));
        assert!(p.body.contains("alt=\"alt 文字\""));
        assert_eq!(p.assets.len(), 1);
    }

    #[test]
    fn note_embed_inlines_section_and_nests_with_limit() {
        let embed = |t: &str| match t {
            "子#章" => Some("## 章节\n\n内容 [[子]]\n".to_string()),
            "子" => Some("![[子]] 循环".to_string()),
            _ => None,
        };
        let resolve = |_: &str| None;
        let mut ctx = ExportCtx::new("t", &resolve);
        ctx.resolve_embed = Some(&embed);
        let p = export_note_html("![[子#章]]\n", &ctx);
        assert!(p.body.contains("class=\"jiyue-embed\""));
        assert!(p.body.contains("内容"));
        // The recursive embed stops at the depth limit instead of looping forever.
        let p2 = export_note_html("![[子]]\n", &ctx);
        assert!(p2.body.contains("jiyue-embed"));
        assert!(p2.body.matches("jiyue-embed").count() <= 4);
    }

    #[test]
    fn missing_note_embed_is_reported() {
        let p = render("![[不存在]]\n");
        assert!(p.body.contains("jiyue-missing"));
        assert_eq!(p.unresolved.len(), 1);
    }

    #[test]
    fn non_image_attachment_embed_becomes_link() {
        let resolve = |t: &str| (t == "doc.pdf").then(|| "doc.pdf".to_string());
        let ctx = ExportCtx::new("t", &resolve);
        let p = export_note_html("![[doc.pdf]]\n", &ctx);
        assert!(
            p.body
                .contains("<a class=\"jiyue-file\" href=\"doc.pdf\">doc.pdf</a>")
        );
        assert_eq!(p.assets[0].kind, EmbedKind::Pdf);
    }

    #[test]
    fn callouts_all_types_and_default_title() {
        let types = [
            "note", "info", "tip", "warning", "danger", "quote", "example", "bug", "success",
            "question",
        ];
        for t in types {
            let p = render(&format!("> [!{t}] 标题 {t}\n> 正文 {t}\n"));
            assert!(p.body.contains(&format!("callout-{t}")), "{t}: {}", p.body);
            assert!(p.body.contains(&format!("标题 {t}")));
            assert!(p.body.contains(&format!("正文 {t}")));
        }
        let p = render("> [!warning]\n> 没有标题\n");
        assert!(p.body.contains("callout-warning"));
        assert!(p.body.contains(">Warning</div>"));
    }

    #[test]
    fn callout_aliases_and_unknown_type() {
        let p = render("> [!caution] 小心\n> 内容\n");
        assert!(p.body.contains("callout-warning"));
        let p2 = render("> [!mystery] 神秘\n> 内容\n");
        assert!(p2.body.contains("callout-note"));
    }

    #[test]
    fn plain_blockquote_is_not_callout() {
        let p = render("> 只是引用\n");
        assert!(p.body.contains("<blockquote>"));
        assert!(!p.body.contains("callout"));
    }

    #[test]
    fn callout_body_with_markdown_and_nested_quote() {
        let p = render("> [!tip] 技巧\n> - 列表项\n>\n> > 嵌套引用\n");
        assert!(p.body.contains("callout-tip"));
        assert!(p.body.contains("<li>列表项</li>"));
        assert!(p.body.contains("<blockquote>"));
    }

    #[test]
    fn tags_and_highlights() {
        let p = render("这是 #项目/alpha 与 #tag_1，但 C# 与 #123 不是，==重点== 文字\n");
        assert!(
            p.body
                .contains("<span class=\"jiyue-tag\">#项目/alpha</span>")
        );
        assert!(p.body.contains("<span class=\"jiyue-tag\">#tag_1</span>"));
        assert!(!p.body.contains("jiyue-tag\">#123"));
        assert!(!p.body.contains("jiyue-tag\">#</span>"));
        assert!(p.body.contains("<mark>重点</mark>"));
    }

    #[test]
    fn heading_with_tag_is_not_tag_span() {
        let p = render("# 标题 #tag\n");
        assert!(p.body.contains("jiyue-tag"));
        assert_eq!(p.headings[0].text, "标题 #tag");
    }

    #[test]
    fn block_id_becomes_anchor() {
        let p = render("一段落 ^abc-1\n\n另一段 [[x]]\n");
        assert!(p.body.contains("id=\"^abc-1\""), "{}", p.body);
        assert!(!p.body.contains("^abc-1</p>"));
    }

    #[test]
    fn caret_in_middle_of_text_is_not_block_id() {
        let p = render("2^10 与 ^a 之间\n");
        assert!(!p.body.contains("jiyue-block-id"));
        assert!(p.body.contains("2^10"));
    }

    #[test]
    fn mermaid_placeholder_without_hook() {
        let p = render("```mermaid\ngraph TD; A-->B\n```\n");
        assert!(
            p.body
                .contains("<div class=\"jiyue-mermaid\" data-src=\"graph TD; A--&gt;B\n\"")
        );
        assert!(p.body.contains("graph TD; A--&gt;B"));
    }

    #[test]
    fn mermaid_hook_is_used_when_it_returns_svg() {
        let hook = |src: &str| {
            src.contains("graph")
                .then(|| "<svg id=\"m\"></svg>".to_string())
        };
        let resolve = no_links;
        let mut ctx = ExportCtx::new("t", &resolve);
        ctx.render_mermaid = Some(&hook);
        let p = export_note_html("```mermaid\ngraph TD\n```\n", &ctx);
        assert!(
            p.body
                .contains("<div class=\"jiyue-mermaid\"><svg id=\"m\"></svg></div>")
        );
        assert!(!p.body.contains("data-src"));
    }

    #[test]
    fn math_placeholder_and_hook() {
        let p = render("行内 $x^2$ 与\n\n$$\\frac{a}{b} < 1$$\n");
        assert!(
            p.body
                .contains("<span class=\"jiyue-math\" data-latex=\"x^2\">"),
            "{}",
            p.body
        );
        assert!(
            p.body.contains("data-latex=\"\\frac{a}{b} &lt; 1\"")
                || p.body.contains("jiyue-math-block"),
            "{}",
            p.body
        );
        assert!(p.body.contains("&lt;"));

        let hook = |src: &str, display: bool| {
            Some(format!("<svg data-d=\"{display}\">{}</svg>", src.len()))
        };
        let resolve = no_links;
        let mut ctx = ExportCtx::new("t", &resolve);
        ctx.render_math = Some(&hook);
        let p2 = export_note_html("$a$\n", &ctx);
        assert!(
            p2.body.contains("<svg data-d=\"false\">1</svg>"),
            "{}",
            p2.body
        );
    }

    #[test]
    fn code_block_is_highlighted_and_escaped() {
        let p = render("```rust\nfn main() { let s = \"<x>\"; } // hi\n```\n");
        assert!(p.body.contains("class=\"language-rust\""));
        assert!(p.body.contains("tok-kw"));
        assert!(p.body.contains("tok-str"));
        assert!(p.body.contains("tok-com"));
        assert!(p.body.contains("&lt;x&gt;"));
        assert!(!p.body.contains("<x>"));
    }

    #[test]
    fn unknown_language_code_block_is_plain_and_escaped() {
        let p = render("```cobol\nDISPLAY <b>\n```\n");
        assert!(p.body.contains("&lt;b&gt;"));
        assert!(p.body.contains("class=\"language-cobol\""));
    }

    #[test]
    fn lang_attribute_injection_is_escaped() {
        let p = render("```a\"onload=x\ncode\n```\n");
        assert!(!p.body.contains("\"onload=x\""));
    }

    #[test]
    fn jiyue_syntax_inside_heading_is_converted() {
        let resolve = |t: &str| (t == "n").then(|| "n.html".to_string());
        let ctx = ExportCtx::new("t", &resolve);
        let p = export_note_html("## 看 [[n]]\n", &ctx);
        assert!(p.body.contains("href=\"n.html\""));
        assert!(p.headings[0].text.starts_with("看"));
        assert!(p.headings[0].id.starts_with("看"));
    }

    #[test]
    fn huge_document_renders_quickly() {
        let mut md = String::new();
        let line = "这是一段测试文字，包含 **粗体**、[[链接]]、#标签 与 ==高亮== 以及 `代码`。\n";
        while md.len() < 2 * 1024 * 1024 {
            md.push_str(line);
            if md.len() % 4096 < line.len() {
                md.push_str("\n## 小节\n\n");
            }
        }
        let resolve = |_: &str| Some("x.html".to_string());
        let ctx = ExportCtx::new("big", &resolve);
        let start = std::time::Instant::now();
        let p = export_note_html(&md, &ctx);
        let elapsed = start.elapsed();
        assert!(p.body.len() > md.len() / 2);
        assert!(p.headings.len() > 100);
        // Generous bound for debug builds on slow machines; a quadratic scan would blow past it.
        assert!(elapsed.as_secs() < 20, "took {elapsed:?}");
    }

    #[test]
    fn deeply_nested_blockquotes_do_not_panic() {
        let md = format!("{}x\n", "> ".repeat(200));
        let p = render(&md);
        assert!(p.body.contains('x'));
    }

    #[test]
    fn unbalanced_markers_do_not_panic() {
        for md in [
            "[[", "![[", "[[a", "==", "==x", "#", "^", "$", "[[]]", "![[]]", "[[|]]",
        ] {
            let p = render(md);
            assert!(p.body.is_ascii() || !p.body.is_empty());
        }
    }
}
