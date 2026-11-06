//! Embedded stylesheet: light and dark palettes, reading layout, CJK-friendly typography,
//! and a print stylesheet for printing to PDF from a browser.

pub(super) const CSS: &str = r##"
:root {
  --bg: #fbfaf7;
  --fg: #1f2328;
  --muted: #6e7781;
  --accent: #0a6cbf;
  --border: #d8d4cc;
  --code-bg: #f3f1ec;
  --quote-bar: #c9c3b6;
  --mark-bg: #fff1a8;
  --tag-bg: #e6f0fa;
  --tag-fg: #135a9a;
  --callout-bg: #f4f7fb;
  --callout-bar: #3b7dd8;
  --warn-bar: #d99a1e;
  --danger-bar: #d1442f;
  --ok-bar: #2e9e5b;
  --kw: #8250df;
  --str: #116329;
  --com: #6e7781;
  --num: #0550ae;
  --fn: #953800;
}
@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) {
    --bg: #161819;
    --fg: #e6e6e3;
    --muted: #8b949e;
    --accent: #6cb6ff;
    --border: #3a3e44;
    --code-bg: #20242a;
    --quote-bar: #4a4f57;
    --mark-bg: #5b4d12;
    --tag-bg: #1d2f44;
    --tag-fg: #9cc9ff;
    --callout-bg: #1b2430;
    --callout-bar: #5b9bf0;
    --warn-bar: #e0a93a;
    --danger-bar: #f0644c;
    --ok-bar: #4fbf7b;
    --kw: #c792ea;
    --str: #7ee787;
    --com: #8b949e;
    --num: #79c0ff;
    --fn: #ffa657;
  }
}
:root[data-theme="dark"] {
  --bg: #161819;
  --fg: #e6e6e3;
  --muted: #8b949e;
  --accent: #6cb6ff;
  --border: #3a3e44;
  --code-bg: #20242a;
  --quote-bar: #4a4f57;
  --mark-bg: #5b4d12;
  --tag-bg: #1d2f44;
  --tag-fg: #9cc9ff;
  --callout-bg: #1b2430;
  --callout-bar: #5b9bf0;
  --warn-bar: #e0a93a;
  --danger-bar: #f0644c;
  --ok-bar: #4fbf7b;
  --kw: #c792ea;
  --str: #7ee787;
  --com: #8b949e;
  --num: #79c0ff;
  --fn: #ffa657;
}
* { box-sizing: border-box; }
html { -webkit-text-size-adjust: 100%; }
body {
  margin: 0;
  background: var(--bg);
  color: var(--fg);
  font-family: system-ui, -apple-system, "Segoe UI", "PingFang SC", "Hiragino Sans GB",
    "Microsoft YaHei", "Noto Sans CJK SC", "Source Han Sans SC", sans-serif;
  font-size: 16px;
  line-height: 1.75;
  word-break: normal;
  overflow-wrap: anywhere;
}
.page {
  max-width: 46rem;
  margin: 0 auto;
  padding: 2.5rem 1rem 4rem;
}
h1, h2, h3, h4, h5, h6 { line-height: 1.35; margin: 1.6em 0 0.6em; font-weight: 650; }
h1 { font-size: 2em; border-bottom: 1px solid var(--border); padding-bottom: 0.3em; }
h2 { font-size: 1.5em; border-bottom: 1px solid var(--border); padding-bottom: 0.2em; }
h3 { font-size: 1.25em; }
p, ul, ol, table, pre, blockquote, .callout, .jiyue-embed { margin: 0.9em 0; }
a { color: var(--accent); text-decoration: none; }
a:hover { text-decoration: underline; }
ul, ol { padding-left: 1.6em; }
li + li { margin-top: 0.2em; }
li > input[type="checkbox"] { margin-right: 0.4em; }
blockquote {
  margin-left: 0;
  padding: 0.1em 1em;
  color: var(--muted);
  border-left: 3px solid var(--quote-bar);
}
hr { border: 0; border-top: 1px solid var(--border); margin: 2em 0; }
img { max-width: 100%; height: auto; border-radius: 4px; }
code, kbd, pre { font-family: ui-monospace, "Cascadia Code", "JetBrains Mono", Consolas, "Sarasa Mono SC", monospace; }
:not(pre) > code {
  background: var(--code-bg);
  padding: 0.12em 0.36em;
  border-radius: 4px;
  font-size: 0.92em;
}
pre.jiyue-code {
  background: var(--code-bg);
  padding: 0.9em 1em;
  border-radius: 6px;
  overflow-x: auto;
  font-size: 0.88em;
  line-height: 1.55;
  white-space: pre;
  overflow-wrap: normal;
}
pre.jiyue-code code { background: none; padding: 0; }
.tok-kw { color: var(--kw); }
.tok-str { color: var(--str); }
.tok-com { color: var(--com); font-style: italic; }
.tok-num { color: var(--num); }
.tok-fn { color: var(--fn); }
table { border-collapse: collapse; width: 100%; display: block; overflow-x: auto; }
th, td { border: 1px solid var(--border); padding: 0.4em 0.7em; text-align: left; vertical-align: top; }
th { background: var(--code-bg); }
del { color: var(--muted); }
mark, .jiyue-mark { background: var(--mark-bg); color: inherit; padding: 0 0.15em; border-radius: 3px; }
.jiyue-tag {
  background: var(--tag-bg);
  color: var(--tag-fg);
  padding: 0.05em 0.45em;
  border-radius: 999px;
  font-size: 0.9em;
  white-space: nowrap;
}
.jiyue-link { border-bottom: 1px dotted var(--accent); }
.jiyue-missing { color: var(--muted); border-bottom: 1px dashed var(--muted); cursor: help; }
.jiyue-embed {
  border-left: 3px solid var(--quote-bar);
  padding: 0.2em 1em;
  opacity: 0.95;
}
.jiyue-embed h1, .jiyue-embed h2 { font-size: 1.2em; border: 0; }
.jiyue-block-id { display: inline-block; width: 0; }
.jiyue-math { font-family: "Cambria Math", "STIX Two Math", serif; }
.jiyue-math-block { text-align: center; margin: 1em 0; overflow-x: auto; }
.jiyue-mermaid { text-align: center; margin: 1em 0; overflow-x: auto; }
.jiyue-mermaid svg { max-width: 100%; height: auto; }
.callout {
  background: var(--callout-bg);
  border-left: 4px solid var(--callout-bar);
  border-radius: 6px;
  padding: 0.7em 1em;
}
.callout-title { font-weight: 650; margin-bottom: 0.3em; }
.callout-body > :first-child { margin-top: 0; }
.callout-body > :last-child { margin-bottom: 0; }
.callout-warning { border-left-color: var(--warn-bar); }
.callout-danger { border-left-color: var(--danger-bar); }
.callout-success, .callout-tip { border-left-color: var(--ok-bar); }
.callout-quote { border-left-color: var(--quote-bar); }
.callout-quote .callout-title { color: var(--muted); }
.toc {
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 0.6em 1em;
  margin: 1em 0 2em;
  font-size: 0.92em;
}
.toc-title { margin: 0 0 0.3em; font-weight: 650; }
.toc ul { list-style: none; padding-left: 0; margin: 0; }
.toc li { margin: 0.1em 0; }
.toc .toc-l2 { padding-left: 1em; }
.toc .toc-l3 { padding-left: 2em; }
.toc .toc-l4, .toc .toc-l5, .toc .toc-l6 { padding-left: 3em; }
.properties { margin: 0 0 2em; font-size: 0.92em; }
.properties table { display: table; }
.properties th { width: 9em; color: var(--muted); font-weight: 500; background: none; }
.properties td, .properties th { border-color: var(--border); }
.footnote-definition { font-size: 0.9em; color: var(--muted); }
.footnote-reference a { font-size: 0.8em; }
.footnotes { margin-top: 2.5em; padding-top: 0.6em; border-top: 1px solid var(--border); font-size: 0.9em; color: var(--muted); }
.footnotes li { margin: 0.3em 0; }
.footnote-backref { text-decoration: none; margin-left: 0.3em; }

