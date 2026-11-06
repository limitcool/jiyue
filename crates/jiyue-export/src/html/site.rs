//! Static site export of a whole vault.
//!
//! Layout: one page per published note at the same relative path (`a/b.md` becomes
//! `a/b.html`), an `index.html` listing every page, a `search-index.json` for the built-in
//! search box, and copies of the attachments that pages refer to. Nothing is fetched from
//! the network. Writes go through a temporary file and a rename.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use jiyue_core::links::{NoteRef, resolve_link};
use jiyue_md::embeds::{
    EmbedKind, LinkKind, embed_kind, extract_block_text, extract_section, headings, parse_links,
};
use jiyue_md::frontmatter::{self, Value};
use serde::Serialize;

use super::{ExportCtx, ExportOptions, FrontmatterView, esc, export_note_html, wrap_document};
use crate::templates::folder_of;

/// Settings for [`export_vault_site`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteOptions {
    /// Site name shown in the header and on the index page.
    pub title: String,
    /// Only export notes under this vault-relative folder. `None` exports the whole vault.
    pub folder: Option<String>,
    /// Table of contents on every page.
    pub toc: bool,
    /// Keep raw HTML from notes. Off by default.
    pub allow_raw_html: bool,
}

impl Default for SiteOptions {
    fn default() -> Self {
        Self {
            title: "笔记".to_string(),
            folder: None,
            toc: true,
            allow_raw_html: false,
        }
    }
}

/// What an export did. Warnings name the note and the target for every problem.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExportReport {
    /// Pages written, not counting the index.
    pub pages: usize,
    /// Notes left out because their frontmatter sets `publish: false`.
    pub skipped_unpublished: usize,
    /// Attachment files copied into the output.
    pub assets_copied: usize,
    /// Links that did not resolve to an exported note or file.
    pub unresolved_links: usize,
    /// Images or embedded files that did not resolve.
    pub missing_images: usize,
    /// Entries in `search-index.json`.
    pub search_entries: usize,
    pub warnings: Vec<String>,
}

#[derive(Serialize)]
struct SearchEntry {
    title: String,
    href: String,
    folder: String,
    text: String,
}

struct Note {
    rel: String,
    out_rel: String,
    title: String,
    text: String,
    aliases: Vec<String>,
}

/// Maximum characters of note text kept in the search index.
const SEARCH_TEXT_LIMIT: usize = 20_000;

