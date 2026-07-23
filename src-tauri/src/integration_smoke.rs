use crate::{
    desktop::{
        state::{DesktopState, WindowMode},
        window, MAIN_WINDOW,
    },
    frontend_probe::FrontendProbeState,
    ledger::{
        domain::{RewardType, TaskEventType, TaskStatus},
        LedgerState,
    },
    runtime_profile::RuntimeProfile,
    window_geometry::bottom_right,
};
use std::{io::Write, thread, time::Duration};
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
            let starts_at_bottom_right = startup
                .work_area
                .map(|area| startup.position == bottom_right(startup.size, area))
                .unwrap_or(false);
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
            let undone_c = ledger
                .undo_completion("desktop-smoke-undo-c", &completed_c.event_id)
                .map_err(|error| error.to_string())?;
            let ledger_after_undo = ledger.snapshot().map_err(|error| error.to_string())?;
            let deleted_a = ledger
                .delete_task("desktop-smoke-delete-a", &captured_a.task_id)
                .map_err(|error| error.to_string())?;
            let ledger_after_delete = ledger.snapshot().map_err(|error| error.to_string())?;
            let completion_reward_recorded = completed_c
                .reward_transaction_id
                .as_deref()
                .map(|reward_id| {
                    ledger_after_completion.rewards.iter().any(|reward| {
                        reward.id == reward_id
                            && reward.task_event_id == completed_c.event_id
                            && reward.reward_type == RewardType::TaskCompletion
                            && reward.amount == 1
                    })
                })
                .unwrap_or(false);
            let undo_reward_recorded = undone_c
                .reward_transaction_id
                .as_deref()
                .map(|reward_id| {
                    ledger_after_undo.rewards.iter().any(|reward| {
                        reward.id == reward_id
                            && reward.task_event_id == undone_c.event_id
                            && reward.reward_type == RewardType::CompletionUndo
                            && reward.amount == -1
                    })
                })
                .unwrap_or(false);
            let undo_event_recorded = ledger_after_undo.events.iter().any(|event| {
                event.id == undone_c.event_id
                    && event.event_type == TaskEventType::CompletionUndone
                    && event.reverses_event_id.as_deref() == Some(completed_c.event_id.as_str())
            });
            let delete_event_recorded = ledger_after_delete.events.iter().any(|event| {
                event.id == deleted_a.event_id
                    && event.event_type == TaskEventType::Abandoned
                    && event.task_id == captured_a.task_id
            });
            let flat_ledger_pass = ledger_after_capture.queue.len() == 3
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
                && completion_reward_recorded
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
                && undo_event_recorded
                && undo_reward_recorded
                && ledger_after_undo.current_task.as_ref().map(|task| &task.id)
                    == Some(&captured_b.task_id)
                && deleted_a.task_id == captured_a.task_id
                && ledger_after_delete
                    .queue
                    .iter()
                    .map(|task| &task.id)
                    .eq([&captured_b.task_id, &captured_c.task_id])
                && ledger_after_delete.balance == 0
                && delete_event_recorded;

            let subtask_smoke = run_subtask_round_trip(&ledger)?;
            let ledger_integrity = ledger
                .verify_integrity()
                .map_err(|error| error.to_string())?;
            let ledger_pass = flat_ledger_pass && subtask_smoke.passed && ledger_integrity.is_ok();

            let modes_pass = mode_checks.iter().all(|item| {
                item["inWorkArea"] == true && item["focused"] == false && item["focusable"] == false
            });
            let passed = frontend_ready
                && starts_unfocused
                && starts_on_top
                && starts_in_work_area
                && starts_at_bottom_right
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
                    "startsAtBottomRight": starts_at_bottom_right,
                    "trayReady": tray_ready,
                    "allPassiveModesSafe": modes_pass,
                    "hideToTray": hidden,
                    "trayRestoreToExpanded": restored.visible && restored.mode == "expanded",
                    "restoredFocusable": restored.focusable,
                    "ledgerRoundTrip": ledger_pass,
                    "subtaskSnapshotProjection": subtask_smoke.snapshot_projection,
                    "subtaskParentCompletionCascade": subtask_smoke.parent_completion_cascaded,
                    "subtaskCompletionRewardRules": subtask_smoke.completion_reward_rules,
                    "subtaskReplayStable": subtask_smoke.replay_stable,
                    "subtaskReorderPersisted": subtask_smoke.reorder_persisted,
                    "subtaskParentCompletionRewarded": subtask_smoke.parent_completion_rewarded,
                    "subtaskUndoGuard": subtask_smoke.child_undo_blocked_while_parent_completed,
                    "subtaskParentUndoRestoresGroup": subtask_smoke.parent_undo_restored_group,
                    "subtaskUndoAfterParentUndo": subtask_smoke.child_undo_after_parent_undo
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
                    "softDeleteEventRecorded": delete_event_recorded,
                    "integrity": ledger_integrity.is_ok(),
                    "subtasks": subtask_smoke.details
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
                let line = format!(
                    "ZUOBAN_SMOKE_RESULT={}",
                    serde_json::to_string(&report).unwrap_or_else(|_| "{}".to_string())
                );
                let _ = writeln!(std::io::stdout().lock(), "{line}");
                let passed = report["passed"].as_bool().unwrap_or(false);
                app.exit(if passed { 0 } else { 1 });
            }
            Err(error) => {
                let _ = writeln!(std::io::stderr().lock(), "ZUOBAN_SMOKE_ERROR={error}");
                app.exit(1);
            }
        }
    });
}

