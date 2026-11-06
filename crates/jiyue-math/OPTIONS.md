# jiyue-math: LaTeX math rendering options (spike W3-X)

Goal: render `$x^2$` (inline) and `$$...$$` (display) from the GPUI app to a safe,
self-contained SVG with no browser, with a cache, a timeout and size limits.

## Recommendation

Use **latex-rust 3.0.1** (pure Rust, MIT OR Apache-2.0, STIX Two Math embedded) as the
layout and SVG back end, wrapped by this crate (timeout, limits, cache, rasteriser,
`$` span finder). Of the options evaluated, it is the only one that turns LaTeX into
SVG with a small dependency and no runtime. Quality on the formulas tested is good
(see "Quality"). Render times are about 0.1 to 1.5 ms per formula in the probe.

Known gap: **CJK inside `\text{}`**. STIX Two Math has no CJK glyphs, so such formulas
return `MathError::Unsupported`. The app should show the raw source for those spans.
Fixing it needs a CJK font, typically several MB (not measured). That choice belongs
to the app owner.

## Comparison table

"Measured" means built and timed in this spike. "Metadata" means read from
crates.io, docs.rs or `cargo info`, and not built.

| # | Option | Version / licence | Maintenance (observed) | Measured cost | LaTeX coverage | CJK in `\text` | Windows | Verdict |
|---|---|---|---|---|---|---|---|---|
| a1 | **latex-rust** (pure Rust TeX layout; STIX font; SVG/PNG/egui output) | 3.0.1; MIT OR Apache-2.0; rust-version 1.99 | Released 3.0.1 on crates.io; package includes CHANGELOG and golden tests | **Measured:** cold release compile 59 s (latex-rust alone, SVG only, with skrifa and read-fonts). Binary 2.68 MB against 0.14 MB for an empty Rust binary (+2.5 MB, including the 0.8 MB STIX font). With the `png` feature: 1 m 46 s compile, 3.2 MB binary. Per formula: 0.1 to 1.5 ms (one 7.3 ms outlier) | 47 of 51 probe inputs rendered. The 4 failures are expected: 2 CJK, 1 malformed, 1 unknown command | No: `MissingGlyph` error | Yes (pure Rust, no build script) | **Chosen** |
| a2 | typst 0.15.1 with typst-as-lib 0.17.0 | typst Apache-2.0; typst-as-lib MIT | Very active | **Measured:** cold release compile 6 m 29 s for typst-as-lib and typst (compile only; no rendering code linked, so binary size unknown, expected large) | Typst math syntax, not LaTeX. LaTeX input needs a converter (mitex or tylax) | Not embedded by default; no CJK font shipped | Yes | Strong typography, but heavy and not LaTeX. Worth it only if the app adopts Typst syntax |
| a3 | mitex 0.2.4 (LaTeX to Typst converter) | Apache-2.0 | Last repo activity early 2025 (lower confidence) | Metadata only. Needs a Typst renderer on top (a2) | Good subset; converts, does not render | Depends on a2 | Yes | Only as an add-on to a2 |
| a4 | tylax 0.3.8 (LaTeX/Typst converter, uses mitex-parser) | Apache-2.0 | Newer than mitex | Metadata only | As a3 | As a3 | Yes | Same as a3 |
| b1 | pulldown-latex 0.8.0 (LaTeX to MathML pull parser) | MIT | Active. README: about 95% of KaTeX coverage; "not recommended for large-scale production use yet" | Metadata only | Good parser; output is MathML | n/a (no layout) | Yes | Needs a MathML layout engine in front of SVG, which is a second project. Not chosen |
| b2 | math-core 0.9.0 (LaTeX to MathML Core) | MIT | Active | Metadata only | Good | n/a | Yes | Same as b1 |
| b3 | formulary 0.3.0 (MathML Core layout from OpenType MATH metrics, SVG debug emitter) | MIT OR Apache-2.0; rust-version 1.87 | Small, young | Metadata only | Needs MathML input (from b1 or b2) | Depends on font | Yes | Good candidate for a future MathML path. Too many moving parts for this spike |
| c1 | rquickjs 0.14.0 running KaTeX | rquickjs MIT; KaTeX MIT | Active | Metadata only. Expected cost (estimate, not measured): a QuickJS runtime plus the KaTeX JS bundle and a JS parse per render | Excellent (KaTeX) | Depends on fonts | Yes | **Rejected.** KaTeX emits HTML and CSS, not SVG, so a second conversion is needed. Adds a JS engine to the app. Slower per render |
| c2 | katex-rs 0.3.0 | MIT | Young; wasm-oriented | Metadata only | Unknown | Unknown | Unknown | Not evaluated further (maturity unclear) |
| d | Browser or headless Chromium with MathJax or KaTeX | n/a | n/a | n/a | Excellent | Yes | Yes | **Rejected by requirement** (no browser) |

