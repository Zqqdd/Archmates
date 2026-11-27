//! A real desktop widget on Windows: the main window lives on the desktop itself.
//!
//! - It is owned by the desktop window ("Progman"): Windows then keeps it on screen when you
//!   show the desktop (Win+D, the corner of the taskbar), like a Rainmeter skin.
//! - It stays under your windows (`AlwaysOnBottom`), and is a tool window: no taskbar button,
//!   not in Alt+Tab. The panel (a conversation…) is a normal window, in the taskbar.
//!
//! Elsewhere (Winmates also builds on Linux) the window is a plain borderless one.

use eframe::egui;

/// Where the widget sits among windows.
pub const WIDGET_LEVEL: egui::WindowLevel = egui::WindowLevel::AlwaysOnBottom;

/// Title of the widget window, and of the panel (the one in the taskbar).
pub fn widget_title() -> String {
    format!("{} widget", crate::APP_NAME)
}

pub fn panel_title() -> String {
    crate::APP_NAME.to_string()
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Returns the widget's window handle.
#[cfg(windows)]
pub fn pin(cc: &eframe::CreationContext<'_>) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DwmSetWindowAttribute,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GWL_EXSTYLE, GWLP_HWNDPARENT, GetWindowLongPtrW, SetWindowLongPtrW, WS_EX_APPWINDOW,
        WS_EX_TOOLWINDOW,
    };
    let Ok(handle) = cc.window_handle() else { return None };
    let RawWindowHandle::Win32(h) = handle.as_raw() else { return None };
    let hwnd = h.hwnd.get() as windows_sys::Win32::Foundation::HWND;
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (style | WS_EX_TOOLWINDOW as isize) & !(WS_EX_APPWINDOW as isize));
        let progman = FindWindowW(wide("Progman").as_ptr(), std::ptr::null());
        if !progman.is_null() {
            SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, progman as isize);
        }
        // No corners or border from Windows: the widget draws its own card, rounded like the
        // Noctalia glass (18 px; Windows' corners are 8), on a transparent window.
        let pref = DWMWCP_DONOTROUND;
        DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE as _, (&pref as *const i32).cast(), 4);
        let none = DWMWA_COLOR_NONE;
        DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR as _, (&none as *const u32).cast(), 4);
    }
    Some(hwnd as isize)
}

#[cfg(not(windows))]
pub fn pin(_cc: &eframe::CreationContext<'_>) -> Option<isize> {
    None
}

/// The panel's window handle, once it is on screen (one of ours, found by its title).
#[cfg(windows)]
pub fn panel_hwnd() -> Option<isize> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowThreadProcessId};
    let title = wide(&panel_title());
    let hwnd = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
    if hwnd.is_null() {
        return None;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    (pid == std::process::id()).then_some(hwnd as isize)
}
