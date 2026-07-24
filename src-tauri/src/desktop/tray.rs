use super::{state::DesktopState, window, MAIN_WINDOW};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};
use tauri_plugin_window_state::{AppHandleExt, StateFlags};

pub(crate) fn install(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let show = MenuItem::with_id(app, "show", "打开待办", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "hide", "隐藏到托盘", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "退出待办", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &hide, &separator, &quit])?;
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or("缺少应用图标，无法创建系统托盘")?;

    TrayIconBuilder::with_id("zuoban-main-tray")
        .icon(icon)
        .tooltip("待办")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let result = match event.id().as_ref() {
                "show" => window::restore_expanded(app),
                "hide" => {
                    let main_window = app
                        .get_webview_window(MAIN_WINDOW)
                        .ok_or_else(|| "主悬浮窗不存在".to_string());
                    main_window.and_then(|main_window| window::hide_to_tray(&main_window))
                }
                "quit" => {
                    if let Err(error) = app.save_window_state(StateFlags::POSITION) {
                        eprintln!("退出前保存窗口位置失败：{error}");
                    }
                    app.exit(0);
                    Ok(())
                }
                _ => Ok(()),
            };
            if let Err(error) = result {
                eprintln!("托盘操作失败：{error}");
            }
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                if let Err(error) = window::restore_expanded(tray.app_handle()) {
                    eprintln!("托盘恢复失败：{error}");
                }
            }
        })
        .build(app)?;
    app.state::<DesktopState>().mark_tray_ready();
    Ok(())
}
