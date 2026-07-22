#[cfg(debug_assertions)]
use super::domain::TaskEventType;
#[cfg(any(test, debug_assertions))]
use super::{
    domain::{Clock, LedgerError, SystemClock, UuidIdGenerator},
    service::TaskService,
    sqlite::SqliteLedgerStore,
};
#[cfg(debug_assertions)]
use std::{
    path::Path,
    process::{Command, ExitStatus},
    time::{Duration, Instant},
};

#[cfg(debug_assertions)]
pub fn handle_cli_mode() -> Option<i32> {
    let arguments: Vec<String> = std::env::args().collect();
    if let Some(index) = arguments
        .iter()
        .position(|argument| argument == "--ledger-smoke")
    {
        if index + 1 != arguments.len() {
            eprintln!("ZUOBAN_LEDGER_SMOKE_ERROR=--ledger-smoke 不接受额外参数");
            return Some(2);
        }
        return Some(run_parent_smoke());
    }
    if let Some(index) = arguments
        .iter()
        .position(|argument| argument == "--ledger-crash-child")
    {
        let values = &arguments[index + 1..];
        if values.len() != 3 {
            eprintln!("ZUOBAN_LEDGER_CRASH_CHILD_ERROR=需要 databasePath、taskId、operationId");
            return Some(2);
        }
        return Some(run_crash_child(
            Path::new(&values[0]),
            &values[1],
            &values[2],
        ));
    }
    if let Some(index) = arguments
        .iter()
        .position(|argument| argument == "--ledger-commit-crash-child")
    {
        let values = &arguments[index + 1..];
        if values.len() != 3 {
            eprintln!(
                "ZUOBAN_LEDGER_COMMIT_CRASH_CHILD_ERROR=需要 databasePath、taskId、operationId"
            );
            return Some(2);
        }
        return Some(run_commit_crash_child(
            Path::new(&values[0]),
            &values[1],
            &values[2],
        ));
    }
    None
}

#[cfg(not(debug_assertions))]
pub fn handle_cli_mode() -> Option<i32> {
    None
}

#[cfg(debug_assertions)]
fn run_crash_child(database_path: &Path, task_id: &str, operation_id: &str) -> i32 {
    let result = (|| -> Result<(), LedgerError> {
        let mut store = SqliteLedgerStore::open(database_path)?;
        store.exit_before_commit(77);
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        service.complete_task(operation_id, task_id)?;
        Err(LedgerError::injected("异常退出注入未在提交前终止子进程"))
    })();
    if let Err(error) = result {
        eprintln!("ZUOBAN_LEDGER_CRASH_CHILD_ERROR={error}");
    }
    78
}

#[cfg(debug_assertions)]
fn run_commit_crash_child(database_path: &Path, task_id: &str, operation_id: &str) -> i32 {
    let result = (|| -> Result<(), LedgerError> {
        let store = SqliteLedgerStore::open(database_path)?;
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        service.complete_task(operation_id, task_id)?;
        std::process::exit(76)
    })();
    if let Err(error) = result {
        eprintln!("ZUOBAN_LEDGER_COMMIT_CRASH_CHILD_ERROR={error}");
    }
    78
}

#[cfg(debug_assertions)]
fn run_parent_smoke() -> i32 {
    let root = std::env::temp_dir().join(format!("zuoban-ledger-smoke-{}", uuid::Uuid::new_v4()));
    let database_path = root.join("ledger.sqlite3");
    let result = run_parent_smoke_at(&database_path);
    let cleanup_result = std::fs::remove_dir_all(&root);

    match result {
        Ok(mut report) => {
            if let Err(error) = cleanup_result {
                report["cleanupWarning"] = serde_json::json!(error.to_string());
            }
            println!(
                "ZUOBAN_LEDGER_SMOKE_RESULT={}",
                serde_json::to_string(&report).unwrap_or_else(|_| "{}".to_string())
            );
            if report["passed"].as_bool().unwrap_or(false) {
                0
            } else {
                1
            }
        }
        Err(error) => {
            eprintln!("ZUOBAN_LEDGER_SMOKE_ERROR={error}");
            if let Err(cleanup_error) = cleanup_result {
                eprintln!("ZUOBAN_LEDGER_SMOKE_CLEANUP_WARNING={cleanup_error}");
            }
            1
        }
    }
}

