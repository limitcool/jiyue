//! Vault data for the live view's image and note widgets (see `embed.rs`).
//!
//! The editor has no access to the app, so the app shares a [`LiveHandle`] with every editor. It
//! holds the vault folder, the file index (for `![[name]]` resolution and image sizes), a snapshot
//! of the note texts (for note excerpts) and the decoded images. [`sync`] refreshes the snapshot
//! when the index or a note changes; it bumps `data_gen`, which makes the editor rebuild the
//! widgets that read it. Images are decoded on the background executor ([`ensure_images`]) and
//! the window is refreshed when one is ready; a widget never waits for a decode.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::SystemTime;

use gpui::{App, RenderImage};
use image::Frame;

use super::diagram::{self, DiagramKey};
use super::embed::{self, DECODE_MAX_SIDE, IMAGE_MAX_BYTES};
use crate::RustdianApp;
use crate::ui::attach_ui::resolve::VaultFiles;
use jiyue_diagram::{DiagramError, DiagramTheme};

/// BGRA pixels, width, height, CSS size and baseline distance of a rendered formula.
type MathPixels = (Vec<u8>, u32, u32, (f32, f32), f32);
/// BGRA pixels, width, height and CSS size of a rendered diagram.
type DiagramPixels = (Vec<u8>, u32, u32, (f32, f32));

/// The context shared by the app and its editors.
pub type LiveHandle = Rc<RefCell<LiveContext>>;

/// A note as the widgets see it: its path, title and whole text.
pub struct NoteSnap {
    pub path: String,
    pub title: String,
    pub body: Rc<str>,
}

/// Where an image stands: decoding, decoded (ready to paint) or unreadable.
pub enum ImageState {
    Loading,
    Ready(Arc<RenderImage>),
    Failed,
}

/// One image of the cache. `mtime` is the file's modification time when the decode started; a
/// result that comes back for an older `mtime` is dropped.
pub struct ImageSlot {
    pub mtime: Option<SystemTime>,
    pub natural: Option<(u32, u32)>,
    pub state: ImageState,
}

/// An image that a widget needs decoded: its vault path, absolute path and modification time.
pub struct Pending {
    pub rel: String,
    pub abs: PathBuf,
    pub mtime: Option<SystemTime>,
}

/// A formula's cache key: its LaTeX, its colour (`#RRGGBB`), its font size in tenths of a CSS
/// pixel, and whether it is a block formula (`true`) or an inline one (`false`).
pub type MathKey = (String, String, u32, bool);

/// Where a formula stands: rendering, rendered (its raster, CSS size and the baseline distance
/// from its top) or refused (the message).
#[derive(Clone)]
pub enum MathState {
    Loading,
    Ready {
        image: Arc<RenderImage>,
        css: (f32, f32),
        baseline: f32,
    },
    Failed(String),
}

/// Where a Mermaid diagram stands: rendering, rendered (its raster and drawn CSS size) or refused.
#[derive(Clone)]
pub enum DiagramState {
    Loading,
    Ready {
        image: Arc<RenderImage>,
        css: (f32, f32),
    },
    Failed(DiagramError),
}

/// A diagram waiting for its render: the cache key and the theme to draw it with.
pub struct DiagramJob {
    pub key: DiagramKey,
    pub theme: DiagramTheme,
}

/// The vault data the widgets read. See the module docs.
#[derive(Default)]
pub struct LiveContext {
    pub root: PathBuf,
    pub files: VaultFiles,
    /// The attachment folder of the settings, when it is fixed (used to resolve bare names).
    pub attach_folder: Option<String>,
    notes: Vec<NoteSnap>,
    images: HashMap<String, ImageSlot>,
    math: HashMap<MathKey, MathState>,
    math_queue: Vec<MathKey>,
    diagrams: HashMap<DiagramKey, DiagramState>,
    diagram_queue: Vec<DiagramJob>,
    /// Bumped whenever something a widget reads changes. Widgets built from older data are rebuilt.
    pub data_gen: u64,
    files_stamp: (usize, usize),
}

impl LiveContext {
    fn bump(&mut self) {
        self.data_gen = self.data_gen.wrapping_add(1);
    }

