use super::{
    state::{DesktopState, WindowMode, WindowStatus},
    MAIN_WINDOW, WINDOW_STATUS_CHANGED_EVENT,
};
use crate::window_geometry::{
    bottom_right, bottom_right_in_selected_work_area, clamp_to_work_area, is_inside_work_area,
    resize_preserving_nearest_edges, select_work_area, Point, Size, WorkArea, WINDOW_MARGIN,
};
use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, WebviewWindow};

fn tauri_error(context: &str, error: impl std::fmt::Display) -> String {
    format!("{context}：{error}")
}

fn monitor_work_area(monitor: &tauri::Monitor) -> WorkArea {
    let area = monitor.work_area();
    WorkArea {
        x: area.position.x,
        y: area.position.y,
        width: area.size.width,
        height: area.size.height,
    }
}

fn available_work_areas(window: &WebviewWindow) -> Result<Vec<WorkArea>, String> {
    window
        .available_monitors()
        .map_err(|error| tauri_error("读取显示器失败", error))
        .map(|monitors| monitors.iter().map(monitor_work_area).collect())
}

fn current_window_geometry(window: &WebviewWindow) -> Result<(Point, Size), String> {
    let position = window
        .outer_position()
        .map_err(|error| tauri_error("读取窗口位置失败", error))?;
    let size = window
        .outer_size()
        .map_err(|error| tauri_error("读取窗口尺寸失败", error))?;
    Ok((
        Point {
            x: position.x,
            y: position.y,
        },
        Size {
            width: size.width,
            height: size.height,
        },
    ))
}

pub(crate) fn ensure_inside_work_area(window: &WebviewWindow) -> Result<(), String> {
    let (position, size) = current_window_geometry(window)?;
    let work_areas = available_work_areas(window)?;
    let selected = select_work_area(position, size, &work_areas)
        .ok_or_else(|| "没有可用显示器，无法校正窗口位置".to_string())?;
    let safe = clamp_to_work_area(position, size, selected);
    if safe != position {
        window
            .set_position(PhysicalPosition::new(safe.x, safe.y))
            .map_err(|error| tauri_error("校正窗口位置失败", error))?;
    }
    Ok(())
}

pub(crate) fn place_at_primary_bottom_right(window: &WebviewWindow) -> Result<(), String> {
    let size = current_window_geometry(window)?.1;
    let primary = window
        .primary_monitor()
        .map_err(|error| tauri_error("读取主显示器失败", error))?
        .or_else(|| window.available_monitors().ok()?.into_iter().next())
        .ok_or_else(|| "没有可用显示器，无法放置悬浮窗".to_string())?;
    let position = bottom_right(size, monitor_work_area(&primary));
    window
        .set_position(PhysicalPosition::new(position.x, position.y))
        .map_err(|error| tauri_error("设置初始窗口位置失败", error))
}

pub(crate) fn place_at_restored_monitor_bottom_right(window: &WebviewWindow) -> Result<(), String> {
    let (restored_position, size) = current_window_geometry(window)?;
    let work_areas = available_work_areas(window)?;
    let position = bottom_right_in_selected_work_area(restored_position, size, &work_areas)
        .ok_or_else(|| "没有可用显示器，无法归位启动胶囊".to_string())?;
    window
        .set_position(PhysicalPosition::new(position.x, position.y))
        .map_err(|error| tauri_error("归位启动胶囊失败", error))
}

fn position_for_mode(
    mode: WindowMode,
    old_position: Point,
    old_size: Size,
    new_size: Size,
    work_area: WorkArea,
) -> Point {
    let mut position = resize_preserving_nearest_edges(old_position, old_size, new_size, work_area);

    if mode.spec().dock_to_edge {
        let old_center = i64::from(old_position.x) + i64::from(old_size.width) / 2;
        let area_center = i64::from(work_area.x) + i64::from(work_area.width) / 2;
        position.x = if old_center <= area_center {
            work_area.x + WINDOW_MARGIN
        } else {
            (i64::from(work_area.x) + i64::from(work_area.width)
                - i64::from(new_size.width)
                - i64::from(WINDOW_MARGIN)) as i32
        };
    }

    clamp_to_work_area(position, new_size, work_area)
}

