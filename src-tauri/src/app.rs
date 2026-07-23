use crate::{
    app_update::AppUpdateService,
    desktop::{state::DesktopState, tray, window, MAIN_WINDOW},
    frontend_probe::FrontendProbeState,
    ledger::LedgerState,
    runtime_profile::RuntimeProfile,
};
use tauri::{Manager, WindowEvent};
use tauri_plugin_window_state::{StateFlags, WindowExt};

pub(crate) fn run() {
    let profile = RuntimeProfile::from_args();
    let state_file = profile.state_file();

    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_filename(state_file)
                .with_state_flags(StateFlags::POSITION)
                .skip_initial_state(MAIN_WINDOW)
                .build(),
        )
        .manage(profile)
        .manage(AppUpdateService::default())
        .manage(DesktopState::default())
        .manage(FrontendProbeState::default())
        .invoke_handler(tauri::generate_handler![
            crate::desktop::commands::set_window_mode,
            crate::desktop::commands::window_status,
            crate::runtime_profile::runtime_profile,
            crate::app_update::commands::check_for_update,
            crate::app_update::commands::install_update,
            crate::desktop::commands::hide_to_tray,
            crate::frontend_probe::report_frontend_ready,
            crate::ledger::commands::capture_task,
            crate::ledger::commands::create_subtask,
            crate::ledger::commands::complete_task,
            crate::ledger::commands::update_task_title,
            crate::ledger::commands::update_task_deadline,
            crate::ledger::commands::delete_task,
            crate::ledger::commands::reorder_tasks,
            crate::ledger::commands::reorder_subtasks,
            crate::ledger::commands::undo_completion,
            crate::ledger::commands::ledger_snapshot,
            crate::ledger::commands::weekly_facts,
            crate::ledger::commands::ledger_integrity
        ])
        .setup(move |app| {
            let ledger_state = if profile.is_smoke() {
                LedgerState::in_memory()
            } else {
                let ledger_path = app.path().app_data_dir()?.join("zuoban-ledger.sqlite3");
                LedgerState::open(&ledger_path)
            }
            .map_err(|error| format!("初始化本地任务账本失败：{error}"))?;
            if !app.manage(ledger_state) {
                return Err("本地任务账本状态重复注册".into());
            }

            let main_window = app
                .get_webview_window(MAIN_WINDOW)
                .ok_or("主悬浮窗没有按配置创建")?;
            let state_path = app.path().app_config_dir()?.join(state_file);
            let had_saved_position = state_path.exists();

            main_window.restore_state(StateFlags::POSITION)?;
            main_window.set_focusable(false)?;
            main_window.set_always_on_top(true)?;
            main_window.set_skip_taskbar(true)?;
            if had_saved_position {
                window::place_at_restored_monitor_bottom_right(&main_window)
                    .map_err(|error| format!("恢复启动胶囊位置失败：{error}"))?;
            } else {
                window::place_at_primary_bottom_right(&main_window)
                    .map_err(|error| format!("设置初始窗口位置失败：{error}"))?;
            }
            tray::install(app)?;
            main_window.show()?;

            if profile.is_smoke() {
                crate::integration_smoke::run(app.handle().clone());
            }
            Ok(())
        })
        .on_window_event(|window_handle, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let app = window_handle.app_handle();
                if let Some(webview_window) = app.get_webview_window(window_handle.label()) {
                    if let Err(error) = window::hide_to_tray(&webview_window) {
                        eprintln!("关闭时隐藏到托盘失败：{error}");
                    }
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                let app = window_handle.app_handle();
                if let Some(webview_window) = app.get_webview_window(window_handle.label()) {
                    if let Err(error) = window::ensure_inside_work_area(&webview_window) {
                        eprintln!("缩放变化后校正窗口失败：{error}");
                    }
                }
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("代办桌面悬浮窗口启动失败");
}