/// Exports the vault at `root` to a static site in `out_dir`.
///
/// Hidden entries (names starting with `.`, such as `.jiyue`, `.trash`, `.obsidian`) are
/// skipped. Notes whose frontmatter says `publish: false` are not exported.
pub fn export_vault_site(
    root: &Path,
    out_dir: &Path,
    opts: &SiteOptions,
) -> io::Result<ExportReport> {
    let mut report = ExportReport::default();
    let out_canon = out_dir.canonicalize().ok();
    let mut md_files = Vec::new();
    let mut other_files = Vec::new();
    walk(
        root,
        root,
        out_canon.as_deref(),
        &mut md_files,
        &mut other_files,
    )?;
    md_files.sort();
    other_files.sort();

    let folder = opts
        .folder
        .as_deref()
        .map(|f| f.trim().trim_matches(['/', '\\']).replace('\\', "/"))
        .filter(|f| !f.is_empty());

    let mut notes: Vec<Note> = Vec::new();
    for rel in &md_files {
        if let Some(f) = &folder
            && !rel.starts_with(&format!("{f}/"))
        {
            continue;
        }
        let text = match fs::read_to_string(root.join(rel)) {
            Ok(t) => t,
            Err(e) => {
                report.warnings.push(format!("{rel}: 无法读取 ({e})"));
                continue;
            }
        };
        if frontmatter::get(&text, "publish") == Some(Value::Bool(false)) {
            report.skipped_unpublished += 1;
            continue;
        }
        let title = frontmatter::title_from(&text)
            .or_else(|| first_heading(&text))
            .unwrap_or_else(|| stem(rel));
        let aliases = frontmatter::aliases(&text);
        notes.push(Note {
            rel: rel.clone(),
            out_rel: html_path(rel),
            title,
            text,
            aliases,
        });
    }

    let refs: Vec<NoteRef> = notes
        .iter()
        .map(|n| {
            let aliases: Vec<&str> = n.aliases.iter().map(String::as_str).collect();
            NoteRef::with_aliases(n.rel.clone(), &aliases)
        })
        .collect();

    // Backlinks: for every note, the notes that link to it.
    let mut backlinks: Vec<Vec<usize>> = vec![Vec::new(); notes.len()];
    for (si, note) in notes.iter().enumerate() {
        for link in parse_links(&note.text) {
            let is_note_link = match link.kind {
                LinkKind::Wiki | LinkKind::Embed => true,
                LinkKind::MdLink => !link.external && embed_kind(&link.target) == EmbedKind::Note,
                LinkKind::MdImage => false,
            };
            if !is_note_link || link.target.is_empty() {
                continue;
            }
            if let Some(ti) = resolve_link(&refs, &link.target, &note.rel)
                && ti != si
                && !backlinks[ti].contains(&si)
            {
                backlinks[ti].push(si);
            }
        }
    }

    let site_title = opts.title.clone();
    let mut used_assets: BTreeMap<String, ()> = BTreeMap::new();
    let mut search: Vec<SearchEntry> = Vec::with_capacity(notes.len());
    let mut pages: Vec<(String, String)> = Vec::with_capacity(notes.len());

    for (si, note) in notes.iter().enumerate() {
        let from_rel = note.rel.as_str();
        let from_dir = folder_of(from_rel);
        let resolve = |target: &str| -> Option<String> {
            if embed_kind(target) == EmbedKind::Note {
                let idx = resolve_link(&refs, target, from_rel)?;
                Some(relative_href(from_dir, &notes[idx].out_rel))
            } else {
                let rel = asset_rel(&other_files, target, from_rel)?;
                Some(relative_href(from_dir, &rel))
            }
        };
        let embed = |full: &str| -> Option<String> {
            let (target, anchor) = match full.split_once('#') {
                Some((t, a)) => (t, Some(a)),
                None => (full, None),
            };
            let idx = if target.is_empty() {
                notes.iter().position(|n| n.rel == from_rel)?
            } else {
                resolve_link(&refs, target, from_rel)?
            };
            let body = body_of(&notes[idx].text);
            match anchor {
                None => Some(body.to_string()),
                Some(a) => match a.strip_prefix('^') {
                    Some(id) => extract_block_text(body, id),
                    None => extract_section(body, a),
                },
            }
        };

        let mut ctx = ExportCtx::new(&note.title, &resolve);
        ctx.options = ExportOptions {
            allow_raw_html: opts.allow_raw_html,
            toc: opts.toc,
            frontmatter: FrontmatterView::Hidden,
        };
        ctx.resolve_embed = Some(&embed);
        let page = export_note_html(&note.text, &ctx);

        for u in &page.unresolved {
            if u.image {
                report.missing_images += 1;
                report
                    .warnings
                    .push(format!("{from_rel}: 图片或附件不存在: {}", u.target));
            } else {
                report.unresolved_links += 1;
                report
                    .warnings
                    .push(format!("{from_rel}: 链接未解析: {}", u.target));
            }
        }
        for a in &page.assets {
            if let Some(rel) = asset_rel(&other_files, &a.target, from_rel) {
                used_assets.insert(rel, ());
            }
        }

        let depth_prefix = root_prefix(from_dir);
        let mut links_html = String::new();
        if !backlinks[si].is_empty() {
            links_html.push_str("<section class=\"backlinks\"><h2>反链</h2><ul>");
            for &src in &backlinks[si] {
                let other = &notes[src];
                links_html.push_str(&format!(
                    "<li><a href=\"{}\">{}</a></li>",
                    esc(&relative_href(from_dir, &other.out_rel)),
                    esc(&other.title)
                ));
            }
            links_html.push_str("</ul></section>");
        }
        let body = format!(
            "{}{}{}{}",
            header_html(&depth_prefix, &site_title),
            page.body,
            links_html,
            SEARCH_SCRIPT
        );
        pages.push((note.out_rel.clone(), wrap_document(&note.title, &body)));
        search.push(SearchEntry {
            title: note.title.clone(),
            href: note.out_rel.clone(),
            folder: folder_of(from_rel).to_string(),
            text: truncate_chars(&page.plain_text, SEARCH_TEXT_LIMIT),
        });
    }

    // Index page.
    let mut groups: BTreeMap<String, Vec<&Note>> = BTreeMap::new();
    for n in &notes {
        groups
            .entry(folder_of(&n.rel).to_string())
            .or_default()
            .push(n);
    }
    let mut index_body = String::new();
    index_body.push_str(&header_html("", &site_title));
    index_body.push_str(&format!(
        "<h1>{}</h1><p class=\"muted\">{} 篇笔记</p><div class=\"folder-list\">",
        esc(&site_title),
        notes.len()
    ));
    for (folder, mut list) in groups {
        list.sort_by(|a, b| a.title.cmp(&b.title));
        let heading = if folder.is_empty() {
            "根目录".to_string()
        } else {
            folder.clone()
        };
        index_body.push_str(&format!("<h2>{}</h2><ul>", esc(&heading)));
        for n in list {
            index_body.push_str(&format!(
                "<li><a href=\"{}\">{}</a></li>",
                esc(&encode_path(&n.out_rel)),
                esc(&n.title)
            ));
        }
        index_body.push_str("</ul>");
    }
    index_body.push_str("</div>");
    index_body.push_str(SEARCH_SCRIPT);
    let index_html = wrap_document(&site_title, &index_body);

    // Writing.
    fs::create_dir_all(out_dir)?;
    for (out_rel, html) in &pages {
        write_atomic(&out_dir.join(out_rel), html.as_bytes())?;
        report.pages += 1;
    }
    write_atomic(&out_dir.join("index.html"), index_html.as_bytes())?;
    let json = serde_json::to_string(&search).map_err(io::Error::other)?;
    write_atomic(&out_dir.join("search-index.json"), json.as_bytes())?;
    report.search_entries = search.len();

    for rel in used_assets.keys() {
        match copy_atomic(&root.join(rel), &out_dir.join(rel)) {
            Ok(()) => report.assets_copied += 1,
            Err(e) => report.warnings.push(format!("{rel}: 复制失败 ({e})")),
        }
    }
    Ok(report)
}

