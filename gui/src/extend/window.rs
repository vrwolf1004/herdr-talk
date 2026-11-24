use eframe::egui;
use crate::extend::*;

/// This window's HWND, or 0 when it cannot be read.
pub(crate) fn window_hwnd(cc: &eframe::CreationContext<'_>) -> isize {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match cc.window_handle().map(|handle| handle.as_raw()) {
        Ok(RawWindowHandle::Win32(win32)) => win32.hwnd.get(),
        _ => 0,
    }
}

/// One DWM window attribute. The result is ignored: Windows before 11 (22H2 for the colors) refuses
/// attributes it does not know, and the window then keeps its default look.
pub(crate) fn set_dwm(hwnd: isize, attribute: i32, value: u32) {
    if hwnd == 0 {
        return;
    }
    // SAFETY: hwnd is this window's, and the value is a u32 of the size passed.
    unsafe {
        windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(
            hwnd as windows_sys::Win32::Foundation::HWND,
            attribute as u32,
            std::ptr::from_ref(&value).cast(),
            std::mem::size_of::<u32>() as u32,
        );
    }
}

/// COLORREF is 0x00BBGGRR.
pub(crate) fn colorref(color: egui::Color32) -> u32 {
    u32::from(color.r()) | u32::from(color.g()) << 8 | u32::from(color.b()) << 16
}

/// Square outer corners (Windows 11 rounds them), and the title bar in the theme's colors.
pub(crate) fn style_frame(hwnd: isize, palette: &Palette) {
    const CORNER_PREFERENCE: i32 = 33;
    const BORDER_COLOR: i32 = 34;
    const CAPTION_COLOR: i32 = 35;
    const TEXT_COLOR: i32 = 36;
    const DO_NOT_ROUND: u32 = 1;
    set_dwm(hwnd, CORNER_PREFERENCE, DO_NOT_ROUND);
    set_dwm(hwnd, CAPTION_COLOR, colorref(palette.background));
    set_dwm(hwnd, TEXT_COLOR, colorref(palette.text));
    set_dwm(hwnd, BORDER_COLOR, colorref(palette.border));
}