pub(crate) fn set_mode(
    window: &WebviewWindow,
    state: &DesktopState,
    mode: WindowMode,
    request_focus: bool,
) -> Result<WindowStatus, String> {
    let (old_position, old_size) = current_window_geometry(window)?;
    let work_areas = available_work_areas(window)?;
    let selected = select_work_area(old_position, old_size, &work_areas)
        .ok_or_else(|| "没有可用显示器，无法切换窗口状态".to_string())?;
    let scale_factor = window
        .scale_factor()
        .map_err(|error| tauri_error("读取窗口缩放失败", error))?;
    let spec = mode.spec();
    let new_physical =
        LogicalSize::new(spec.logical_width, spec.logical_height).to_physical(scale_factor);
    let new_size = Size {
        width: new_physical.width,
        height: new_physical.height,
    };
    let next_position = position_for_mode(mode, old_position, old_size, new_size, selected);

    window
        .set_size(LogicalSize::new(spec.logical_width, spec.logical_height))
        .map_err(|error| tauri_error("切换窗口尺寸失败", error))?;
    window
        .set_position(PhysicalPosition::new(next_position.x, next_position.y))
        .map_err(|error| tauri_error("切换窗口位置失败", error))?;
    window
        .set_always_on_top(true)
        .map_err(|error| tauri_error("设置始终置顶失败", error))?;
    window
        .set_skip_taskbar(true)
        .map_err(|error| tauri_error("隐藏任务栏图标失败", error))?;

    let should_focus = spec.allows_focus && request_focus;
    window
        .set_focusable(should_focus)
        .map_err(|error| tauri_error("切换窗口焦点策略失败", error))?;
    state.set_focusable(should_focus);
    window
        .show()
        .map_err(|error| tauri_error("显示悬浮窗失败", error))?;
    if should_focus {
        window
            .set_focus()
            .map_err(|error| tauri_error("聚焦展开窗口失败", error))?;
    }

    state.set_mode(mode)?;
    let status = status(window, state)?;
    window
        .emit(WINDOW_STATUS_CHANGED_EVENT, &status)
        .map_err(|error| tauri_error("同步窗口状态到界面失败", error))?;
    Ok(status)
}

pub(crate) fn status(window: &WebviewWindow, state: &DesktopState) -> Result<WindowStatus, String> {
    let (position, size) = current_window_geometry(window)?;
    let monitors = window
        .available_monitors()
        .map_err(|error| tauri_error("读取显示器状态失败", error))?;
    let work_areas: Vec<WorkArea> = monitors.iter().map(monitor_work_area).collect();
    let selected = select_work_area(position, size, &work_areas);
    let current_monitor = window
        .current_monitor()
        .map_err(|error| tauri_error("读取当前显示器失败", error))?;

    Ok(WindowStatus {
        mode: state.mode()?.as_str().to_string(),
        visible: window
            .is_visible()
            .map_err(|error| tauri_error("读取窗口可见性失败", error))?,
        focused: window
            .is_focused()
            .map_err(|error| tauri_error("读取窗口焦点失败", error))?,
        focusable: state.focusable(),
        always_on_top: window
            .is_always_on_top()
            .map_err(|error| tauri_error("读取置顶状态失败", error))?,
        tray_ready: state.tray_ready(),
        in_work_area: selected
            .map(|area| is_inside_work_area(position, size, area))
            .unwrap_or(false),
        position,
        size,
        work_area: selected,
        monitor_name: current_monitor
            .as_ref()
            .and_then(|monitor| monitor.name().cloned()),
        scale_factor: window
            .scale_factor()
            .map_err(|error| tauri_error("读取窗口缩放失败", error))?,
    })
}

pub(crate) fn hide_to_tray(window: &WebviewWindow) -> Result<(), String> {
    window
        .hide()
        .map_err(|error| tauri_error("隐藏到系统托盘失败", error))
}

pub(crate) fn restore_expanded(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window(MAIN_WINDOW)
        .ok_or_else(|| "主悬浮窗不存在".to_string())?;
    let state = app.state::<DesktopState>();
    set_mode(&window, &state, WindowMode::Expanded, true)?;
    ensure_inside_work_area(&window)
}