/// Recursively lists notes (`.md`) and other files, as `/`-separated paths relative to `root`.
/// Hidden entries and the output directory (when it already exists) are skipped.
fn walk(
    root: &Path,
    dir: &Path,
    skip: Option<&Path>,
    notes: &mut Vec<String>,
    files: &mut Vec<String>,
) -> io::Result<()> {
    let entries = if dir == root {
        fs::read_dir(dir)?.collect::<Vec<_>>()
    } else {
        match fs::read_dir(dir) {
            Ok(rd) => rd.collect(),
            Err(_) => return Ok(()),
        }
    };
    for entry in entries {
        let Ok(entry) = entry else { continue };
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            if let Some(skip) = skip
                && path.canonicalize().ok().as_deref() == Some(skip)
            {
                continue;
            }
            walk(root, &path, skip, notes, files)?;
        } else if file_type.is_file()
            && let Ok(rel) = path.strip_prefix(root)
        {
            let rel = rel.to_string_lossy().replace('\\', "/");
            if rel.to_ascii_lowercase().ends_with(".md") {
                notes.push(rel);
            } else {
                files.push(rel);
            }
        }
    }
    Ok(())
}

/// Vault-relative path of the attachment a link target names, if there is one.
///
/// Targets with a folder part match by path suffix; targets without one match by file name,
/// case-insensitively. Relative `../` targets are resolved from the note's folder. Ties go to
/// the file in the note's own folder, then the shortest path.
fn asset_rel(files: &[String], target: &str, from_rel: &str) -> Option<String> {
    let t = target.trim().replace('\\', "/");
    let t = t.trim_start_matches("./");
    if t.is_empty() {
        return None;
    }
    if t.starts_with("../") {
        let joined = normalize(&format!("{}/{}", folder_of(from_rel), t));
        return files.iter().find(|f| **f == joined).cloned();
    }
    let from_dir = folder_of(from_rel);
    let suffix = format!("/{t}");
    let has_folder = t.contains('/');
    files
        .iter()
        .filter(|f| {
            f.as_str() == t
                || f.ends_with(&suffix)
                || (!has_folder && basename(f).eq_ignore_ascii_case(t))
        })
        .min_by_key(|f| {
            let same_folder = folder_of(f) != from_dir;
            (same_folder, f.len(), f.to_string())
        })
        .cloned()
}

fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn stem(rel: &str) -> String {
    let name = basename(rel);
    let cut = name.len().saturating_sub(3);
    match name.get(cut..) {
        Some(ext) if ext.eq_ignore_ascii_case(".md") => name[..cut].to_string(),
        _ => name.to_string(),
    }
}

fn html_path(rel: &str) -> String {
    let base = match rel.get(..rel.len().saturating_sub(3)) {
        Some(b) if rel.to_ascii_lowercase().ends_with(".md") => b,
        _ => rel,
    };
    let out = format!("{base}.html");
    // A note at the top level called "index" must not replace the site index.
    if out == "index.html" {
        "index-note.html".to_string()
    } else {
        out
    }
}

fn body_of(text: &str) -> &str {
    let (_, off) = frontmatter::split(text);
    text.get(off..).unwrap_or(text)
}

fn first_heading(text: &str) -> Option<String> {
    headings(body_of(text))
        .into_iter()
        .find(|h| h.level == 1)
        .map(|h| h.text)
        .filter(|t| !t.trim().is_empty())
}

/// `../` prefix that leads from a page in `folder` back to the site root.
fn root_prefix(folder: &str) -> String {
    if folder.is_empty() {
        String::new()
    } else {
        "../".repeat(folder.split('/').count())
    }
}

/// Relative URL from a page in `from_dir` to the output file `to_rel`.
fn relative_href(from_dir: &str, to_rel: &str) -> String {
    format!("{}{}", root_prefix(from_dir), encode_path(to_rel))
}

/// Percent-encodes the characters that would break a URL path. Non-ASCII is left as is.
fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        match c {
            ' ' | '#' | '?' | '%' | '"' | '<' | '>' | '`' | '\\' => {
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).bytes() {
                    out.push_str(&format!("%{b:02X}"));
                }
            }
            c if c.is_control() => out.push_str(&format!("%{:02X}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((idx, _)) => s[..idx].to_string(),
        None => s.to_string(),
    }
}

fn header_html(root: &str, site_title: &str) -> String {
    format!(
        "<header class=\"site-top\"><a class=\"site-name\" href=\"{root}index.html\">{}</a>\
<div class=\"search\" data-root=\"{root}\"><input type=\"search\" placeholder=\"搜索笔记\" \
aria-label=\"搜索笔记\"><ul class=\"search-results\"></ul></div></header>",
        esc(site_title)
    )
}

/// Client-side search over `search-index.json`. Fixed text: it never includes note content.
const SEARCH_SCRIPT: &str = "<script>(function(){\
var box=document.querySelector('.search');if(!box){return;}\
var input=box.querySelector('input');var list=box.querySelector('.search-results');\
var root=box.getAttribute('data-root')||'';var loading=null;var all=null;\
function load(){if(!loading){loading=fetch(root+'search-index.json').then(function(r){return r.json();}).then(function(j){all=j;return j;});}return loading;}\
function line(text,msg){list.textContent='';var li=document.createElement('li');li.textContent=msg||text;list.appendChild(li);}\
function show(hits){list.textContent='';if(!hits.length){line('','没有结果');return;}\
hits.slice(0,30).forEach(function(h){var li=document.createElement('li');var a=document.createElement('a');\
a.href=root+h.item.href;a.textContent=h.item.title;li.appendChild(a);\
if(h.snippet){var s=document.createElement('span');s.className='snippet';s.textContent=h.snippet;li.appendChild(s);}\
list.appendChild(li);});}\
input.addEventListener('input',function(){var q=input.value.trim().toLowerCase();\
if(!q){list.textContent='';return;}\
load().then(function(items){var hits=[];items.forEach(function(it){var t=(it.title||'').toLowerCase();\
var body=(it.text||'');var p=body.toLowerCase().indexOf(q);var tp=t.indexOf(q);\
if(tp>=0||p>=0){var snip='';if(p>=0){snip=body.slice(Math.max(0,p-30),p+60);}\
hits.push({item:it,snippet:snip,rank:tp>=0?0:1});}});\
hits.sort(function(a,b){return a.rank-b.rank;});show(hits);})\
.catch(function(){line('','搜索索引不可用（请通过 http 服务打开）');});});\
})();</script>";

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = tmp_path(path);
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

