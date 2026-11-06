//! Native Mermaid rendering for Jiyue.
//!
//! Pipeline: Mermaid source -> [`render_mermaid`] -> self-contained SVG ->
//! [`rasterize`] -> RGBA pixels for the GPUI preview. No browser and no JavaScript.
//!
//! * The SVG is produced by the pure-Rust `mermaid-rs-renderer` crate, then
//!   rewritten by a sanitiser that removes scripts, foreign objects, external
//!   references and event handlers.
//! * Rendering runs on a worker thread with a 3 s default timeout, and source
//!   size, line and node-count limits are checked up front.
//! * [`DiagramCache`] is a byte-bounded LRU keyed by `blake3(source, theme)`.
//!
//! ```no_run
//! use jiyue_diagram::{DiagramTheme, DiagramCache, rasterize};
//!
//! let cache = DiagramCache::new(8 * 1024 * 1024);
//! let theme = DiagramTheme::dark();
//! match cache.get_or_render("flowchart LR\n A[开始] --> B[结束]", &theme) {
//!     Ok(diagram) => {
//!         let pixels = rasterize(&diagram, 2.0);
//!         let _ = pixels;
//!     }
//!     Err(e) => eprintln!("{e}"),
//! }
//! ```

#![forbid(unsafe_code)]

mod cache;
mod color;
mod error;
mod kind;
mod raster;
mod render;
mod sanitize;
mod theme;

pub use cache::DiagramCache;
pub use error::DiagramError;
pub use kind::{DiagramKind, detect_kind};
pub use raster::{MAX_RASTER_PIXELS, Rgba8Image, rasterize};
pub use render::{
    DEFAULT_TIMEOUT, MAX_LINES, MAX_NODES, MAX_SOURCE_BYTES, RenderedDiagram, render_mermaid,
    render_mermaid_with_timeout,
};
pub use theme::DiagramTheme;