/* Site chrome (used by jiyue-export site pages). */
.site-top {
  display: flex;
  flex-wrap: wrap;
  gap: 0.6em 1em;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 1.5em;
  font-size: 0.92em;
}
.site-top .site-name { font-weight: 650; color: var(--fg); }
.search { flex: 1 1 14em; max-width: 22em; position: relative; }
.search input {
  width: 100%;
  padding: 0.4em 0.7em;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--bg);
  color: var(--fg);
  font: inherit;
}
.search-results {
  list-style: none;
  padding: 0;
  margin: 0.3em 0 0;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--bg);
  max-height: 60vh;
  overflow-y: auto;
  position: absolute;
  left: 0;
  right: 0;
  z-index: 10;
}
.search-results:empty { display: none; }
.search-results li { padding: 0.45em 0.7em; }
.search-results li + li { border-top: 1px solid var(--border); margin: 0; }
.search-results .snippet { color: var(--muted); font-size: 0.88em; display: block; }
.backlinks { margin-top: 3em; border-top: 1px solid var(--border); padding-top: 1em; font-size: 0.95em; }
.backlinks h2 { font-size: 1.05em; border: 0; margin: 0 0 0.4em; color: var(--muted); }
.folder-list h2 { font-size: 1.1em; border: 0; color: var(--muted); margin-top: 1.6em; }
.muted { color: var(--muted); }

@media (max-width: 600px) {
  .page { padding: 1.4rem 0.9rem 3rem; }
  h1 { font-size: 1.6em; }
}

@media print {
  body { background: #fff; color: #000; font-size: 11pt; line-height: 1.5; }
  .page { max-width: none; padding: 0; }
  .site-top, .search, .search-results, .toc, .backlinks { display: none !important; }
  a { color: #000; text-decoration: none; }
  a[href^="http"]::after { content: " (" attr(href) ")"; color: #555; font-size: 0.85em; }
  pre.jiyue-code, blockquote, .callout, table, img, .jiyue-mermaid, .jiyue-math-block {
    break-inside: avoid;
  }
  pre.jiyue-code { white-space: pre-wrap; border: 1px solid #ccc; background: #f6f6f6; }
  h1, h2, h3 { break-after: avoid; }
  .callout { background: #fff; border: 1px solid #bbb; border-left-width: 4px; }
  .tok-kw, .tok-str, .tok-com, .tok-num, .tok-fn { color: #000 !important; }
  .tok-com { font-style: italic; }
  .jiyue-tag { background: none; color: #000; border: 1px solid #999; }
  mark { background: none; text-decoration: underline; }
}
"##;