fn copy_atomic(src: &Path, dst: &Path) -> io::Result<()> {
    if let Some(dir) = dst.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = tmp_path(dst);
    fs::copy(src, &tmp)?;
    fs::rename(&tmp, dst).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".jiyue-tmp");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{cleanup, temp_root};

    fn write(root: &Path, rel: &str, content: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, content).expect("write");
    }

    fn sample_vault(tag: &str) -> PathBuf {
        let root = temp_root(tag);
        write(
            &root,
            "a.md",
            "---\ntitle: 甲\n---\n# 甲\n\n见 [[b]] 与 [[sub/c|C 笔记]]\n\n![[img/x.png|200]]\n\n[[草稿]] 和 [[不存在]]\n\n> [!tip] 提示\n> 内容 #标签\n",
        );
        write(
            &root,
            "b.md",
            "# 乙\n\n回指 [[a]]\n\n![[a#甲]]\n\n[[#乙]]\n",
        );
        write(
            &root,
            "sub/c.md",
            "# 丙\n\n返回 [[a]] 并看 [[b#乙]] 与 [图](../img/x.png)\n\n```rust\nfn x() {}\n```\n",
        );
        write(&root, "草稿.md", "---\npublish: false\n---\n# 草稿\n秘密\n");
        write(&root, "img/x.png", "PNGDATA");
        write(&root, "img/unused.png", "UNUSED");
        write(&root, ".jiyue/hidden.md", "# 隐藏\n");
        write(&root, ".trash/gone.md", "# 回收\n");
        write(&root, ".obsidian/app.json", "{}");
        root
    }

    #[test]
    fn exports_pages_index_and_search_index() {
        let root = sample_vault("site1");
        let out = temp_root("site1-out").join("site");
        let report = export_vault_site(&root, &out, &SiteOptions::default()).expect("export");
        assert_eq!(report.pages, 3, "{:?}", report.warnings);
        assert_eq!(report.skipped_unpublished, 1);
        assert!(out.join("a.html").is_file());
        assert!(out.join("b.html").is_file());
        assert!(out.join("sub/c.html").is_file());
        assert!(out.join("index.html").is_file());
        assert!(!out.join("草稿.html").exists());
        assert!(!out.join(".jiyue").exists());
        assert!(!out.join("hidden.html").exists());
        cleanup(&root);
        cleanup(out.parent().expect("parent"));
    }

    #[test]
    fn links_resolve_between_pages() {
        let root = sample_vault("site2");
        let out = temp_root("site2-out").join("site");
        export_vault_site(&root, &out, &SiteOptions::default()).expect("export");
        let a = fs::read_to_string(out.join("a.html")).expect("a");
        assert!(a.contains("href=\"b.html\""), "{a}");
        assert!(a.contains("href=\"sub/c.html\""), "{a}");
        let c = fs::read_to_string(out.join("sub/c.html")).expect("c");
        assert!(c.contains("href=\"../a.html\""), "{c}");
        assert!(c.contains("href=\"../b.html#乙\""), "{c}");
        assert!(c.contains("href=\"../img/x.png\""), "{c}");
        cleanup(&root);
        cleanup(out.parent().expect("parent"));
    }

    #[test]
    fn backlinks_listed_on_target_page() {
        let root = sample_vault("site3");
        let out = temp_root("site3-out").join("site");
        export_vault_site(&root, &out, &SiteOptions::default()).expect("export");
        let b = fs::read_to_string(out.join("b.html")).expect("b");
        assert!(b.contains("class=\"backlinks\""), "{b}");
        assert!(b.contains("href=\"a.html\">甲</a>"), "{b}");
        assert!(b.contains("href=\"sub/c.html\">丙</a>"), "{b}");
        let a = fs::read_to_string(out.join("a.html")).expect("a");
        assert!(
            a.contains("href=\"sub/c.html\">丙</a>"),
            "backlink from c: {a}"
        );
        cleanup(&root);
        cleanup(out.parent().expect("parent"));
    }

    #[test]
    fn embedded_section_from_other_note_is_inlined() {
        let root = sample_vault("site4");
        let out = temp_root("site4-out").join("site");
        export_vault_site(&root, &out, &SiteOptions::default()).expect("export");
        let b = fs::read_to_string(out.join("b.html")).expect("b");
        assert!(b.contains("class=\"jiyue-embed\""), "{b}");
        assert!(b.contains("data-embed=\"a\""), "{b}");
        cleanup(&root);
        cleanup(out.parent().expect("parent"));
    }

    #[test]
    fn assets_are_copied_only_when_referenced() {
        let root = sample_vault("site5");
        let out = temp_root("site5-out").join("site");
        let report = export_vault_site(&root, &out, &SiteOptions::default()).expect("export");
        assert_eq!(fs::read(out.join("img/x.png")).expect("copied"), b"PNGDATA");
        assert!(!out.join("img/unused.png").exists());
        assert_eq!(report.assets_copied, 1);
        cleanup(&root);
        cleanup(out.parent().expect("parent"));
    }

    #[test]
    fn unresolved_links_and_missing_images_are_reported() {
        let root = sample_vault("site6");
        let out = temp_root("site6-out").join("site");
        let report = export_vault_site(&root, &out, &SiteOptions::default()).expect("export");
        // 草稿 is unpublished, 不存在 does not exist.
        assert!(report.unresolved_links >= 2, "{:?}", report.warnings);
        assert!(report.warnings.iter().any(|w| w.contains("不存在")));
        assert!(report.warnings.iter().any(|w| w.contains("草稿")));
        cleanup(&root);
        cleanup(out.parent().expect("parent"));
    }

    #[test]
    fn missing_image_counted_separately() {
        let root = temp_root("site7");
        write(&root, "n.md", "![[nope.png]]\n");
        let out = temp_root("site7-out").join("site");
        let report = export_vault_site(&root, &out, &SiteOptions::default()).expect("export");
        assert_eq!(report.missing_images, 1);
        assert_eq!(report.unresolved_links, 0);
        cleanup(&root);
        cleanup(out.parent().expect("parent"));
    }

    #[test]
    fn search_index_is_valid_json_with_all_pages() {
        let root = sample_vault("site8");
        let out = temp_root("site8-out").join("site");
        let report = export_vault_site(&root, &out, &SiteOptions::default()).expect("export");
        let text = fs::read_to_string(out.join("search-index.json")).expect("index");
        let value: serde_json::Value = serde_json::from_str(&text).expect("valid json");
        let arr = value.as_array().expect("array");
        assert_eq!(arr.len(), report.search_entries);
        assert_eq!(arr.len(), 3);
        let titles: Vec<&str> = arr.iter().filter_map(|e| e["title"].as_str()).collect();
        assert!(titles.contains(&"甲"));
        assert!(titles.contains(&"乙"));
        assert!(titles.contains(&"丙"));
        let text_of_a = arr
            .iter()
            .find(|e| e["href"] == "a.html")
            .and_then(|e| e["text"].as_str())
            .expect("a text");
        assert!(text_of_a.contains("提示"));
        assert!(!text_of_a.contains("秘密"));
        cleanup(&root);
        cleanup(out.parent().expect("parent"));
    }

    #[test]
    fn folder_filter_limits_pages() {
        let root = sample_vault("site9");
        let out = temp_root("site9-out").join("site");
        let opts = SiteOptions {
            folder: Some("sub".into()),
            ..SiteOptions::default()
        };
        let report = export_vault_site(&root, &out, &opts).expect("export");
        assert_eq!(report.pages, 1);
        assert!(out.join("sub/c.html").is_file());
        assert!(!out.join("a.html").exists());
        cleanup(&root);
        cleanup(out.parent().expect("parent"));
    }

    #[test]
    fn no_temp_files_left_behind() {
        let root = sample_vault("site10");
        let out = temp_root("site10-out").join("site");
        export_vault_site(&root, &out, &SiteOptions::default()).expect("export");
        export_vault_site(&root, &out, &SiteOptions::default()).expect("second export");
        let mut stack = vec![out.clone()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).expect("read") {
                let p = entry.expect("entry").path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    assert!(!p.to_string_lossy().ends_with(".jiyue-tmp"), "{p:?}");
                }
            }
        }
        cleanup(&root);
        cleanup(out.parent().expect("parent"));
    }

    #[test]
    fn output_inside_vault_is_not_walked() {
        let root = sample_vault("site11");
        let out = root.join("_site");
        export_vault_site(&root, &out, &SiteOptions::default()).expect("first");
        let report = export_vault_site(&root, &out, &SiteOptions::default()).expect("second");
        assert_eq!(report.pages, 3);
        assert!(!out.join("_site").exists());
        cleanup(&root);
    }

    #[test]
    fn titles_fall_back_to_heading_then_stem() {
        let root = temp_root("site12");
        write(&root, "无标题.md", "正文没有标题\n");
        write(&root, "h.md", "# 来自标题\n");
        let out = temp_root("site12-out").join("site");
        export_vault_site(&root, &out, &SiteOptions::default()).expect("export");
        let index = fs::read_to_string(out.join("index.html")).expect("index");
        assert!(index.contains(">无标题</a>"));
        assert!(index.contains(">来自标题</a>"));
        cleanup(&root);
        cleanup(out.parent().expect("parent"));
    }

    #[test]
    fn index_note_does_not_replace_site_index() {
        let root = temp_root("site13");
        write(&root, "index.md", "# 首页笔记\n");
        let out = temp_root("site13-out").join("site");
        export_vault_site(&root, &out, &SiteOptions::default()).expect("export");
        assert!(out.join("index-note.html").is_file());
        let index = fs::read_to_string(out.join("index.html")).expect("index");
        assert!(index.contains("<h1>笔记</h1>"));
        cleanup(&root);
        cleanup(out.parent().expect("parent"));
    }

    #[test]
    fn spaces_in_names_are_encoded_in_hrefs() {
        let root = temp_root("site14");
        write(&root, "one.md", "[[two three]]\n");
        write(&root, "two three.md", "# 二\n");
        let out = temp_root("site14-out").join("site");
        let report = export_vault_site(&root, &out, &SiteOptions::default()).expect("export");
        assert_eq!(report.unresolved_links, 0, "{:?}", report.warnings);
        let one = fs::read_to_string(out.join("one.html")).expect("one");
        assert!(one.contains("href=\"two%20three.html\""), "{one}");
        assert!(out.join("two three.html").is_file());
        cleanup(&root);
        cleanup(out.parent().expect("parent"));
    }

    #[test]
    fn missing_vault_root_is_an_error_not_a_panic() {
        let root = temp_root("site15").join("does-not-exist");
        let out = temp_root("site15-out").join("site");
        assert!(export_vault_site(&root, &out, &SiteOptions::default()).is_err());
    }

    #[test]
    fn helper_functions() {
        assert_eq!(normalize("a/./b/../c"), "a/c");
        assert_eq!(html_path("x/Note.MD"), "x/Note.html");
        assert_eq!(html_path("index.md"), "index-note.html");
        assert_eq!(stem("a/b.md"), "b");
        assert_eq!(stem("a/b.txt"), "b.txt");
        assert_eq!(root_prefix("a/b"), "../../");
        assert_eq!(relative_href("", "x.html"), "x.html");
        assert_eq!(encode_path("a b#c.png"), "a%20b%23c.png");
        assert_eq!(truncate_chars("一二三", 2), "一二");
        assert_eq!(truncate_chars("ab", 9), "ab");
    }

    #[test]
    fn asset_resolution_prefers_same_folder_and_relative_paths() {
        let files = vec!["img/a.png".to_string(), "sub/a.png".to_string()];
        assert_eq!(
            asset_rel(&files, "a.png", "sub/n.md").as_deref(),
            Some("sub/a.png")
        );
        assert_eq!(
            asset_rel(&files, "a.png", "n.md").as_deref(),
            Some("img/a.png")
        );
        assert_eq!(
            asset_rel(&files, "../img/a.png", "sub/n.md").as_deref(),
            Some("img/a.png")
        );
        assert_eq!(
            asset_rel(&files, "A.PNG", "n.md").as_deref(),
            Some("img/a.png")
        );
        assert_eq!(asset_rel(&files, "missing.png", "n.md"), None);
        assert_eq!(asset_rel(&files, "", "n.md"), None);
    }
}
