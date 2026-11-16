//! Windows clipboard media reading: PNG/GIF/JFIF/SVG, `CF_DIB` bitmaps (screenshots), and
//! copied local files (`CF_HDROP`).

#[cfg(any(windows, test))]
use std::io::Cursor;

#[cfg(any(windows, test))]
use image::codecs::bmp::BmpDecoder;
#[cfg(any(windows, test))]
use image::{DynamicImage, ImageFormat};

/// Converts a Windows `CF_DIB` payload (`BITMAPINFOHEADER` + pixels, without BITMAPFILEHEADER)
/// into PNG bytes. Repairs GDI 32bpp `BI_RGB` images whose alpha channel is all zero.
#[cfg(any(windows, test))]
pub fn dib_to_png(dib: &[u8]) -> Option<Vec<u8>> {
    let decoder = BmpDecoder::new_without_file_header(Cursor::new(dib)).ok()?;
    let img = DynamicImage::from_decoder(decoder).ok()?;
    let mut rgba = img.into_rgba8();
    if !rgba.is_empty() && rgba.pixels().all(|p| p[3] == 0) {
        for p in rgba.pixels_mut() {
            p[3] = 255;
        }
    }
    let mut out = Vec::new();
    DynamicImage::ImageRgba8(rgba)
        .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
        .ok()?;
    Some(out)
}

#[cfg(windows)]
mod win32 {
    use core::ffi::c_void;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;
    use std::sync::LazyLock;

    use super::super::import::Source;
    use super::dib_to_png;