#[cfg(debug_assertions)]
fn run_parent_smoke_at(database_path: &Path) -> Result<serde_json::Value, LedgerError> {
    let (task_a_id, task_b_id) = {
        let store = SqliteLedgerStore::open(database_path)?;
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let task_a = service.capture_task("smoke-capture-a", "整理本周完成记录")?;
        let task_b = service.capture_task("smoke-capture-b", "准备下周计划")?;
        (task_a.task_id, task_b.task_id)
    };

    let child_status = run_ledger_child(
        "--ledger-crash-child",
        database_path,
        &task_a_id,
        "smoke-complete-after-crash",
    )?;
    let crash_exit_verified = child_status.code() == Some(77);
    if !crash_exit_verified {
        return Err(LedgerError::injected(format!(
            "异常退出子进程返回 {:?}，预期 77",
            child_status.code()
        )));
    }

    let rollback_verified = {
        let store = SqliteLedgerStore::open(database_path)?;
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let after_crash = service.snapshot()?;
        after_crash.queue.len() == 2
            && after_crash.completed.is_empty()
            && after_crash.events.len() == 2
            && after_crash.rewards.is_empty()
            && after_crash.balance == 0
            && after_crash
                .current_task
                .as_ref()
                .map(|task| task.id.as_str())
                == Some(task_a_id.as_str())
    };

    let commit_child_status = run_ledger_child(
        "--ledger-commit-crash-child",
        database_path,
        &task_a_id,
        "smoke-complete-after-crash",
    )?;
    let commit_exit_verified = commit_child_status.code() == Some(76);
    if !commit_exit_verified {
        return Err(LedgerError::injected(format!(
            "提交后异常退出子进程返回 {:?}，预期 76",
            commit_child_status.code()
        )));
    }

    let (idempotency_verified, command_conflict_verified, undo_verified, weekly_facts_verified) = {
        let store = SqliteLedgerStore::open(database_path)?;
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let completion = service.complete_task("smoke-complete-after-crash", &task_a_id)?;
        let replay = service.complete_task("smoke-complete-after-crash", &task_a_id)?;
        let after_replay = service.snapshot()?;
        let idempotency_verified = completion.replayed
            && replay.replayed
            && replay.event_id == completion.event_id
            && replay.reward_transaction_id == completion.reward_transaction_id
            && after_replay.events.len() == 3
            && after_replay.rewards.len() == 1
            && after_replay.balance == 1;

        let conflict = service.complete_task("smoke-complete-after-crash", &task_b_id);
        let command_conflict_verified = matches!(
            conflict,
            Err(ref error) if error.code() == "COMMAND_ID_CONFLICT"
        ) && service.snapshot()?.rewards.len() == 1;

        service.undo_completion("smoke-undo-a", &completion.event_id)?;
        let after_undo = service.snapshot()?;
        let undo_verified = after_undo.balance == 0
            && after_undo.events.len() == 4
            && after_undo.rewards.len() == 2
            && after_undo
                .current_task
                .as_ref()
                .map(|task| task.id.as_str())
                == Some(task_b_id.as_str());

        let after_undo_facts = service.weekly_facts(0, i64::MAX)?;
        let undone_excluded = after_undo_facts.effective_completions.is_empty();
        service.complete_task("smoke-complete-b", &task_b_id)?;
        service.complete_task("smoke-recomplete-a", &task_a_id)?;
        let facts = service.weekly_facts(0, i64::MAX)?;
        let weekly_facts_verified = undone_excluded
            && facts.effective_completions.len() == 2
            && facts.effective_completions.iter().all(|event| {
                event.event_type == TaskEventType::Completed && event.id != completion.event_id
            });

        (
            idempotency_verified,
            command_conflict_verified,
            undo_verified,
            weekly_facts_verified,
        )
    };

    let (restart_verified, integrity_verified, final_balance, final_event_count) = {
        let store = SqliteLedgerStore::open(database_path)?;
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let snapshot = service.snapshot()?;
        let integrity = service.verify_integrity()?;
        (
            snapshot.completed.len() == 2
                && snapshot.queue.is_empty()
                && snapshot.balance == 2
                && snapshot.rewards.len() == 4,
            integrity.is_ok(),
            snapshot.balance,
            snapshot.events.len(),
        )
    };

    let passed = crash_exit_verified
        && commit_exit_verified
        && rollback_verified
        && idempotency_verified
        && command_conflict_verified
        && undo_verified
        && weekly_facts_verified
        && restart_verified
        && integrity_verified;
    Ok(serde_json::json!({
        "passed": passed,
        "checks": {
            "crashExitBeforeCommit": crash_exit_verified,
            "crashRollbackAfterReopen": rollback_verified,
            "crashExitAfterCommitBeforeAck": commit_exit_verified,
            "lostAckReplaysCommittedReceipt": idempotency_verified,
            "sameOperationIdIsIdempotent": idempotency_verified,
            "sameOperationIdDifferentRequestRejected": command_conflict_verified,
            "undoAppendsNegativeReward": undo_verified,
            "weeklyFactsExcludeUndoneCompletion": weekly_facts_verified,
            "restartKeepsSnapshotAndLedger": restart_verified,
            "integrityReportPassed": integrity_verified
        },
        "finalBalance": final_balance,
        "finalEventCount": final_event_count,
        "database": database_path.file_name().and_then(|name| name.to_str()),
        "clockSampleMs": SystemClock.now_ms()
    }))
}

#[cfg(debug_assertions)]
fn run_ledger_child(
    mode: &str,
    database_path: &Path,
    task_id: &str,
    operation_id: &str,
) -> Result<ExitStatus, LedgerError> {
    let executable = std::env::current_exe()
        .map_err(|error| LedgerError::storage(format!("读取当前可执行文件失败：{error}")))?;
    let mut child = Command::new(executable)
        .arg(mode)
        .arg(database_path)
        .arg(task_id)
        .arg(operation_id)
        .spawn()
        .map_err(|error| LedgerError::storage(format!("启动账本异常子进程失败：{error}")))?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| LedgerError::storage(format!("等待账本子进程失败：{error}")))?
        {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(LedgerError::storage(format!(
                "账本子进程 {mode} 在 15 秒内未退出"
            )));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(test)]
mod tests;
