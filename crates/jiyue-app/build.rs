//! Embeds the Jiyue icon and version info into `jiyue.exe` (Windows only).
//!
//! The icon is written as resource ID 1, the application icon that GPUI loads for its window
//! class (`LoadImageW(module, 1, IMAGE_ICON)` in gpui's Windows platform code). The title bar,
//! taskbar, Alt-Tab and Explorer therefore show it without any change in the window code.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/logo/jiyue.ico");

    // `cfg!(windows)` in a build script describes the host, not the target, so check the target OS
    // that cargo passes in. This keeps a cross check (for example `--target x86_64-unknown-linux-gnu`
    // from Windows) from running the Windows resource compiler.
    // The `winresource` build-dependency exists only on Windows hosts, so the code that uses it is
    // also gated at compile time.
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/logo/jiyue.ico");
        res.set("ProductName", "Jiyue");
        res.set("CompanyName", "limitcool");
        res.set("LegalCopyright", "\u{a9} 2026 limitcool");
        res.set("FileDescription", "Jiyue - local-first Markdown notes");
        res.set("InternalName", "jiyue");
        res.set("OriginalFilename", "jiyue.exe");
        if let Err(err) = res.compile() {
            panic!("failed to embed the Windows icon and version info: {err}");
        }
    }
}