    /// The vault-relative file a link target names, as seen from the note `note_path`.
    pub fn resolve(&self, note_path: &str, target: &str) -> Option<String> {
        self.files
            .resolve(note_path, target, self.attach_folder.as_deref())
    }

    /// Absolute path of a vault-relative file.
    pub fn abs(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    /// Pixel size of an image: from the file index, else from its decode.
    pub fn natural_size(&self, rel: &str) -> Option<(u32, u32)> {
        self.files
            .image_size(rel)
            .or_else(|| self.images.get(rel).and_then(|s| s.natural))
    }

    /// The cache entry of an image, if one was started.
    pub fn slot(&self, rel: &str) -> Option<&ImageSlot> {
        self.images.get(rel)
    }

    /// The state of a formula, if it was asked for.
    pub fn math_state(&self, key: &MathKey) -> Option<MathState> {
        self.math.get(key).cloned()
    }

    /// Asks for a formula: it is marked as loading and queued for [`ensure_math`].
    pub fn queue_math(&mut self, key: MathKey) {
        if !self.math.contains_key(&key) {
            self.math.insert(key.clone(), MathState::Loading);
            self.math_queue.push(key);
            self.bump();
        }
    }

    /// The state of a Mermaid diagram, if it was asked for.
    pub fn diagram_state(&self, key: &DiagramKey) -> Option<DiagramState> {
        self.diagrams.get(key).cloned()
    }

    /// Asks for a diagram render: it is marked as loading and queued for [`ensure_diagrams`]. A
    /// key that is already rendering or rendered is not queued again. Old finished entries are
    /// dropped beyond [`diagram::CACHE_MAX`].
    pub fn queue_diagram(&mut self, key: DiagramKey, theme: DiagramTheme) {
        if self.diagrams.contains_key(&key) {
            return;
        }
        if self.diagrams.len() >= diagram::CACHE_MAX {
            let victim = self
                .diagrams
                .iter()
                .find(|(_, s)| !matches!(s, DiagramState::Loading))
                .map(|(k, _)| k.clone());
            if let Some(k) = victim {
                self.diagrams.remove(&k);
            }
        }
        self.diagrams.insert(key.clone(), DiagramState::Loading);
        self.diagram_queue.push(DiagramJob { key, theme });
        self.bump();
    }

    /// The note with this title (case-insensitive, the same rule as the vault's title lookup).
    pub fn note_by_title(&self, title: &str) -> Option<&NoteSnap> {
        self.notes
            .iter()
            .find(|n| n.title.eq_ignore_ascii_case(title))
    }
}

/// Refreshes the handle from the app: the root, the file index and the note texts. Cheap when
/// nothing changed (it compares the sizes of the index and of the note list). Call it once per
/// frame before the editors paint.
pub fn sync(handle: &LiveHandle, app: &RustdianApp) {
    let mut c = handle.borrow_mut();
    if c.root != app.vault.root_dir {
        c.root = app.vault.root_dir.clone();
        c.images.clear();
        c.bump();
    }
    let files = &app.ext.attach.files;
    let stamp = (files.files.len(), files.dims.len());
    let folder = app.ext.attach.fixed_folder();
    if stamp != c.files_stamp || folder != c.attach_folder {
        c.files = files.clone();
        c.files_stamp = stamp;
        c.attach_folder = folder;
        // A changed index can name other files: cached images of the old index are dropped.
        c.images.clear();
        c.bump();
    }
    // The snapshots are compared with the notes by content (a memory compare, no copies): an edit
    // that keeps the length of a note (a replaced letter) is seen too.
    let same = notes_match(
        &c.notes,
        app.vault
            .notes
            .iter()
            .map(|n| (n.path.as_str(), n.title.as_str(), n.content())),
    );
    if !same {
        c.notes = app
            .vault
            .notes
            .iter()
            .map(|n| NoteSnap {
                path: n.path.clone(),
                title: n.title.clone(),
                body: Rc::from(n.content()),
            })
            .collect();
        c.bump();
    }
}

/// Whether the snapshots are the notes `(path, title, body)` in the same order with the same text.
pub fn notes_match<'a>(
    snaps: &[NoteSnap],
    notes: impl ExactSizeIterator<Item = (&'a str, &'a str, &'a str)>,
) -> bool {
    snaps.len() == notes.len()
        && snaps
            .iter()
            .zip(notes)
            .all(|(s, (path, title, body))| s.path == path && s.title == title && &*s.body == body)
}

/// Modification time of a file, `None` when it cannot be read.
pub fn modified(abs: &Path) -> Option<SystemTime> {
    std::fs::metadata(abs).and_then(|m| m.modified()).ok()
}

/// Starts decoding the images in `pending` on the background executor. Each slot is set to
/// `Loading` at once, so a widget built next frame waits for it instead of asking again. When a
/// decode ends, its slot is filled (unless a newer request replaced it) and the windows refresh.
pub fn ensure_images(handle: &LiveHandle, pending: Vec<Pending>, cx: &mut App) {
    for p in pending {
        {
            let mut c = handle.borrow_mut();
            let natural = c.natural_size(&p.rel);
            c.images.insert(
                p.rel.clone(),
                ImageSlot {
                    mtime: p.mtime,
                    natural,
                    state: ImageState::Loading,
                },
            );
            c.bump();
        }
        let h = handle.clone();
        let abs = p.abs;
        let rel = p.rel;
        let mtime = p.mtime;
        cx.spawn(async move |cx| {
            let decoded = cx
                .background_executor()
                .spawn(async move { decode(&abs) })
                .await;
            let filled = {
                let mut c = h.borrow_mut();
                match c.images.get_mut(&rel) {
                    Some(slot)
                        if slot.mtime == mtime && matches!(slot.state, ImageState::Loading) =>
                    {
                        match decoded {
                            Ok((image, natural)) => {
                                slot.natural = Some(natural);
                                slot.state = ImageState::Ready(image);
                            }
                            Err(_) => slot.state = ImageState::Failed,
                        }
                        true
                    }
                    _ => false,
                }
            };
            if filled {
                h.borrow_mut().bump();
            }
            let _ = cx.refresh();
        })
        .detach();
    }
}

/// Renders the queued block formulas on the background executor (see [`MathState`]). A formula
/// that is already rendered or rendering is not queued again.
pub fn ensure_math(handle: &LiveHandle, cx: &mut App) {
    let queue: Vec<MathKey> = std::mem::take(&mut handle.borrow_mut().math_queue);
    for key in queue {
        let latex = key.0.clone();
        let colour = key.1.clone();
        let px = key.2 as f32 / 10.0;
        let display = key.3;
        let h = handle.clone();
        cx.spawn(async move |cx| {
            let rendered = cx
                .background_executor()
                .spawn(async move { render_math_pixels(&latex, &colour, px, display) })
                .await;
            let state = match rendered {
                Ok((data, w, hgt, css, baseline)) => match image::RgbaImage::from_raw(w, hgt, data)
                {
                    Some(buf) => MathState::Ready {
                        image: Arc::new(RenderImage::new(vec![Frame::new(buf)])),
                        css,
                        baseline,
                    },
                    None => MathState::Failed("raster size".to_string()),
                },
                Err(err) => MathState::Failed(embed::math_error_reason(&err).to_string()),
            };
            {
                let mut c = h.borrow_mut();
                c.math.insert(key, state);
                c.bump();
            }
            let _ = cx.refresh();
        })
        .detach();
    }
}

/// Renders one formula to BGRA pixels (gpui paints BGRA). `display` selects display style (block)
/// or text style (inline). Returns the pixels, their size, the CSS size of the formula and its
/// baseline distance from the top.
fn render_math_pixels(
    latex: &str,
    colour: &str,
    px: f32,
    display: bool,
) -> Result<MathPixels, jiyue_math::MathError> {
    let theme = jiyue_math::MathTheme {
        color: colour.to_string(),
        font_size: px,
    };
    let rendered = jiyue_math::render_math(latex, display, &theme)?;
    let raster = jiyue_math::rasterize(&rendered, embed::MATH_RENDER_SCALE)?;
    let mut data = raster.data;
    for pixel in data.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Ok((
        data,
        raster.width,
        raster.height,
        (rendered.width, rendered.height),
        rendered.baseline,
    ))
}

/// Renders the queued Mermaid diagrams on the background executor (see [`DiagramState`]). The
/// window is refreshed once per diagram, when its state changes.
pub fn ensure_diagrams(handle: &LiveHandle, cx: &mut App) {
    let queue: Vec<DiagramJob> = std::mem::take(&mut handle.borrow_mut().diagram_queue);
    for job in queue {
        let h = handle.clone();
        cx.spawn(async move |cx| {
            let key = job.key.clone();
            let column = key.width as f32;
            let theme = job.theme;
            let source = key.source.clone();
            let rendered = cx
                .background_executor()
                .spawn(async move { render_diagram_pixels(&source, &theme, column) })
                .await;
            let state = match rendered {
                Ok((data, w, hgt, css)) => match image::RgbaImage::from_raw(w, hgt, data) {
                    Some(buf) => DiagramState::Ready {
                        image: Arc::new(RenderImage::new(vec![Frame::new(buf)])),
                        css,
                    },
                    None => DiagramState::Failed(DiagramError::Render("raster size".into())),
                },
                Err(err) => DiagramState::Failed(err),
            };
            {
                let mut c = h.borrow_mut();
                c.diagrams.insert(key, state);
                c.bump();
            }
            let _ = cx.refresh();
        })
        .detach();
    }
}

/// Renders one Mermaid source to BGRA pixels drawn at most `column` px wide (and at most
/// [`diagram::MAX_CSS_H`] tall), rasterised at [`diagram::RASTER_DPI`]. Returns the pixels, their
/// size and the drawn CSS size.
fn render_diagram_pixels(
    source: &str,
    theme: &DiagramTheme,
    column: f32,
) -> Result<DiagramPixels, DiagramError> {
    let rendered = jiyue_diagram::render_mermaid(source, theme)?;
    let natural = (rendered.width, rendered.height);
    let Some(drawn) = diagram::fit(natural, column) else {
        return Err(DiagramError::Render("the diagram has no size".into()));
    };
    let scale = diagram::raster_scale(natural.0, drawn.0);
    let raster = jiyue_diagram::rasterize(&rendered, scale)?;
    let mut data = raster.data;
    for pixel in data.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Ok((data, raster.width, raster.height, drawn))
}

/// Reads and decodes one image: the size cap first, then the decode, then a downscale to the GPU
/// size when it is larger. The pixels are kept as BGRA, which is what gpui paints. Returns the
/// image and its natural size.
pub fn decode(abs: &Path) -> Result<(Arc<RenderImage>, (u32, u32)), String> {
    let meta = std::fs::metadata(abs).map_err(|e| e.to_string())?;
    if meta.len() > IMAGE_MAX_BYTES {
        return Err("image is larger than the size cap".to_string());
    }
    let img = image::ImageReader::open(abs)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?;
    let natural = (img.width(), img.height());
    let target = embed::decode_size(natural, DECODE_MAX_SIDE);
    let img = if target != natural {
        img.resize_exact(target.0, target.1, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let mut rgba = img.into_rgba8();
    for pixel in rgba.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Ok((Arc::new(RenderImage::new(vec![Frame::new(rgba)])), natural))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(path: &str, title: &str, body: &str) -> NoteSnap {
        NoteSnap {
            path: path.to_string(),
            title: title.to_string(),
            body: Rc::from(body),
        }
    }

    #[test]
    fn same_length_edit_is_seen() {
        let snaps = vec![snap("a.md", "A", "abc"), snap("b.md", "B", "xyz")];
        assert!(notes_match(
            &snaps,
            [("a.md", "A", "abc"), ("b.md", "B", "xyz")].into_iter()
        ));
        // One letter replaced, the byte length unchanged.
        assert!(!notes_match(
            &snaps,
            [("a.md", "A", "abd"), ("b.md", "B", "xyz")].into_iter()
        ));
    }

    #[test]
    fn added_or_renamed_notes_are_seen() {
        let snaps = vec![snap("a.md", "A", "abc")];
        assert!(!notes_match(
            &snaps,
            [("a.md", "A", "abc"), ("c.md", "C", "")].into_iter()
        ));
        assert!(!notes_match(&snaps, [("c.md", "A", "abc")].into_iter()));
        assert!(!notes_match(&snaps, std::iter::empty()));
    }
}
