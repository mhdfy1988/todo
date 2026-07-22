use super::{
    state::{DesktopState, WindowMode, WindowStatus},
    window,
};
use tauri::{State, WebviewWindow};

#[tauri::command]
pub(crate) fn set_window_mode(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    mode: String,
    request_focus: bool,
) -> Result<WindowStatus, String> {
    window::set_mode(&window, &state, WindowMode::parse(&mode)?, request_focus)
}

#[tauri::command]
pub(crate) fn window_status(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<WindowStatus, String> {
    window::status(&window, &state)
}

#[tauri::command]
pub(crate) fn hide_to_tray(window: WebviewWindow) -> Result<(), String> {
    window::hide_to_tray(&window)
}
