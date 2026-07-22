use crate::{
    desktop::{
        state::{DesktopState, WindowMode},
        window, MAIN_WINDOW,
    },
    frontend_probe::FrontendProbeState,
    ledger::LedgerState,
    runtime_profile::RuntimeProfile,
};
use std::{thread, time::Duration};
use tauri::{AppHandle, Manager};

pub(crate) fn run(app: AppHandle) {
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(1200));
        let result = (|| -> Result<serde_json::Value, String> {
            let frontend_report = app
                .state::<FrontendProbeState>()
                .wait_for_report(Duration::from_secs(10))?;
            let frontend_ready = frontend_report.is_ready_for(RuntimeProfile::Smoke);

            let main_window = app
                .get_webview_window(MAIN_WINDOW)
                .ok_or_else(|| "主悬浮窗不存在".to_string())?;
            let state = app.state::<DesktopState>();
            let startup = window::status(&main_window, &state)?;
            let starts_unfocused = !startup.focused && !startup.focusable;
            let starts_on_top = startup.always_on_top;
            let starts_in_work_area = startup.in_work_area;
            let tray_ready = startup.tray_ready;

            let mut mode_checks = Vec::new();
            for mode in [
                WindowMode::Expanded,
                WindowMode::Capsule,
                WindowMode::Pet,
                WindowMode::Edge,
            ] {
                let status = window::set_mode(&main_window, &state, mode, false)?;
                mode_checks.push(serde_json::json!({
                    "mode": status.mode,
                    "focused": status.focused,
                    "focusable": status.focusable,
                    "inWorkArea": status.in_work_area,
                    "width": status.size.width,
                    "height": status.size.height
                }));
            }

            window::hide_to_tray(&main_window)?;
            let hidden = !window::status(&main_window, &state)?.visible;
            window::restore_expanded(&app)?;
            let restored = window::status(&main_window, &state)?;

            let ledger = app.state::<LedgerState>();
            let captured_a = ledger
                .capture_task("desktop-smoke-capture-a", "桌面联合冒烟任务 A")
                .map_err(|error| error.to_string())?;
            let captured_b = ledger
                .capture_task("desktop-smoke-capture-b", "桌面联合冒烟任务 B")
                .map_err(|error| error.to_string())?;
            let captured_c = ledger
                .capture_task("desktop-smoke-capture-c", "桌面联合冒烟任务 C")
                .map_err(|error| error.to_string())?;
            let ledger_after_capture = ledger.snapshot().map_err(|error| error.to_string())?;
            let updated_b = ledger
                .update_task_title(
                    "desktop-smoke-update-title-b",
                    &captured_b.task_id,
                    "桌面联合冒烟任务 B（修改）",
                )
                .map_err(|error| error.to_string())?;
            let ledger_after_title_update = ledger.snapshot().map_err(|error| error.to_string())?;
            let updated_deadline_b = ledger
                .update_task_deadline(
                    "desktop-smoke-update-deadline-b",
                    &captured_b.task_id,
                    Some("2026-07-20"),
                )
                .map_err(|error| error.to_string())?;
            let ledger_after_deadline_update =
                ledger.snapshot().map_err(|error| error.to_string())?;
            let completed_c = ledger
                .complete_task("desktop-smoke-complete-c", &captured_c.task_id)
                .map_err(|error| error.to_string())?;
            let replayed_c = ledger
                .complete_task("desktop-smoke-complete-c", &captured_c.task_id)
                .map_err(|error| error.to_string())?;
            let ledger_after_completion = ledger.snapshot().map_err(|error| error.to_string())?;
            ledger
                .reorder_tasks(
                    "desktop-smoke-reorder-b-a",
                    &captured_b.task_id,
                    &[captured_a.task_id.clone(), captured_b.task_id.clone()],
                    &[captured_b.task_id.clone(), captured_a.task_id.clone()],
                )
                .map_err(|error| error.to_string())?;
            let ledger_after_reorder = ledger.snapshot().map_err(|error| error.to_string())?;
            ledger
                .undo_completion("desktop-smoke-undo-c", &completed_c.event_id)
                .map_err(|error| error.to_string())?;
            let ledger_after_undo = ledger.snapshot().map_err(|error| error.to_string())?;
            let deleted_a = ledger
                .delete_task("desktop-smoke-delete-a", &captured_a.task_id)
                .map_err(|error| error.to_string())?;
            let ledger_after_delete = ledger.snapshot().map_err(|error| error.to_string())?;
            let ledger_integrity = ledger
                .verify_integrity()
                .map_err(|error| error.to_string())?;
            let ledger_pass = ledger_after_capture.queue.len() == 3
                && ledger_after_capture
                    .current_task
                    .as_ref()
                    .map(|task| &task.id)
                    == Some(&captured_a.task_id)
                && updated_b.task_id == captured_b.task_id
                && updated_b.reward_transaction_id.is_none()
                && ledger_after_title_update.balance == 0
                && ledger_after_title_update
                    .queue
                    .iter()
                    .map(|task| (task.id.as_str(), task.title.as_str()))
                    .eq([
                        (captured_a.task_id.as_str(), "桌面联合冒烟任务 A"),
                        (captured_b.task_id.as_str(), "桌面联合冒烟任务 B（修改）"),
                        (captured_c.task_id.as_str(), "桌面联合冒烟任务 C"),
                    ])
                && ledger_after_title_update
                    .current_task
                    .as_ref()
                    .map(|task| &task.id)
                    == Some(&captured_a.task_id)
                && updated_deadline_b.task_id == captured_b.task_id
                && updated_deadline_b.reward_transaction_id.is_none()
                && ledger_after_deadline_update.balance == 0
                && ledger_after_deadline_update
                    .queue
                    .iter()
                    .find(|task| task.id == captured_b.task_id)
                    .and_then(|task| task.deadline_on.as_deref())
                    == Some("2026-07-20")
                && ledger_after_deadline_update
                    .current_task
                    .as_ref()
                    .map(|task| &task.id)
                    == Some(&captured_a.task_id)
                && !completed_c.replayed
                && replayed_c.replayed
                && replayed_c.event_id == completed_c.event_id
                && ledger_after_completion.balance == 1
                && ledger_after_completion.rewards.len() == 1
                && ledger_after_completion
                    .current_task
                    .as_ref()
                    .map(|task| &task.id)
                    == Some(&captured_a.task_id)
                && ledger_after_reorder
                    .queue
                    .iter()
                    .map(|task| &task.id)
                    .eq([&captured_b.task_id, &captured_a.task_id])
                && ledger_after_reorder
                    .current_task
                    .as_ref()
                    .map(|task| &task.id)
                    == Some(&captured_b.task_id)
                && ledger_after_undo.balance == 0
                && ledger_after_undo.events.len() == 8
                && ledger_after_undo.rewards.len() == 2
                && ledger_after_undo.current_task.as_ref().map(|task| &task.id)
                    == Some(&captured_b.task_id)
                && deleted_a.task_id == captured_a.task_id
                && ledger_after_delete
                    .queue
                    .iter()
                    .map(|task| &task.id)
                    .eq([&captured_b.task_id, &captured_c.task_id])
                && ledger_after_delete.balance == 0
                && ledger_after_delete.events.len() == 9
                && ledger_after_delete.rewards.len() == 2
                && ledger_integrity.is_ok();

            let modes_pass = mode_checks.iter().all(|item| {
                item["inWorkArea"] == true && item["focused"] == false && item["focusable"] == false
            });
            let passed = frontend_ready
                && starts_unfocused
                && starts_on_top
                && starts_in_work_area
                && tray_ready
                && modes_pass
                && hidden
                && restored.visible
                && restored.mode == "expanded"
                && restored.in_work_area
                && restored.focusable
                && ledger_pass;

            Ok(serde_json::json!({
                "passed": passed,
                "checks": {
                    "frontendReady": frontend_ready,
                    "startsUnfocused": starts_unfocused,
                    "startsAlwaysOnTop": starts_on_top,
                    "startsInWorkArea": starts_in_work_area,
                    "trayReady": tray_ready,
                    "allPassiveModesSafe": modes_pass,
                    "hideToTray": hidden,
                    "trayRestoreToExpanded": restored.visible && restored.mode == "expanded",
                    "restoredFocusable": restored.focusable,
                    "ledgerRoundTrip": ledger_pass
                },
                "frontend": frontend_report,
                "ledger": {
                    "capturedTaskCount": ledger_after_capture.queue.len(),
                    "updatedPendingTaskTitle": ledger_after_title_update.queue.iter().any(|task| {
                        task.id == captured_b.task_id && task.title == "桌面联合冒烟任务 B（修改）"
                    }),
                    "updatedPendingTaskDeadline": ledger_after_deadline_update.queue.iter().any(|task| {
                        task.id == captured_b.task_id
                            && task.deadline_on.as_deref() == Some("2026-07-20")
                    }),
                    "completedNonCurrentTask": completed_c.task_id == captured_c.task_id,
                    "idempotentReplay": replayed_c.replayed,
                    "reorderedCurrentTask": ledger_after_reorder.current_task.as_ref().map(|task| &task.id) == Some(&captured_b.task_id),
                    "balanceAfterCompletion": ledger_after_completion.balance,
                    "balanceAfterUndo": ledger_after_undo.balance,
                    "deletedPendingTask": deleted_a.task_id == captured_a.task_id,
                    "eventCountAfterDelete": ledger_after_delete.events.len(),
                    "integrity": ledger_integrity.is_ok()
                },
                "modes": mode_checks,
                "monitor": restored.monitor_name,
                "scaleFactor": restored.scale_factor,
                "finalPosition": restored.position,
                "finalSize": restored.size
            }))
        })();

        match result {
            Ok(report) => {
                println!(
                    "ZUOBAN_SMOKE_RESULT={}",
                    serde_json::to_string(&report).unwrap_or_else(|_| "{}".to_string())
                );
                let passed = report["passed"].as_bool().unwrap_or(false);
                app.exit(if passed { 0 } else { 1 });
            }
            Err(error) => {
                eprintln!("ZUOBAN_SMOKE_ERROR={error}");
                app.exit(1);
            }
        }
    });
}