    const CF_DIB: u32 = 8;
    const CF_HDROP: u32 = 15;
    const DRAGDROP_GET_FILES_COUNT: u32 = 0xFFFF_FFFF;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn OpenClipboard(hwnd_new_owner: *mut c_void) -> i32;
        fn CloseClipboard() -> i32;
        fn IsClipboardFormatAvailable(format: u32) -> i32;
        fn GetClipboardData(format: u32) -> *mut c_void;
        fn RegisterClipboardFormatW(lpsz_format: *const u16) -> u32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalSize(hmem: *mut c_void) -> usize;
        fn GlobalLock(hmem: *mut c_void) -> *mut c_void;
        fn GlobalUnlock(hmem: *mut c_void) -> i32;
    }

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn DragQueryFileW(hdrop: *mut c_void, i_file: u32, lpsz_file: *mut u16, cch: u32) -> u32;
    }

    static CLIPBOARD_PNG_FORMAT: LazyLock<u32> = LazyLock::new(|| register_format("PNG"));
    static CLIPBOARD_GIF_FORMAT: LazyLock<u32> = LazyLock::new(|| register_format("GIF"));
    static CLIPBOARD_JPG_FORMAT: LazyLock<u32> = LazyLock::new(|| register_format("JFIF"));
    static CLIPBOARD_SVG_FORMAT: LazyLock<u32> = LazyLock::new(|| register_format("image/svg+xml"));

    fn register_format(name: &str) -> u32 {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe { RegisterClipboardFormatW(wide.as_ptr()) }
    }

    fn format_available(format: u32) -> bool {
        format != 0 && unsafe { IsClipboardFormatAvailable(format) != 0 }
    }

    struct ClipboardGuard;

    impl ClipboardGuard {
        fn open() -> Option<Self> {
            (unsafe { OpenClipboard(core::ptr::null_mut()) } != 0).then_some(Self)
        }
    }

    impl Drop for ClipboardGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseClipboard();
            }
        }
    }

    fn with_locked_data<R>(format: u32, f: impl FnOnce(*const u8, usize) -> R) -> Option<R> {
        let handle = unsafe { GetClipboardData(format) };
        if handle.is_null() {
            return None;
        }
        let size = unsafe { GlobalSize(handle) };
        if size == 0 {
            return None;
        }
        let ptr = unsafe { GlobalLock(handle) };
        if ptr.is_null() {
            return None;
        }
        let out = f(ptr as *const u8, size);
        unsafe {
            let _ = GlobalUnlock(handle);
        }
        Some(out)
    }

    fn read_bytes(format: u32) -> Option<Vec<u8>> {
        with_locked_data(format, |ptr, size| {
            unsafe { std::slice::from_raw_parts(ptr, size) }.to_vec()
        })
    }

    fn read_drop_files() -> Vec<PathBuf> {
        with_locked_data(CF_HDROP, |ptr, _size| {
            let hdrop = ptr.cast_mut().cast();
            let count = unsafe {
                DragQueryFileW(hdrop, DRAGDROP_GET_FILES_COUNT, core::ptr::null_mut(), 0)
            };
            let mut paths = Vec::new();
            for index in 0..count {
                let len =
                    unsafe { DragQueryFileW(hdrop, index, core::ptr::null_mut(), 0) } as usize;
                if len == 0 {
                    continue;
                }
                let mut buf = vec![0u16; len + 1];
                let written =
                    unsafe { DragQueryFileW(hdrop, index, buf.as_mut_ptr(), (len + 1) as u32) }
                        as usize;
                if written == 0 {
                    continue;
                }
                let path = PathBuf::from(OsString::from_wide(&buf[..written]));
                if path.is_file() {
                    paths.push(path);
                }
            }
            paths
        })
        .unwrap_or_default()
    }

    /// True when the Windows clipboard holds an image (encoded or `CF_DIB`) or local files (`CF_HDROP`).
    pub fn has_pasteable_media() -> bool {
        if format_available(*CLIPBOARD_PNG_FORMAT)
            || format_available(*CLIPBOARD_GIF_FORMAT)
            || format_available(*CLIPBOARD_JPG_FORMAT)
            || format_available(*CLIPBOARD_SVG_FORMAT)
            || format_available(CF_DIB)
        {
            return true;
        }
        if !format_available(CF_HDROP) {
            return false;
        }
        let Some(_guard) = ClipboardGuard::open() else {
            return false;
        };
        !read_drop_files().is_empty()
    }

    /// Reads image bytes or copied local files from the Windows clipboard.
    pub fn read_pasteable_sources() -> Vec<Source> {
        let Some(_guard) = ClipboardGuard::open() else {
            return Vec::new();
        };
        let encoded = [
            (*CLIPBOARD_PNG_FORMAT, "png"),
            (*CLIPBOARD_GIF_FORMAT, "gif"),
            (*CLIPBOARD_JPG_FORMAT, "jpg"),
            (*CLIPBOARD_SVG_FORMAT, "svg"),
        ];
        for (format, ext) in encoded {
            if format_available(format)
                && let Some(bytes) = read_bytes(format)
                && !bytes.is_empty()
            {
                return vec![Source::Clipboard {
                    bytes,
                    ext: Some(ext),
                }];
            }
        }
        if format_available(CF_DIB)
            && let Some(dib) = read_bytes(CF_DIB)
            && let Some(png) = dib_to_png(&dib)
        {
            return vec![Source::Clipboard {
                bytes: png,
                ext: Some("png"),
            }];
        }
        if format_available(CF_HDROP) {
            let files = read_drop_files();
            if !files.is_empty() {
                return files.into_iter().map(Source::File).collect();
            }
        }
        Vec::new()
    }
}

#[cfg(windows)]
pub use win32::{has_pasteable_media, read_pasteable_sources};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dib_to_png_converts_32bpp_gdi_bitmap_and_repairs_zero_alpha() {
        let mut dib = Vec::new();
        dib.extend_from_slice(&40u32.to_le_bytes()); // biSize
        dib.extend_from_slice(&1i32.to_le_bytes()); // biWidth
        dib.extend_from_slice(&1i32.to_le_bytes()); // biHeight
        dib.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
        dib.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
        dib.extend_from_slice(&0u32.to_le_bytes()); // biCompression = BI_RGB
        dib.extend_from_slice(&4u32.to_le_bytes()); // biSizeImage
        dib.extend_from_slice(&[0u8; 16]); // pels per meter + colors
        // BGRA pixel with alpha = 0 (common in Win32 GDI bitmaps)
        dib.extend_from_slice(&[0x10, 0x20, 0x30, 0x00]);

        let png = dib_to_png(&dib).expect("valid DIB must convert to PNG");
        let decoded = image::load_from_memory_with_format(&png, ImageFormat::Png)
            .expect("output must be valid PNG")
            .into_rgba8();
        assert_eq!(decoded.dimensions(), (1, 1));
        assert_eq!(decoded.get_pixel(0, 0).0, [0x30, 0x20, 0x10, 255]);
    }
}
