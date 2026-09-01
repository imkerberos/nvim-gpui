use gpui::App;
use std::borrow::Cow;

pub const SYMBOLS_NERD_FONT_FAMILY: &str = "Symbols Nerd Font";

const SYMBOLS_NERD_FONT: &[u8] = include_bytes!("../assets/fonts/SymbolsNerdFont-Regular.ttf");
const APPLICATION_ICON_PNG: &[u8] = include_bytes!("../assets/neovim-gpui-app-icon.png");

/// Register resources that must be available before the first Neovim redraw
/// is rendered. GPUI keeps these fonts in its in-memory font source, so the
/// user's system font installation is never modified.
pub fn register_bundled_fonts(cx: &App) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    register_font_with_core_text(SYMBOLS_NERD_FONT)?;

    cx.text_system()
        .add_fonts(vec![Cow::Borrowed(SYMBOLS_NERD_FONT)])
        .map_err(|error| format!("failed to register Symbols Nerd Font: {error}"))
}

#[cfg(target_os = "macos")]
fn register_font_with_core_text(font_data: &[u8]) -> Result<(), String> {
    use core_graphics::{data_provider::CGDataProvider, font::CGFont};
    use foreign_types::ForeignType;
    use std::ptr;

    #[link(name = "CoreText", kind = "framework")]
    unsafe extern "C" {
        fn CTFontManagerRegisterGraphicsFont(
            font: *mut core_graphics::sys::CGFont,
            error: *mut *mut std::ffi::c_void,
        ) -> bool;
    }

    let provider = unsafe { CGDataProvider::from_slice(font_data) };
    let font = CGFont::from_data_provider(provider)
        .map_err(|_| "macOS could not decode Symbols Nerd Font".to_owned())?;
    let registered = unsafe { CTFontManagerRegisterGraphicsFont(font.as_ptr(), ptr::null_mut()) };
    if registered {
        Ok(())
    } else {
        Err("macOS could not register Symbols Nerd Font for this process".to_owned())
    }
}

#[cfg(target_os = "macos")]
pub fn install_dock_icon() -> Result<(), String> {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApp, NSImage};
    use objc2_foundation::NSData;

    let marker = MainThreadMarker::new()
        .ok_or_else(|| "Dock icon installation must run on the macOS main thread".to_owned())?;
    let data = NSData::with_bytes(APPLICATION_ICON_PNG);
    let image = NSImage::initWithData(NSImage::alloc(), &data)
        .ok_or_else(|| "macOS could not decode the application icon PNG".to_owned())?;

    // AppKit's setter is unsafe because it accepts an optional image even
    // though NSApplication expects a valid icon for this operation.
    unsafe { NSApp(marker).setApplicationIconImage(Some(&image)) };
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn install_dock_icon() -> Result<(), String> {
    Ok(())
}
