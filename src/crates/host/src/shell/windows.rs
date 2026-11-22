//! Windows: the app keeps its taskbar entry, and the global shortcut is not
//! implemented yet (macOS uses ⌘⌥H to show Settings; the tray menu does here).

#[path = "windows/session.rs"]
mod session;

pub(crate) fn prepare(app: &tauri::AppHandle) -> Result<(), String> {
    session::install(app)
}

pub(crate) fn install(_app: &tauri::AppHandle) {}

pub(crate) fn window_opened(_app: &tauri::AppHandle) {}

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

pub(crate) fn statistics_chrome<'a>(
    builder: tauri::WebviewWindowBuilder<'a, tauri::Wry, tauri::AppHandle>,
) -> tauri::WebviewWindowBuilder<'a, tauri::Wry, tauri::AppHandle> {
    builder
}

pub(crate) fn system_prefers_chinese() -> bool {
    // The primary language is the low ten bits; 0x04 is Chinese.
    unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() & 0x3ff == 0x04 }
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