struct SubtaskSmokeResult {
    passed: bool,
    snapshot_projection: bool,
    parent_completion_cascaded: bool,
    completion_reward_rules: bool,
    replay_stable: bool,
    reorder_persisted: bool,
    parent_completion_rewarded: bool,
    child_undo_blocked_while_parent_completed: bool,
    parent_undo_restored_group: bool,
    child_undo_after_parent_undo: bool,
    details: serde_json::Value,
}

fn run_subtask_round_trip(ledger: &LedgerState) -> Result<SubtaskSmokeResult, String> {
    let baseline = ledger.snapshot().map_err(|error| error.to_string())?;
    let baseline_balance = baseline.balance;
    let baseline_reward_count = baseline.rewards.len();

    let parent = ledger
        .capture_task("desktop-smoke-subtask-parent", "桌面联合冒烟父代办")
        .map_err(|error| error.to_string())?;
    let child_a = ledger
        .create_subtask(
            "desktop-smoke-subtask-create-a",
            &parent.task_id,
            "桌面联合冒烟子代办 A",
        )
        .map_err(|error| error.to_string())?;
    let child_b = ledger
        .create_subtask(
            "desktop-smoke-subtask-create-b",
            &parent.task_id,
            "桌面联合冒烟子代办 B",
        )
        .map_err(|error| error.to_string())?;
    let after_create = ledger.snapshot().map_err(|error| error.to_string())?;
    let created_children: Vec<_> = after_create
        .subtasks
        .iter()
        .filter(|task| task.parent_task_id.as_deref() == Some(parent.task_id.as_str()))
        .collect();
    let initial_order: Vec<String> = created_children
        .iter()
        .map(|task| task.id.clone())
        .collect();
    let initial_positions: Vec<Option<i64>> = created_children
        .iter()
        .map(|task| task.sibling_position)
        .collect();
    let parent_queued_after_create = after_create.queue.iter().any(|task| {
        task.id == parent.task_id
            && task.status == TaskStatus::Pending
            && task.parent_task_id.is_none()
    });
    let snapshot_projection = parent_queued_after_create
        && child_a.reward_transaction_id.is_none()
        && child_b.reward_transaction_id.is_none()
        && created_children.len() == 2
        && created_children.iter().all(|task| {
            task.status == TaskStatus::Pending
                && task.parent_task_id.as_deref() == Some(parent.task_id.as_str())
        })
        && initial_order == vec![child_a.task_id.clone(), child_b.task_id.clone()]
        && initial_positions == vec![Some(1), Some(2)];

    let reordered = ledger
        .reorder_subtasks(
            "desktop-smoke-subtask-reorder-b-a",
            &parent.task_id,
            &child_b.task_id,
            &[child_a.task_id.clone(), child_b.task_id.clone()],
            &[child_b.task_id.clone(), child_a.task_id.clone()],
        )
        .map_err(|error| error.to_string())?;
    let after_reorder = ledger.snapshot().map_err(|error| error.to_string())?;
    let reordered_children: Vec<_> = after_reorder
        .subtasks
        .iter()
        .filter(|task| task.parent_task_id.as_deref() == Some(parent.task_id.as_str()))
        .collect();
    let reordered_order: Vec<String> = reordered_children
        .iter()
        .map(|task| task.id.clone())
        .collect();
    let reordered_positions: Vec<Option<i64>> = reordered_children
        .iter()
        .map(|task| task.sibling_position)
        .collect();
    let reorder_persisted = reordered.reward_transaction_id.is_none()
        && reordered_order == vec![child_b.task_id.clone(), child_a.task_id.clone()]
        && reordered_positions == vec![Some(1), Some(2)]
        && after_reorder.events.iter().any(|event| {
            event.id == reordered.event_id && event.event_type == TaskEventType::SubtasksReordered
        });

    let completed_child_b = ledger
        .complete_task("desktop-smoke-subtask-complete-b", &child_b.task_id)
        .map_err(|error| error.to_string())?;
    let before_parent_completion = ledger.snapshot().map_err(|error| error.to_string())?;
    let child_a_before_parent = before_parent_completion
        .subtasks
        .iter()
        .find(|task| task.id == child_a.task_id)
        .cloned()
        .ok_or_else(|| "父项完成前缺少待完成子代办 A".to_string())?;
    let child_b_before_parent = before_parent_completion
        .subtasks
        .iter()
        .find(|task| task.id == child_b.task_id)
        .cloned()
        .ok_or_else(|| "父项完成前缺少已完成子代办 B".to_string())?;

    let completed_parent = ledger
        .complete_task("desktop-smoke-subtask-complete-parent", &parent.task_id)
        .map_err(|error| error.to_string())?;
    let after_parent_completion = ledger.snapshot().map_err(|error| error.to_string())?;
    let replayed_parent = ledger
        .complete_task("desktop-smoke-subtask-complete-parent", &parent.task_id)
        .map_err(|error| error.to_string())?;
    let after_replay = ledger.snapshot().map_err(|error| error.to_string())?;

    let completed_child_a = after_parent_completion
        .subtasks
        .iter()
        .find(|task| task.id == child_a.task_id)
        .ok_or_else(|| "父项完成后缺少子代办 A".to_string())?;
    let completed_child_b_projection = after_parent_completion
        .subtasks
        .iter()
        .find(|task| task.id == child_b.task_id)
        .ok_or_else(|| "父项完成后缺少子代办 B".to_string())?;
    let cascaded_child_event_id = completed_child_a
        .active_completion_event_id
        .as_deref()
        .ok_or_else(|| "级联完成子代办 A 缺少有效完成事件".to_string())?;
    let cascaded_child_event = after_parent_completion
        .events
        .iter()
        .find(|event| event.id == cascaded_child_event_id)
        .ok_or_else(|| "级联完成子代办 A 的事件未写入审计账本".to_string())?;
    let parent_event = after_parent_completion
        .events
        .iter()
        .find(|event| event.id == completed_parent.event_id)
        .ok_or_else(|| "父代办主完成事件未写入审计账本".to_string())?;
    let parent_completion_cascaded = completed_child_a.status == TaskStatus::Completed
        && completed_child_a.version == child_a_before_parent.version + 1
        && completed_child_b_projection.status == TaskStatus::Completed
        && completed_child_b_projection.version == child_b_before_parent.version
        && completed_child_b_projection.active_completion_event_id
            == child_b_before_parent.active_completion_event_id
        && cascaded_child_event.event_type == TaskEventType::SubtaskCompleted
        && cascaded_child_event.command_id == format!("cascade/{cascaded_child_event_id}")
        && cascaded_child_event
            .metadata
            .get("cascadeParentEventId")
            .and_then(|value| value.as_str())
            == Some(completed_parent.event_id.as_str())
        && cascaded_child_event
            .metadata
            .get("cascadeCommandId")
            .and_then(|value| value.as_str())
            == Some("desktop-smoke-subtask-complete-parent")
        && parent_event
            .metadata
            .get("cascadeSubtaskEventIds")
            .and_then(|value| value.as_array())
            .is_some_and(|event_ids| {
                event_ids.as_slice()
                    == [serde_json::Value::String(
                        cascaded_child_event_id.to_string(),
                    )]
            });

    let parent_reward_recorded = completed_parent
        .reward_transaction_id
        .as_deref()
        .map(|reward_id| {
            after_parent_completion.rewards.iter().any(|reward| {
                reward.id == reward_id
                    && reward.task_event_id == completed_parent.event_id
                    && reward.reward_type == RewardType::TaskCompletion
                    && reward.amount == 1
            })
        })
        .unwrap_or(false);
    let completion_reward_rules = completed_child_b.reward_transaction_id.is_none()
        && completed_child_b.balance == baseline_balance
        && before_parent_completion.balance == baseline_balance
        && before_parent_completion.rewards.len() == baseline_reward_count
        && after_parent_completion.rewards.len() == baseline_reward_count + 1
        && !after_parent_completion
            .rewards
            .iter()
            .any(|reward| reward.task_event_id == cascaded_child_event_id)
        && parent_reward_recorded;
    let replay_stable = replayed_parent.replayed
        && replayed_parent.event_id == completed_parent.event_id
        && replayed_parent.reward_transaction_id == completed_parent.reward_transaction_id
        && after_replay.events.len() == after_parent_completion.events.len()
        && after_replay.rewards.len() == after_parent_completion.rewards.len();
    let parent_completion_rewarded = completed_parent.balance == baseline_balance + 1
        && after_parent_completion.balance == baseline_balance + 1
        && parent_reward_recorded
        && after_parent_completion
            .completed
            .iter()
            .any(|task| task.id == parent.task_id && task.status == TaskStatus::Completed)
        && !after_parent_completion
            .queue
            .iter()
            .any(|task| task.id == parent.task_id);

    let blocked_child_undo_error = match ledger.undo_completion(
        "desktop-smoke-subtask-undo-child-while-parent-completed",
        cascaded_child_event_id,
    ) {
        Ok(_) => return Err("父代办完成后子代办撤销意外成功".to_string()),
        Err(error) => error,
    };
    let after_blocked_child_undo = ledger.snapshot().map_err(|error| error.to_string())?;
    let child_undo_blocked_while_parent_completed = blocked_child_undo_error.code()
        == "INVALID_TASK_STATE"
        && after_blocked_child_undo.balance == baseline_balance + 1
        && after_blocked_child_undo
            .completed
            .iter()
            .any(|task| task.id == parent.task_id)
        && !after_blocked_child_undo.events.iter().any(|event| {
            event.command_id == "desktop-smoke-subtask-undo-child-while-parent-completed"
        });

    let undone_parent = ledger
        .undo_completion(
            "desktop-smoke-subtask-undo-parent",
            &completed_parent.event_id,
        )
        .map_err(|error| error.to_string())?;
    let after_parent_undo = ledger.snapshot().map_err(|error| error.to_string())?;
    let parent_undo_reward_recorded = undone_parent
        .reward_transaction_id
        .as_deref()
        .map(|reward_id| {
            after_parent_undo.rewards.iter().any(|reward| {
                reward.id == reward_id
                    && reward.task_event_id == undone_parent.event_id
                    && reward.reward_type == RewardType::CompletionUndo
                    && reward.amount == -1
            })
        })
        .unwrap_or(false);
    let restored_children: Vec<_> = after_parent_undo
        .subtasks
        .iter()
        .filter(|task| task.parent_task_id.as_deref() == Some(parent.task_id.as_str()))
        .collect();
    let parent_undo_restored_group = undone_parent.balance == baseline_balance
        && after_parent_undo.balance == baseline_balance
        && parent_undo_reward_recorded
        && after_parent_undo
            .queue
            .iter()
            .any(|task| task.id == parent.task_id && task.status == TaskStatus::Pending)
        && restored_children.len() == 2
        && restored_children
            .iter()
            .all(|task| task.status == TaskStatus::Completed);

    let undone_child_a = ledger
        .undo_completion(
            "desktop-smoke-subtask-undo-child-after-parent",
            cascaded_child_event_id,
        )
        .map_err(|error| error.to_string())?;
    let final_snapshot = ledger.snapshot().map_err(|error| error.to_string())?;
    let final_children: Vec<_> = final_snapshot
        .subtasks
        .iter()
        .filter(|task| task.parent_task_id.as_deref() == Some(parent.task_id.as_str()))
        .collect();
    let final_order: Vec<String> = final_children.iter().map(|task| task.id.clone()).collect();
    let child_undo_after_parent_undo = undone_child_a.reward_transaction_id.is_none()
        && undone_child_a.balance == baseline_balance
        && final_snapshot.balance == baseline_balance
        && final_snapshot
            .queue
            .iter()
            .any(|task| task.id == parent.task_id && task.status == TaskStatus::Pending)
        && final_order == vec![child_b.task_id.clone(), child_a.task_id.clone()]
        && final_children.iter().any(|task| {
            task.id == child_a.task_id
                && task.status == TaskStatus::Pending
                && task.active_completion_event_id.is_none()
        })
        && final_children.iter().any(|task| {
            task.id == child_b.task_id
                && task.status == TaskStatus::Completed
                && task.active_completion_event_id.as_deref()
                    == Some(completed_child_b.event_id.as_str())
        });

    let passed = snapshot_projection
        && parent_completion_cascaded
        && completion_reward_rules
        && replay_stable
        && reorder_persisted
        && parent_completion_rewarded
        && child_undo_blocked_while_parent_completed
        && parent_undo_restored_group
        && child_undo_after_parent_undo;
    let final_states: Vec<_> = final_children
        .iter()
        .map(|task| {
            serde_json::json!({
                "taskId": task.id,
                "status": task.status,
                "siblingPosition": task.sibling_position
            })
        })
        .collect();
    let details = serde_json::json!({
        "passed": passed,
        "parentTaskId": parent.task_id,
        "childTaskIds": [child_a.task_id, child_b.task_id],
        "snapshotProjection": snapshot_projection,
        "initialOrder": initial_order,
        "initialSiblingPositions": initial_positions,
        "parentCompletionCascaded": parent_completion_cascaded,
        "completionRewardRules": completion_reward_rules,
        "replayStable": replay_stable,
        "cascadedChildEventId": cascaded_child_event_id,
        "reorderedOrder": reordered_order,
        "reorderedSiblingPositions": reordered_positions,
        "parentCompletionRewarded": parent_completion_rewarded,
        "childUndoWhileParentCompleted": {
            "rejected": child_undo_blocked_while_parent_completed,
            "code": blocked_child_undo_error.code(),
            "message": blocked_child_undo_error.message()
        },
        "parentUndoRestoredGroup": parent_undo_restored_group,
        "childUndoAfterParentUndo": child_undo_after_parent_undo,
        "baselineBalance": baseline_balance,
        "finalBalance": final_snapshot.balance,
        "finalOrder": final_order,
        "finalChildren": final_states
    });

    Ok(SubtaskSmokeResult {
        passed,
        snapshot_projection,
        parent_completion_cascaded,
        completion_reward_rules,
        replay_stable,
        reorder_persisted,
        parent_completion_rewarded,
        child_undo_blocked_while_parent_completed,
        parent_undo_restored_group,
        child_undo_after_parent_undo,
        details,
    })
}

