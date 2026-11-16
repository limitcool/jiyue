//! Windows native OLE `IDropTarget` registration.
//!
//! GPUI 0.2.2's built-in `WindowsDragDropHandler` dereferences `STGMEDIUM.u.hGlobal` as
//! `*mut HDROP`, causing Windows external file drops (`ExternalPaths`) to always yield empty paths.
//! Replacing the window's `IDropTarget` fixes local file drops (`CF_HDROP`) and adds support for
//! dragging image payloads (`PNG` / `GIF` / `JFIF` / `SVG` / `CF_DIB`) from browsers or chat apps.

#[cfg(windows)]
mod win32 {
    use core::ffi::c_void;
    use std::cell::RefCell;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;
    use std::sync::LazyLock;

    use gpui::{AnyWindowHandle, AsyncApp, Entity, ForegroundExecutor, WeakEntity, Window};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::{HWND, POINT, POINTL, S_OK};
    use windows::Win32::Graphics::Gdi::ScreenToClient;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, CoCreateInstance, DVASPECT_CONTENT, FORMATETC, IDataObject,
        TYMED_HGLOBAL,
    };
    use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
    use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
    use windows::Win32::System::Ole::{
        CF_DIB, CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE, IDropTarget,
        IDropTarget_Impl, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop,
    };
    use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
    use windows::Win32::UI::Shell::{
        CLSID_DragDropHelper, DragQueryFileW, HDROP, IDropTargetHelper,
    };
    use windows::core::{PCWSTR, Ref, Result as WinResult, implement};

    use super::super::clipboard_win::dib_to_png;
    use super::super::import::Source;

    const DRAGDROP_GET_FILES_COUNT: u32 = 0xFFFF_FFFF;

    static DROP_PNG_FORMAT: LazyLock<u32> =
        LazyLock::new(|| register_format(windows::core::w!("PNG")));
    static DROP_GIF_FORMAT: LazyLock<u32> =
        LazyLock::new(|| register_format(windows::core::w!("GIF")));
    static DROP_JPG_FORMAT: LazyLock<u32> =
        LazyLock::new(|| register_format(windows::core::w!("JFIF")));
    static DROP_SVG_FORMAT: LazyLock<u32> =
        LazyLock::new(|| register_format(windows::core::w!("image/svg+xml")));

    fn register_format(name: PCWSTR) -> u32 {
        unsafe { RegisterClipboardFormatW(name) }
    }

    fn format_etc(cf: u16) -> FORMATETC {
        FORMATETC {
            cfFormat: cf,
            ptd: core::ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT.0,
            lindex: -1,
            tymed: TYMED_HGLOBAL.0 as u32,
        }
    }

    fn read_hdrop_files(data_obj: &IDataObject) -> Vec<PathBuf> {
        let cfg = format_etc(CF_HDROP.0);
        if unsafe { data_obj.QueryGetData(&cfg) } != S_OK {
            return Vec::new();
        }
        let Ok(mut stg) = (unsafe { data_obj.GetData(&cfg) }) else {
            return Vec::new();
        };
        let mut paths = Vec::new();
        let hglobal = unsafe { stg.u.hGlobal };
        if !hglobal.is_invalid() {
            // `stg.u.hGlobal.0` is already the `HDROP` handle (must not be dereferenced again).
            let hdrop = HDROP(hglobal.0);
            let count = unsafe { DragQueryFileW(hdrop, DRAGDROP_GET_FILES_COUNT, None) };
            for index in 0..count {
                let len = unsafe { DragQueryFileW(hdrop, index, None) } as usize;
                if len == 0 {
                    continue;
                }
                let mut buf = vec![0u16; len + 1];
                let written = unsafe { DragQueryFileW(hdrop, index, Some(&mut buf)) } as usize;
                if written == 0 {
                    continue;
                }
                let path = PathBuf::from(OsString::from_wide(&buf[..written]));
                if path.is_file() {
                    paths.push(path);
                }
            }
        }
        unsafe { ReleaseStgMedium(&mut stg) };
        paths
    }

    fn read_hglobal_bytes(data_obj: &IDataObject, format: u32) -> Option<Vec<u8>> {
        let cf = u16::try_from(format).ok()?;
        let cfg = format_etc(cf);
        if unsafe { data_obj.QueryGetData(&cfg) } != S_OK {
            return None;
        }
        let mut stg = unsafe { data_obj.GetData(&cfg) }.ok()?;
        let hglobal = unsafe { stg.u.hGlobal };
        let bytes = if !hglobal.is_invalid() {
            let size = unsafe { GlobalSize(hglobal) };
            if size > 0 {
                let ptr = unsafe { GlobalLock(hglobal) };
                let out = if !ptr.is_null() {
                    Some(unsafe { std::slice::from_raw_parts(ptr as *const u8, size) }.to_vec())
                } else {
                    None
                };
                let _ = unsafe { GlobalUnlock(hglobal) };
                out
            } else {
                None
            }
        } else {
            None
        };
        unsafe { ReleaseStgMedium(&mut stg) };
        bytes.filter(|b| !b.is_empty())
    }

    fn extract_sources(data_obj: &IDataObject) -> Vec<Source> {
        let files = read_hdrop_files(data_obj);
        if !files.is_empty() {
            return files.into_iter().map(Source::File).collect();
        }
        let encoded = [
            (*DROP_PNG_FORMAT, "png"),
            (*DROP_GIF_FORMAT, "gif"),
            (*DROP_JPG_FORMAT, "jpg"),
            (*DROP_SVG_FORMAT, "svg"),
        ];
        for (format, ext) in encoded {
            if format != 0
                && let Some(bytes) = read_hglobal_bytes(data_obj, format)
            {
                return vec![Source::Clipboard {
                    bytes,
                    ext: Some(ext),
                }];
            }
        }
        if let Some(dib) = read_hglobal_bytes(data_obj, CF_DIB.0 as u32)
            && let Some(png) = dib_to_png(&dib)
        {
            return vec![Source::Clipboard {
                bytes: png,
                ext: Some("png"),
            }];
        }
        Vec::new()
    }

    #[implement(IDropTarget)]
    struct JiyueDropTarget {
        hwnd: HWND,
        helper: Option<IDropTargetHelper>,
        executor: ForegroundExecutor,
        async_app: AsyncApp,
        window_handle: AnyWindowHandle,
        app: WeakEntity<crate::RustdianApp>,
        cached: RefCell<Vec<Source>>,
    }

    impl JiyueDropTarget {
        fn set_drag_over(&self, active: bool) {
            let app_weak = self.app.clone();
            let mut async_app = self.async_app.clone();
            self.executor
                .spawn(async move {
                    let _ = app_weak.update(&mut async_app, |app, cx| {
                        if app.ext.attach.os_drag_over != active {
                            app.ext.attach.os_drag_over = active;
                            cx.notify();
                        }
                    });
                })
                .detach();
        }
    }

    #[allow(non_snake_case)]
    impl IDropTarget_Impl for JiyueDropTarget_Impl {
        fn DragEnter(
            &self,
            pdataobj: Ref<'_, IDataObject>,
            _grfkeystate: MODIFIERKEYS_FLAGS,
            pt: &POINTL,
            pdweffect: *mut DROPEFFECT,
        ) -> WinResult<()> {
            let idata_obj = pdataobj.ok()?;
            let sources = extract_sources(idata_obj);
            let has_media = !sources.is_empty();
            *self.cached.borrow_mut() = sources;

            let effect = if has_media {
                DROPEFFECT_COPY
            } else {
                DROPEFFECT_NONE
            };
            if !pdweffect.is_null() {
                unsafe { *pdweffect = effect };
            }
            if let Some(helper) = &self.helper {
                let cursor = POINT { x: pt.x, y: pt.y };
                let _ = unsafe { helper.DragEnter(self.hwnd, idata_obj, &cursor, effect) };
            }
            self.set_drag_over(has_media);
            Ok(())
        }

        fn DragOver(
            &self,
            _grfkeystate: MODIFIERKEYS_FLAGS,
            pt: &POINTL,
            pdweffect: *mut DROPEFFECT,
        ) -> WinResult<()> {
            let effect = if self.cached.borrow().is_empty() {
                DROPEFFECT_NONE
            } else {
                DROPEFFECT_COPY
            };
            if !pdweffect.is_null() {
                unsafe { *pdweffect = effect };
            }
            if let Some(helper) = &self.helper {
                let cursor = POINT { x: pt.x, y: pt.y };
                let _ = unsafe { helper.DragOver(&cursor, effect) };
            }
            Ok(())
        }

        fn DragLeave(&self) -> WinResult<()> {
            self.cached.borrow_mut().clear();
            if let Some(helper) = &self.helper {
                let _ = unsafe { helper.DragLeave() };
            }
            self.set_drag_over(false);
            Ok(())
        }

        fn Drop(
            &self,
            pdataobj: Ref<'_, IDataObject>,
            _grfkeystate: MODIFIERKEYS_FLAGS,
            pt: &POINTL,
            pdweffect: *mut DROPEFFECT,
        ) -> WinResult<()> {
            let mut sources = std::mem::take(&mut *self.cached.borrow_mut());
            if sources.is_empty()
                && let Ok(obj) = pdataobj.ok()
            {
                sources = extract_sources(obj);
            }
            let effect = if sources.is_empty() {
                DROPEFFECT_NONE
            } else {
                DROPEFFECT_COPY
            };
            if !pdweffect.is_null() {
                unsafe { *pdweffect = effect };
            }
            let screen_pt = POINT { x: pt.x, y: pt.y };
            if let Some(helper) = &self.helper
                && let Ok(obj) = pdataobj.ok()
            {
                let _ = unsafe { helper.Drop(obj, &screen_pt, effect) };
            }
            let mut client_pt = screen_pt;
            let _ = unsafe { ScreenToClient(self.hwnd, &mut client_pt) };

            let window_handle = self.window_handle;
            let mut async_app = self.async_app.clone();
            let app_weak = self.app.clone();
            self.executor
                .spawn(async move {
                    let _ = window_handle.update(&mut async_app, |_, window, cx| {
                        let scale = window.scale_factor().max(0.1);
                        let pos = gpui::point(
                            gpui::px(client_pt.x as f32 / scale),
                            gpui::px(client_pt.y as f32 / scale),
                        );
                        let _ = app_weak.update(cx, |app, cx| {
                            app.ext.attach.os_drag_over = false;
                            if !sources.is_empty() {
                                super::super::drop_sources_at(app, sources, Some(pos), window, cx);
                            }
                            cx.notify();
                        });
                    });
                })
                .detach();
            Ok(())
        }
    }

    /// Replaces GPUI's broken Windows `IDropTarget` with one that handles `CF_HDROP` and images.
    pub fn install_window_drop(
        window: &mut Window,
        app: &Entity<crate::RustdianApp>,
        cx: &mut gpui::App,
    ) {
        let Ok(handle) = HasWindowHandle::window_handle(window) else {
            return;
        };
        let RawWindowHandle::Win32(win32) = handle.as_raw() else {
            return;
        };
        let hwnd = HWND(win32.hwnd.get() as *mut c_void);
        let helper: Option<IDropTargetHelper> =
            unsafe { CoCreateInstance(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER) }.ok();
        let target: IDropTarget = JiyueDropTarget {
            hwnd,
            helper,
            executor: cx.foreground_executor().clone(),
            async_app: cx.to_async(),
            window_handle: gpui::Window::window_handle(window),
            app: app.downgrade(),
            cached: RefCell::new(Vec::new()),
        }
        .into();

        unsafe {
            let _ = RevokeDragDrop(hwnd);
            let _ = RegisterDragDrop(hwnd, &target);
        }
    }
}

#[cfg(windows)]
pub use win32::install_window_drop;

#[cfg(not(windows))]
pub fn install_window_drop(
    _window: &mut gpui::Window,
    _app: &gpui::Entity<crate::RustdianApp>,
    _cx: &mut gpui::App,
) {
}
