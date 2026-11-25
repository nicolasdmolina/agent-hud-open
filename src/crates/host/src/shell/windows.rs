//! Windows: the app keeps its taskbar entry, and the global shortcut is not
//! implemented yet (macOS uses ⌘⌥H to show Settings; the tray menu does here).

#[path = "windows/session.rs"]
mod session;

pub(crate) fn prepare(app: &tauri::AppHandle) -> Result<(), String> {
    session::install(app)
}

pub(crate) fn install(_app: &tauri::AppHandle) {}

pub(crate) fn window_opened(_window: &tauri::WebviewWindow) {}

pub(crate) fn windows_changed(_app: &tauri::AppHandle) {}

/// Nothing to do, and nothing measured on Windows. By Microsoft's own
/// documents Windows keeps the priority of an app whose windows cannot be
/// seen: on battery it only runs it on the efficient cores at their most
/// efficient clock (Low QoS), and Windows 11 stops promising it a finer
/// timer than the default. If polls are ever seen to stall there,
/// `SetProcessInformation(ProcessPowerThrottling)` with the execution-speed
/// control and an empty state asks for full speed while watching.
pub(crate) fn watching(_on: bool) {}

/// No Dock: Settings opens from the tray menu.
pub(crate) fn reopened(_event: &tauri::RunEvent) -> bool {
    false
}

/// The screen under the pointer, both in physical pixels here.
pub(crate) fn pointer_monitor(app: &tauri::AppHandle) -> Option<tauri::Monitor> {
    let pointer = app.cursor_position().ok()?;
    app.monitor_from_point(pointer.x, pointer.y).ok().flatten()
}

pub(crate) fn toolbar_chrome<'a>(
    builder: tauri::WebviewWindowBuilder<'a, tauri::Wry, tauri::AppHandle>,
) -> tauri::WebviewWindowBuilder<'a, tauri::Wry, tauri::AppHandle> {
    builder
}

pub(crate) fn system_languages() -> Vec<String> {
    use windows::Win32::Globalization::{GetUserDefaultUILanguage, LCIDToLocaleName};
    // The UI language's name, such as "zh-TW" or "ja-JP".
    let mut name = [0u16; 85];
    let length = unsafe { LCIDToLocaleName(GetUserDefaultUILanguage() as u32, Some(&mut name), 0) };
    if length <= 1 {
        return Vec::new();
    }
    vec![String::from_utf16_lossy(&name[..length as usize - 1])]
}

/// A message box from a process that has not started Tauri yet.
pub(crate) fn alert(title: &str, body: &str, _ok: &str) {
    use windows::{
        core::HSTRING,
        Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK},
    };
    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(body),
            &HSTRING::from(title),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

/// No shortcut for the clipboard window on this system yet.
pub(crate) const CLIP_SHORTCUT: bool = false;

pub(crate) fn clip_shortcut(_app: &tauri::AppHandle, _shortcut: Option<crate::inspect::Shortcut>) {}