## Measured details (latex-rust probe)

Probe: 51 inputs, `latex-rust` 3.0.1, `MathFont::stix_two_math`, `SvgOptions` at 16 pt,
release build on Windows 11, in a scratch project outside the workspace.

- Successes: 47. Failures: `\text{中文}` and `\text{hello 世界}` (`MissingGlyph`, CJK),
  `\frac{` (`Malformed`), `\unknowncmd` (`Unknown`).
- Timing: 0.1 to 1.5 ms for most inputs. `\sqrt{1+\sqrt{x}}` took 7.3 ms (likely a
  first-call cost).
- Output: SVG with glyphs as `<path>` and rules as `<rect>`, inside `<g>`. Checked in the
  latex-rust source: no `<text>`, no fonts needed at render time, no links, no scripts.

## Quality (checked by eye on rasterised PNGs at 2x, over white)

Folder: `<scratch>\math-shots\`
(16 PNGs; regenerate with `cargo run -p jiyue-math --example shots -- <dir>`).

- Good: fractions (including nested), radicals, sum, integral and limit placement,
  `pmatrix`, `bmatrix`, `cases`, `aligned`, `\left( \right)`, `\binom`, accents
  (`\hat \bar \vec \ddot`), Greek, relations.
- Acceptable: `\mathbb`, `\mathcal`, `\mathbf` use STIX alphabets. `\mathcal` is less
  distinct than a true calligraphic face at small sizes.
- Not rendered: CJK inside `\text{}` (honest error, see above).

## What this crate does NOT cover

- CJK, or any non-Latin script, inside `\text{}` (returns `Unsupported`). No font fallback.
- LaTeX macros (`\newcommand`), `\label`, `\ref`, `\tag`, and environments outside the
  matrix, cases, array, aligned and align families were not exercised.
- Theme: colour only. No weight or font-family choice. STIX Two Math only.
- Timeout: the worker thread cannot be killed. On timeout the caller gets `Timeout`
  at once and the worker finishes in the background. The 20 KB input cap bounds that cost.
- Markdown: indented (4-space) code blocks, raw HTML and link targets are not skipped by
  `find_math_spans`, so a `$` inside a URL could open math.
- `$2^n$` is not treated as math (inline `$` followed by a digit). This is a deliberate
  trade-off against currency text.
- Alignment in GPUI (placing the baseline inside a line box) was not verified in the
  app. Only the metrics `width`, `height` and `baseline` are provided, and they are tested.
- Only latex-rust was built as a back end. The rasteriser is resvg 0.47, which is
  already in the workspace lockfile.

## Dependencies of jiyue-math

| Dependency | Version | Why | Compile and size impact |
|---|---|---|---|
| latex-rust | =3.0.1 (MIT OR Apache-2.0) | TeX layout and SVG output | About 59 s cold, including skrifa, read-fonts and font-types. +2.5 MB in a binary |
| blake3 | 1.8.7 (already in workspace lock) | Cache keys | Negligible; already built by jiyue-md |
| serde | workspace (derive) | Serialise `MathTheme` and `RenderedMath` | Shared |
| resvg | 0.47.0 (optional; default feature `raster`; already in workspace lock) | SVG to RGBA | No new compile, shared with another workspace crate |
| image | workspace, png (dev-dependency only) | PNG writing in the `shots` example | Dev only |

No other runtime dependency. No `unsafe`. No build script.