#[cfg(test)]
mod tests {
    use super::{run_subtask_round_trip, LedgerState};

    #[test]
    fn subtask_round_trip_covers_projection_guards_rewards_reorder_and_undo() {
        let ledger = LedgerState::in_memory().expect("应创建隔离内存账本");
        let report = run_subtask_round_trip(&ledger).expect("子代办联合闭环应执行完成");

        assert!(report.snapshot_projection, "父子快照投影应完整");
        assert!(
            report.parent_completion_cascaded,
            "父项完成应原子完成仍待办的子项，并保留已完成子项"
        );
        assert!(
            report.completion_reward_rules,
            "子项完成不应产生奖励，父项完成只应增加一枚金币"
        );
        assert!(report.replay_stable, "父项完成幂等重放不应追加重复事件");
        assert!(report.reorder_persisted, "同组重排应写入真实投影");
        assert!(
            report.parent_completion_rewarded,
            "父项最终完成应只增加一枚金币"
        );
        assert!(
            report.child_undo_blocked_while_parent_completed,
            "父项完成时应拒绝直接撤销子项"
        );
        assert!(
            report.parent_undo_restored_group,
            "撤销父项后应恢复可操作父子组"
        );
        assert!(
            report.child_undo_after_parent_undo,
            "撤销父项后应允许撤销子项"
        );
        assert!(report.passed, "子代办联合闭环应整体通过");
        assert!(
            ledger.verify_integrity().expect("应完成完整性检查").is_ok(),
            "闭环结束后账本完整性应通过"
        );
    }
}
