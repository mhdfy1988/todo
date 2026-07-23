use super::*;
use crate::ledger::{
    domain::{
        complete_task_transition, undo_completion_transition, CascadedSubtaskCompletion,
        MutationContext, RewardType, StoredReceipt, TaskEventType, TaskStatus,
    },
    service::LedgerStore,
    sqlite::{FailurePoint, SCHEMA_VERSION},
};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicI64, Ordering},
    Arc, Barrier,
};

#[derive(Clone)]
struct ManualClock {
    now_ms: Arc<AtomicI64>,
}

impl ManualClock {
    fn new(now_ms: i64) -> Self {
        Self {
            now_ms: Arc::new(AtomicI64::new(now_ms)),
        }
    }

    fn set(&self, now_ms: i64) {
        self.now_ms.store(now_ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> i64 {
        self.now_ms.load(Ordering::SeqCst)
    }
}

fn service_with_one_task(
) -> Result<TaskService<SqliteLedgerStore, SystemClock, UuidIdGenerator>, LedgerError> {
    let store = SqliteLedgerStore::open_in_memory()?;
    let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
    service.capture_task("capture-one", "写周报")?;
    Ok(service)
}

#[test]
fn ledger_smoke_rolls_back_every_injected_write_point() {
    for point in [
        FailurePoint::AfterTaskWrite,
        FailurePoint::AfterEventAppend,
        FailurePoint::AfterRewardAppend,
        FailurePoint::BeforeCommit,
    ] {
        let mut service = service_with_one_task().expect("测试账本应建立");
        let task_id = service
            .snapshot()
            .expect("应读取快照")
            .current_task
            .expect("应有当前任务")
            .id;
        let mut store = service.into_store();
        store.inject_failure_once(point);
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let error = service
            .complete_task(&format!("complete-{point:?}"), &task_id)
            .expect_err("注入点必须让事务失败");
        assert_eq!(error.code(), "INJECTED_FAILURE");

        let snapshot = service.snapshot().expect("失败后应可继续读取");
        assert_eq!(snapshot.queue.len(), 1, "失败点：{point:?}");
        assert!(snapshot.completed.is_empty(), "失败点：{point:?}");
        assert_eq!(snapshot.events.len(), 1, "失败点：{point:?}");
        assert!(snapshot.rewards.is_empty(), "失败点：{point:?}");
        assert_eq!(snapshot.balance, 0, "失败点：{point:?}");
        assert!(service.verify_integrity().expect("应完成校验").is_ok());
    }
}

#[test]
fn ledger_smoke_replay_and_undo_are_append_only() {
    let mut service = service_with_one_task().expect("测试账本应建立");
    let task_id = service
        .snapshot()
        .expect("应读取快照")
        .current_task
        .expect("应有当前任务")
        .id;
    let completed = service
        .complete_task("complete-one", &task_id)
        .expect("完成应成功");
    assert!(
        service
            .complete_task("complete-one", &task_id)
            .expect("重放应成功")
            .replayed
    );
    service
        .undo_completion("undo-one", &completed.event_id)
        .expect("撤销应成功");

    let snapshot = service.snapshot().expect("应读取最终快照");
    assert_eq!(snapshot.events.len(), 3);
    assert_eq!(snapshot.rewards.len(), 2);
    assert_eq!(snapshot.balance, 0);
    assert!(service
        .weekly_facts(0, i64::MAX)
        .expect("应生成事实")
        .effective_completions
        .is_empty());
}

#[test]
fn ledger_smoke_soft_delete_is_idempotent_and_preserves_queue_history() {
    let store = SqliteLedgerStore::open_in_memory().expect("应建立软删除测试账本");
    let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
    let task_a = service
        .capture_task("capture-delete-a", "任务 A")
        .expect("应创建 A");
    let task_b = service
        .capture_task("capture-delete-b", "任务 B")
        .expect("应创建 B");
    let task_c = service
        .capture_task("capture-delete-c", "任务 C")
        .expect("应创建 C");

    let deleted_b = service
        .delete_task("delete-middle", &task_b.task_id)
        .expect("应删除中间任务 B");
    assert!(!deleted_b.replayed);
    assert!(deleted_b.reward_transaction_id.is_none());
    assert_eq!(
        deleted_b.current_task_id.as_deref(),
        Some(task_a.task_id.as_str())
    );

    let replayed_b = service
        .delete_task("delete-middle", &task_b.task_id)
        .expect("相同删除命令应幂等重放");
    assert!(replayed_b.replayed);
    assert_eq!(replayed_b.event_id, deleted_b.event_id);

    let conflict = service.delete_task("delete-middle", &task_c.task_id);
    assert!(matches!(
        conflict,
        Err(ref error) if error.code() == "COMMAND_ID_CONFLICT"
    ));
    let repeated_with_new_id = service.delete_task("delete-middle-again", &task_b.task_id);
    assert!(matches!(
        repeated_with_new_id,
        Err(ref error) if error.code() == "INVALID_TASK_STATE"
    ));

    let deleted_a = service
        .delete_task("delete-head", &task_a.task_id)
        .expect("应删除队首任务 A");
    assert_eq!(
        deleted_a.current_task_id.as_deref(),
        Some(task_c.task_id.as_str())
    );

    let snapshot = service.snapshot().expect("应读取删除后的快照");
    assert_eq!(
        snapshot
            .queue
            .iter()
            .map(|task| task.id.as_str())
            .collect::<Vec<_>>(),
        vec![task_c.task_id.as_str()]
    );
    assert!(snapshot.completed.is_empty());
    assert_eq!(snapshot.events.len(), 5);
    assert!(snapshot.rewards.is_empty());
    assert_eq!(snapshot.balance, 0);
    let delete_events = snapshot
        .events
        .iter()
        .filter(|event| event.event_type == TaskEventType::Abandoned)
        .collect::<Vec<_>>();
    assert_eq!(delete_events.len(), 2);
    assert!(delete_events.iter().all(|event| {
        event.reason.as_deref() == Some("用户删除") && event.metadata["action"] == "delete"
    }));
    let facts = service
        .weekly_facts(0, i64::MAX)
        .expect("应读取删除后的周报事实");
    assert!(facts.effective_completions.is_empty());
    assert_eq!(facts.ongoing_tasks.len(), 1);
    assert_eq!(facts.ongoing_tasks[0].id, task_c.task_id);
    assert!(service
        .verify_integrity()
        .expect("软删除账本应完成校验")
        .is_ok());

    let store = service.into_store();
    for task_id in [&task_a.task_id, &task_b.task_id] {
        let task = store
            .task_by_id(task_id)
            .expect("应读取软删除任务")
            .expect("软删除不能物理抹掉任务");
        assert_eq!(task.status, TaskStatus::Abandoned);
        assert_eq!(task.queue_position, None);
        assert_eq!(task.abandon_reason.as_deref(), Some("用户删除"));
    }
}

#[test]
fn ledger_smoke_rolls_back_every_soft_delete_write_point() {
    for point in [
        FailurePoint::AfterTaskWrite,
        FailurePoint::AfterEventAppend,
        FailurePoint::AfterRewardAppend,
        FailurePoint::BeforeCommit,
    ] {
        let store = SqliteLedgerStore::open_in_memory().expect("应建立删除回滚账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let task_a = service
            .capture_task(&format!("capture-delete-rollback-a-{point:?}"), "任务 A")
            .expect("应创建 A");
        let task_b = service
            .capture_task(&format!("capture-delete-rollback-b-{point:?}"), "任务 B")
            .expect("应创建 B");
        let operation_id = format!("delete-rollback-{point:?}");
        let mut store = service.into_store();
        store.inject_failure_once(point);
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);

        let error = service
            .delete_task(&operation_id, &task_a.task_id)
            .expect_err("删除注入点必须让事务失败");
        assert_eq!(error.code(), "INJECTED_FAILURE");
        let after_failure = service.snapshot().expect("失败后应读取原队列");
        assert_eq!(
            after_failure
                .queue
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            vec![task_a.task_id.as_str(), task_b.task_id.as_str()],
            "失败点：{point:?}"
        );
        assert_eq!(after_failure.events.len(), 2, "失败点：{point:?}");
        assert!(after_failure.rewards.is_empty(), "失败点：{point:?}");
        assert_eq!(after_failure.balance, 0, "失败点：{point:?}");
        assert!(service
            .verify_integrity()
            .expect("回滚后应完成校验")
            .is_ok());

        let retried = service
            .delete_task(&operation_id, &task_a.task_id)
            .expect("回滚后同一 operationId 应能重新提交");
        assert!(!retried.replayed);
        let after_retry = service.snapshot().expect("应读取重试后的快照");
        assert_eq!(after_retry.queue.len(), 1);
        assert_eq!(after_retry.queue[0].id, task_b.task_id);
        assert_eq!(after_retry.events.len(), 3);
        assert!(after_retry.rewards.is_empty());
        assert_eq!(after_retry.balance, 0);
        assert!(service
            .verify_integrity()
            .expect("重试后应完成校验")
            .is_ok());
    }
}

#[test]
fn ledger_smoke_title_update_is_audited_idempotent_and_preserves_projection() {
    let store = SqliteLedgerStore::open_in_memory().expect("应建立标题修改测试账本");
    let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
    let task_a = service
        .capture_task("capture-title-a", "任务 A")
        .expect("应创建 A");
    let task_b = service
        .capture_task("capture-title-b", "任务 B")
        .expect("应创建 B");

    let updated_b = service
        .update_task_title("update-title-b", &task_b.task_id, " 任务 B（修改） ")
        .expect("应修改非队首任务 B");
    assert!(!updated_b.replayed);
    assert!(updated_b.reward_transaction_id.is_none());
    assert_eq!(
        updated_b.current_task_id.as_deref(),
        Some(task_a.task_id.as_str())
    );
    let replayed_b = service
        .update_task_title("update-title-b", &task_b.task_id, "任务 B（修改）")
        .expect("相同标题修改命令应幂等重放");
    assert!(replayed_b.replayed);
    assert_eq!(replayed_b.event_id, updated_b.event_id);
    assert!(matches!(
        service.update_task_title("update-title-b", &task_b.task_id, "另一标题"),
        Err(ref error) if error.code() == "COMMAND_ID_CONFLICT"
    ));
    assert!(matches!(
        service.update_task_title("update-title-b", &task_a.task_id, "任务 A（修改）"),
        Err(ref error) if error.code() == "COMMAND_ID_CONFLICT"
    ));
    assert!(matches!(
        service.update_task_title("update-title-b-again", &task_b.task_id, "任务 B（修改）"),
        Err(ref error) if error.code() == "INVALID_TASK_STATE"
    ));
    assert!(matches!(
        service.update_task_title("update-title-missing", "missing-task", "不存在"),
        Err(ref error) if error.code() == "TASK_NOT_FOUND"
    ));

    service
        .update_task_title("update-title-a", &task_a.task_id, "任务 A（修改）")
        .expect("应修改队首任务 A");
    let after_updates = service.snapshot().expect("应读取修改后快照");
    assert_eq!(
        after_updates
            .queue
            .iter()
            .map(|task| (task.id.as_str(), task.title.as_str(), task.queue_position))
            .collect::<Vec<_>>(),
        vec![
            (task_a.task_id.as_str(), "任务 A（修改）", Some(1)),
            (task_b.task_id.as_str(), "任务 B（修改）", Some(2)),
        ]
    );
    assert_eq!(
        after_updates
            .current_task
            .as_ref()
            .map(|task| task.id.as_str()),
        Some(task_a.task_id.as_str())
    );
    assert_eq!(after_updates.events.len(), 4);
    assert!(after_updates.rewards.is_empty());
    assert_eq!(after_updates.balance, 0);
    let update_events = after_updates
        .events
        .iter()
        .filter(|event| event.event_type == TaskEventType::TitleUpdated)
        .collect::<Vec<_>>();
    assert_eq!(update_events.len(), 2);
    let event_b = update_events
        .iter()
        .find(|event| event.task_id == task_b.task_id)
        .expect("应保留 B 的标题修改事件");
    assert_eq!(event_b.title_snapshot, "任务 B（修改）");
    assert_eq!(event_b.metadata["beforeTitle"], "任务 B");
    assert_eq!(event_b.metadata["afterTitle"], "任务 B（修改）");

    service
        .complete_task("complete-renamed-b", &task_b.task_id)
        .expect("修改后仍应能完成非队首 B");
    assert!(matches!(
        service.update_task_title("update-completed-b", &task_b.task_id, "完成后修改"),
        Err(ref error) if error.code() == "INVALID_TASK_STATE"
    ));
    let facts = service
        .weekly_facts(0, i64::MAX)
        .expect("应读取修改后的周报事实");
    assert_eq!(facts.effective_completions.len(), 1);
    assert_eq!(
        facts.effective_completions[0].title_snapshot,
        "任务 B（修改）"
    );
    assert_eq!(facts.ongoing_tasks.len(), 1);
    assert_eq!(facts.ongoing_tasks[0].title, "任务 A（修改）");
    assert!(service
        .verify_integrity()
        .expect("标题修改账本应完成校验")
        .is_ok());
}

#[test]
fn ledger_smoke_rolls_back_every_title_update_write_point() {
    for point in [
        FailurePoint::AfterTaskWrite,
        FailurePoint::AfterEventAppend,
        FailurePoint::AfterRewardAppend,
        FailurePoint::BeforeCommit,
    ] {
        let store = SqliteLedgerStore::open_in_memory().expect("应建立标题修改回滚账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let task_a = service
            .capture_task(&format!("capture-title-rollback-a-{point:?}"), "任务 A")
            .expect("应创建 A");
        let task_b = service
            .capture_task(&format!("capture-title-rollback-b-{point:?}"), "任务 B")
            .expect("应创建 B");
        let operation_id = format!("update-title-rollback-{point:?}");
        let mut store = service.into_store();
        store.inject_failure_once(point);
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);

        let error = service
            .update_task_title(&operation_id, &task_a.task_id, "任务 A（修改）")
            .expect_err("标题修改注入点必须让事务失败");
        assert_eq!(error.code(), "INJECTED_FAILURE");
        let after_failure = service.snapshot().expect("失败后应读取原快照");
        assert_eq!(
            after_failure
                .queue
                .iter()
                .map(|task| (task.id.as_str(), task.title.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (task_a.task_id.as_str(), "任务 A"),
                (task_b.task_id.as_str(), "任务 B"),
            ],
            "失败点：{point:?}"
        );
        assert_eq!(after_failure.events.len(), 2, "失败点：{point:?}");
        assert!(after_failure.rewards.is_empty(), "失败点：{point:?}");
        assert_eq!(after_failure.balance, 0, "失败点：{point:?}");
        assert!(service
            .verify_integrity()
            .expect("回滚后应完成校验")
            .is_ok());

        let retried = service
            .update_task_title(&operation_id, &task_a.task_id, "任务 A（修改）")
            .expect("回滚后同一 operationId 应能重新提交");
        assert!(!retried.replayed);
        let after_retry = service.snapshot().expect("应读取重试后快照");
        assert_eq!(after_retry.queue[0].title, "任务 A（修改）");
        assert_eq!(after_retry.queue[1].id, task_b.task_id);
        assert_eq!(after_retry.events.len(), 3);
        assert!(after_retry.rewards.is_empty());
        assert_eq!(after_retry.balance, 0);
        assert!(service
            .verify_integrity()
            .expect("重试后应完成校验")
            .is_ok());
    }
}

#[test]
fn ledger_smoke_deadline_update_is_audited_idempotent_and_preserved_by_lifecycle() {
    let store = SqliteLedgerStore::open_in_memory().expect("应建立截止日期测试账本");
    let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
    let task_a = service
        .capture_task("capture-deadline-a", "任务 A")
        .expect("应创建 A");
    let task_b = service
        .capture_task("capture-deadline-b", "任务 B")
        .expect("应创建 B");

    let updated_b = service
        .update_task_deadline("update-deadline-b", &task_b.task_id, Some(" 2026-07-01 "))
        .expect("应允许给非队首任务设置过去期限");
    assert!(!updated_b.replayed);
    assert!(updated_b.reward_transaction_id.is_none());
    assert_eq!(
        updated_b.current_task_id.as_deref(),
        Some(task_a.task_id.as_str())
    );
    let replayed_b = service
        .update_task_deadline("update-deadline-b", &task_b.task_id, Some("2026-07-01"))
        .expect("规范化后相同请求应幂等重放");
    assert!(replayed_b.replayed);
    assert_eq!(replayed_b.event_id, updated_b.event_id);
    assert!(matches!(
        service.update_task_deadline("update-deadline-b", &task_b.task_id, None),
        Err(ref error) if error.code() == "COMMAND_ID_CONFLICT"
    ));
    assert!(matches!(
        service.update_task_deadline(
            "update-deadline-invalid",
            &task_b.task_id,
            Some("2026-02-30"),
        ),
        Err(ref error) if error.code() == "VALIDATION_ERROR"
    ));
    assert!(matches!(
        service.update_task_deadline(
            "update-deadline-same",
            &task_b.task_id,
            Some("2026-07-01"),
        ),
        Err(ref error) if error.code() == "INVALID_TASK_STATE"
    ));

    service
        .update_task_deadline(
            "update-deadline-b-again",
            &task_b.task_id,
            Some("2026-07-20"),
        )
        .expect("应允许修改期限");
    service
        .update_task_deadline("update-deadline-a", &task_a.task_id, Some("2026-07-18"))
        .expect("应设置 A 期限");
    service
        .update_task_deadline("clear-deadline-a", &task_a.task_id, None)
        .expect("应清除 A 期限");
    service
        .update_task_deadline("restore-deadline-a", &task_a.task_id, Some("2026-07-19"))
        .expect("应重新设置 A 期限");

    let completed_b = service
        .complete_task("complete-dated-b", &task_b.task_id)
        .expect("有期限任务仍应能完成");
    let after_completion = service.snapshot().expect("应读取完成后快照");
    assert_eq!(
        after_completion.completed[0].deadline_on.as_deref(),
        Some("2026-07-20")
    );
    service
        .undo_completion("undo-dated-b", &completed_b.event_id)
        .expect("撤销完成应保留期限");
    let after_undo = service.snapshot().expect("应读取撤销后快照");
    assert_eq!(
        after_undo
            .queue
            .iter()
            .find(|task| task.id == task_b.task_id)
            .and_then(|task| task.deadline_on.as_deref()),
        Some("2026-07-20")
    );

    service
        .delete_task("delete-dated-a", &task_a.task_id)
        .expect("删除有期限任务应成功");
    assert!(service
        .verify_integrity()
        .expect("截止日期账本应完成校验")
        .is_ok());
    let store = service.into_store();
    let deleted_a = store
        .task_by_id(&task_a.task_id)
        .expect("应读取已删除任务")
        .expect("软删除任务仍应存在");
    assert_eq!(deleted_a.status, TaskStatus::Abandoned);
    assert_eq!(deleted_a.deadline_on.as_deref(), Some("2026-07-19"));
}

#[test]
fn ledger_smoke_rolls_back_every_deadline_update_write_point() {
    for point in [
        FailurePoint::AfterTaskWrite,
        FailurePoint::AfterEventAppend,
        FailurePoint::AfterRewardAppend,
        FailurePoint::BeforeCommit,
    ] {
        let store = SqliteLedgerStore::open_in_memory().expect("应建立截止日期回滚账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let task = service
            .capture_task(&format!("capture-deadline-rollback-{point:?}"), "任务 A")
            .expect("应创建任务");
        let operation_id = format!("update-deadline-rollback-{point:?}");
        let mut store = service.into_store();
        store.inject_failure_once(point);
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);

        let error = service
            .update_task_deadline(&operation_id, &task.task_id, Some("2026-07-20"))
            .expect_err("截止日期注入点必须让事务失败");
        assert_eq!(error.code(), "INJECTED_FAILURE");
        let after_failure = service.snapshot().expect("失败后应读取原快照");
        assert_eq!(
            after_failure.queue[0].deadline_on, None,
            "失败点：{point:?}"
        );
        assert_eq!(after_failure.events.len(), 1, "失败点：{point:?}");
        assert!(after_failure.rewards.is_empty(), "失败点：{point:?}");
        assert!(service
            .verify_integrity()
            .expect("回滚后应完成校验")
            .is_ok());

        let retried = service
            .update_task_deadline(&operation_id, &task.task_id, Some("2026-07-20"))
            .expect("回滚后同一 operationId 应能重新提交");
        assert!(!retried.replayed);
        let after_retry = service.snapshot().expect("应读取重试后快照");
        assert_eq!(
            after_retry.queue[0].deadline_on.as_deref(),
            Some("2026-07-20")
        );
        assert_eq!(after_retry.events.len(), 2);
        assert!(after_retry.rewards.is_empty());
        assert!(service
            .verify_integrity()
            .expect("重试后应完成校验")
            .is_ok());
    }
}

#[test]
fn ledger_smoke_completes_any_visible_task_and_persists_reorder() {
    let path = temporary_database_path("complete-any-and-reorder");
    let (task_a_id, task_b_id, task_c_id) = {
        let store = SqliteLedgerStore::open(&path).expect("应建立任意完成测试账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let task_a = service
            .capture_task("capture-any-a", "任务 A")
            .expect("应创建 A");
        let task_b = service
            .capture_task("capture-any-b", "任务 B")
            .expect("应创建 B");
        let task_c = service
            .capture_task("capture-any-c", "任务 C")
            .expect("应创建 C");

        service
            .complete_task("complete-any-c", &task_c.task_id)
            .expect("非队首 C 也应能完成");
        let after_completion = service.snapshot().expect("应读取非队首完成快照");
        assert_eq!(
            after_completion
                .queue
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            vec![task_a.task_id.as_str(), task_b.task_id.as_str()]
        );
        assert_eq!(
            after_completion
                .current_task
                .as_ref()
                .map(|task| task.id.as_str()),
            Some(task_a.task_id.as_str())
        );
        assert_eq!(after_completion.completed[0].id, task_c.task_id);
        assert_eq!(after_completion.balance, 1);

        let expected = vec![task_a.task_id.clone(), task_b.task_id.clone()];
        let ordered = vec![task_b.task_id.clone(), task_a.task_id.clone()];
        let reordered = service
            .reorder_tasks("reorder-b-before-a", &task_b.task_id, &expected, &ordered)
            .expect("应把 B 移到 A 前");
        assert!(!reordered.replayed);
        assert_eq!(
            reordered.current_task_id.as_deref(),
            Some(task_b.task_id.as_str())
        );
        let replay = service
            .reorder_tasks("reorder-b-before-a", &task_b.task_id, &expected, &ordered)
            .expect("相同重排命令应幂等重放");
        assert!(replay.replayed);
        assert_eq!(replay.event_id, reordered.event_id);

        let conflict =
            service.reorder_tasks("reorder-b-before-a", &task_a.task_id, &ordered, &expected);
        assert!(matches!(
            conflict,
            Err(ref error) if error.code() == "COMMAND_ID_CONFLICT"
        ));
        (task_a.task_id, task_b.task_id, task_c.task_id)
    };

    {
        let store = SqliteLedgerStore::open(&path).expect("应重开已重排账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let snapshot = service.snapshot().expect("应读取持久化重排快照");
        assert_eq!(
            snapshot
                .queue
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            vec![task_b_id.as_str(), task_a_id.as_str()]
        );
        assert_eq!(
            snapshot.current_task.as_ref().map(|task| task.id.as_str()),
            Some(task_b_id.as_str())
        );
        assert_eq!(snapshot.completed[0].id, task_c_id);
        assert_eq!(snapshot.events.len(), 5);
        assert_eq!(snapshot.rewards.len(), 1);
        assert!(service.verify_integrity().expect("应校验重排账本").is_ok());
    }
    remove_database_family(&path);
}

#[test]
fn ledger_smoke_rolls_back_every_reorder_write_point() {
    for point in [
        FailurePoint::AfterTaskWrite,
        FailurePoint::AfterEventAppend,
        FailurePoint::AfterRewardAppend,
        FailurePoint::BeforeCommit,
    ] {
        let store = SqliteLedgerStore::open_in_memory().expect("应建立重排回滚账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let task_a = service
            .capture_task(&format!("capture-reorder-a-{point:?}"), "任务 A")
            .expect("应创建 A");
        let task_b = service
            .capture_task(&format!("capture-reorder-b-{point:?}"), "任务 B")
            .expect("应创建 B");
        let expected = vec![task_a.task_id.clone(), task_b.task_id.clone()];
        let ordered = vec![task_b.task_id.clone(), task_a.task_id.clone()];
        let mut store = service.into_store();
        store.inject_failure_once(point);
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);

        let error = service
            .reorder_tasks(
                &format!("reorder-failure-{point:?}"),
                &task_b.task_id,
                &expected,
                &ordered,
            )
            .expect_err("重排注入点必须让事务失败");
        assert_eq!(error.code(), "INJECTED_FAILURE");
        let snapshot = service.snapshot().expect("失败后应读取原顺序");
        assert_eq!(
            snapshot
                .queue
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            vec![task_a.task_id.as_str(), task_b.task_id.as_str()],
            "失败点：{point:?}"
        );
        assert_eq!(snapshot.events.len(), 2, "失败点：{point:?}");
        assert!(service.verify_integrity().expect("应校验回滚账本").is_ok());
    }
}

#[test]
fn ledger_smoke_subtask_lifecycle_keeps_parent_reward_and_visibility_rules() {
    let store = SqliteLedgerStore::open_in_memory().expect("应打开子代办内存账本");
    let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
    let parent = service
        .capture_task("subtask-parent", "写周报")
        .expect("应创建父代办");
    let child_a = service
        .create_subtask("subtask-create-a", &parent.task_id, "汇总本周完成")
        .expect("应创建子代办 A");
    let child_b = service
        .create_subtask("subtask-create-b", &parent.task_id, "整理本周问题")
        .expect("应创建子代办 B");
    let child_c = service
        .create_subtask("subtask-create-c", &parent.task_id, "无需继续的旧步骤")
        .expect("应创建子代办 C");
    let completed_b = service
        .complete_task("subtask-complete-b", &child_b.task_id)
        .expect("应先完成子代办 B");
    service
        .delete_task("subtask-delete-c", &child_c.task_id)
        .expect("应软删除子代办 C");

    let created = service.snapshot().expect("应读取子代办快照");
    assert_eq!(created.queue.len(), 1);
    assert_eq!(
        created.current_task.as_ref().map(|task| &task.id),
        Some(&parent.task_id)
    );
    assert_eq!(created.subtasks.len(), 2);
    assert_eq!(
        created.subtasks[0].parent_task_id.as_deref(),
        Some(parent.task_id.as_str())
    );
    assert_eq!(created.subtasks[0].sibling_position, Some(1));
    assert_eq!(created.subtasks[1].sibling_position, Some(2));
    assert!(completed_b.reward_transaction_id.is_none());
    let child_b_before_parent = created
        .subtasks
        .iter()
        .find(|task| task.id == child_b.task_id)
        .cloned()
        .expect("父项完成前应保留已完成子项 B");

    let parent_completion = service
        .complete_task("subtask-complete-parent", &parent.task_id)
        .expect("父项完成应级联完成仍待办的子项");
    assert!(parent_completion.reward_transaction_id.is_some());
    let completed_parent = service.snapshot().expect("应读取父项完成快照");
    assert_eq!(completed_parent.balance, 1);
    assert!(
        completed_parent.queue.is_empty(),
        "已完成父项不应留在主队列"
    );
    assert_eq!(
        completed_parent.subtasks.len(),
        2,
        "父项完成后仍应保留真实子项投影"
    );
    assert_eq!(completed_parent.effective_completions.len(), 3);
    let child_a_after = completed_parent
        .subtasks
        .iter()
        .find(|task| task.id == child_a.task_id)
        .expect("级联后应保留子代办 A");
    let child_b_after = completed_parent
        .subtasks
        .iter()
        .find(|task| task.id == child_b.task_id)
        .expect("级联后应保留子代办 B");
    assert_eq!(child_a_after.status, TaskStatus::Completed);
    assert_eq!(child_a_after.version, 2);
    assert_eq!(child_b_after.version, child_b_before_parent.version);
    assert_eq!(
        child_b_after.active_completion_event_id, child_b_before_parent.active_completion_event_id,
        "已完成子项不得重复完成"
    );
    let cascaded_child_event_id = child_a_after
        .active_completion_event_id
        .clone()
        .expect("级联完成子项应记录有效完成事件");
    let cascaded_child_event = completed_parent
        .events
        .iter()
        .find(|event| event.id == cascaded_child_event_id)
        .expect("级联完成子事件应进入审计账本");
    assert_eq!(
        cascaded_child_event.event_type,
        TaskEventType::SubtaskCompleted
    );
    assert_eq!(
        cascaded_child_event.command_id,
        format!("cascade/{cascaded_child_event_id}")
    );
    assert_eq!(
        cascaded_child_event
            .metadata
            .get("cascadeParentEventId")
            .and_then(|value| value.as_str()),
        Some(parent_completion.event_id.as_str())
    );
    assert_eq!(
        cascaded_child_event
            .metadata
            .get("cascadeCommandId")
            .and_then(|value| value.as_str()),
        Some("subtask-complete-parent")
    );
    assert!(!completed_parent
        .rewards
        .iter()
        .any(|reward| reward.task_event_id == cascaded_child_event_id));
    let parent_event = completed_parent
        .events
        .iter()
        .find(|event| event.id == parent_completion.event_id)
        .expect("父完成回执应指向父主事件");
    assert_eq!(parent_event.task_id, parent.task_id);
    assert_eq!(parent_event.event_type, TaskEventType::Completed);
    assert_eq!(
        parent_event
            .metadata
            .get("cascadeSubtaskEventIds")
            .and_then(|value| value.as_array()),
        Some(&vec![serde_json::Value::String(
            cascaded_child_event_id.clone()
        )])
    );

    let event_count_before_replay = completed_parent.events.len();
    let reward_count_before_replay = completed_parent.rewards.len();
    let replayed_parent = service
        .complete_task("subtask-complete-parent", &parent.task_id)
        .expect("父项完成命令应幂等重放");
    assert!(replayed_parent.replayed);
    assert_eq!(replayed_parent.event_id, parent_completion.event_id);
    assert_eq!(
        replayed_parent.reward_transaction_id,
        parent_completion.reward_transaction_id
    );
    let after_replay = service.snapshot().expect("应读取重放后快照");
    assert_eq!(after_replay.events.len(), event_count_before_replay);
    assert_eq!(after_replay.rewards.len(), reward_count_before_replay);

    let child_undo_while_parent_completed = service
        .undo_completion("subtask-undo-a-too-early", &cascaded_child_event_id)
        .expect_err("父项已完成时不能直接撤销子项");
    assert_eq!(
        child_undo_while_parent_completed.code(),
        "INVALID_TASK_STATE"
    );

    service
        .undo_completion("subtask-undo-parent", &parent_completion.event_id)
        .expect("应先撤销父项");
    service
        .undo_completion("subtask-undo-a", &cascaded_child_event_id)
        .expect("父项恢复后应允许撤销子项");
    assert_eq!(service.snapshot().expect("应读取撤销后快照").balance, 0);

    service
        .delete_task("subtask-delete-parent", &parent.task_id)
        .expect("父删除只应软删父项");
    let hidden = service.snapshot().expect("应读取父删除后快照");
    assert!(hidden.queue.is_empty());
    assert_eq!(
        hidden.subtasks.len(),
        2,
        "父项软删后应保留未软删子项供历史组统计"
    );
    assert!(hidden
        .subtasks
        .iter()
        .all(|task| task.parent_task_id.as_deref() == Some(parent.task_id.as_str())));
    assert_eq!(
        hidden
            .effective_completions
            .iter()
            .map(|event| event.id.as_str())
            .collect::<Vec<_>>(),
        vec![completed_b.event_id.as_str()],
        "父项与子项 A 撤销后只应保留子项 B 的当前有效完成事实"
    );
    assert!(service
        .verify_integrity()
        .expect("子代办生命周期应完整")
        .is_ok());
    let store = service.into_store();
    assert_eq!(
        store
            .task_by_id(&child_c.task_id)
            .expect("应读取已软删子项 C")
            .expect("子项 C 应保留")
            .status,
        TaskStatus::Abandoned,
        "父项完成必须忽略已放弃子项"
    );
}

#[test]
fn ledger_smoke_rolls_back_every_parent_completion_cascade_write_point() {
    for point in [
        FailurePoint::AfterFirstCompanionTaskWrite,
        FailurePoint::AfterTaskWrite,
        FailurePoint::AfterFirstCompanionEventAppend,
        FailurePoint::AfterEventAppend,
        FailurePoint::AfterRewardAppend,
        FailurePoint::BeforeCommit,
    ] {
        let store = SqliteLedgerStore::open_in_memory().expect("应打开级联回滚内存账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let parent = service
            .capture_task(&format!("cascade-failure-parent-{point:?}"), "发布新版本")
            .expect("应创建父代办");
        service
            .create_subtask(
                &format!("cascade-failure-child-a-{point:?}"),
                &parent.task_id,
                "构建安装包",
            )
            .expect("应创建子代办 A");
        service
            .create_subtask(
                &format!("cascade-failure-child-b-{point:?}"),
                &parent.task_id,
                "验证更新",
            )
            .expect("应创建子代办 B");
        let mut store = service.into_store();
        store.inject_failure_once(point);
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let command_id = format!("cascade-failure-complete-{point:?}");

        let error = service
            .complete_task(&command_id, &parent.task_id)
            .expect_err("注入点应中断父子级联事务");
        assert_eq!(error.code(), "INJECTED_FAILURE", "失败点：{point:?}");
        let rolled_back = service.snapshot().expect("失败后应读取原始快照");
        assert_eq!(rolled_back.balance, 0, "失败点：{point:?}");
        assert!(rolled_back.rewards.is_empty(), "失败点：{point:?}");
        assert_eq!(rolled_back.events.len(), 3, "失败点：{point:?}");
        assert!(
            rolled_back.effective_completions.is_empty(),
            "失败点：{point:?}"
        );
        assert!(rolled_back.queue.iter().any(|task| {
            task.id == parent.task_id && task.status == TaskStatus::Pending && task.version == 1
        }));
        assert_eq!(rolled_back.subtasks.len(), 2, "失败点：{point:?}");
        assert!(rolled_back
            .subtasks
            .iter()
            .all(|task| task.status == TaskStatus::Pending && task.version == 1));
        assert!(
            service
                .verify_integrity()
                .expect("失败回滚后应校验账本")
                .is_ok(),
            "失败点：{point:?}"
        );

        let retried = service
            .complete_task(&command_id, &parent.task_id)
            .expect("同一主命令应可在完整回滚后重试");
        assert!(!retried.replayed, "失败点：{point:?}");
        let completed = service.snapshot().expect("重试后应读取完成快照");
        assert_eq!(completed.balance, 1, "失败点：{point:?}");
        assert_eq!(completed.rewards.len(), 1, "失败点：{point:?}");
        assert_eq!(completed.events.len(), 6, "失败点：{point:?}");
        assert!(completed.queue.is_empty(), "失败点：{point:?}");
        assert!(completed
            .subtasks
            .iter()
            .all(|task| task.status == TaskStatus::Completed));
        assert!(
            service
                .verify_integrity()
                .expect("重试完成后应校验账本")
                .is_ok(),
            "失败点：{point:?}"
        );
    }
}

#[test]
fn ledger_snapshot_effective_completions_and_group_facts_ignore_audit_limit() {
    let store = SqliteLedgerStore::open_in_memory().expect("应打开事件截断测试账本");
    let clock = ManualClock::new(100);
    let mut service = TaskService::new(store, clock.clone(), UuidIdGenerator);
    let parent = service
        .capture_task("facts-parent", "发布版本")
        .expect("应创建父项");
    clock.set(200);
    let child = service
        .create_subtask("facts-child", &parent.task_id, "核对发布说明")
        .expect("应创建子项");
    clock.set(300);
    let child_completion = service
        .complete_task("facts-complete-child", &child.task_id)
        .expect("应完成子项");
    clock.set(400);
    let parent_completion = service
        .complete_task("facts-complete-parent", &parent.task_id)
        .expect("应完成父项");

    for index in 0..105 {
        clock.set(500 + index);
        service
            .capture_task(
                &format!("facts-noise-{index}"),
                &format!("后续审计事件 {index}"),
            )
            .expect("应追加超过审计窗口的其他事件");
    }

    let snapshot = service.snapshot().expect("应读取截断后的快照");
    assert_eq!(snapshot.events.len(), 100, "审计事件仍应保留 LIMIT 100");
    assert!(!snapshot.events.iter().any(|event| {
        event.id == child_completion.event_id || event.id == parent_completion.event_id
    }));
    assert_eq!(
        snapshot
            .effective_completions
            .iter()
            .map(|event| event.id.as_str())
            .collect::<Vec<_>>(),
        vec![
            parent_completion.event_id.as_str(),
            child_completion.event_id.as_str(),
        ],
        "快照有效完成事实应独立于审计截断并按最新完成优先返回"
    );
    assert_eq!(snapshot.subtasks.len(), 1);
    assert_eq!(snapshot.subtasks[0].id, child.task_id);
    let snapshot_json = serde_json::to_value(&snapshot).expect("快照应可序列化");
    assert_eq!(
        snapshot_json["effectiveCompletions"][0]["id"],
        parent_completion.event_id
    );
    assert!(snapshot_json.get("effective_completions").is_none());

    let weekly = service
        .weekly_facts(0, 10_000)
        .expect("周报组事实不应依赖快照审计窗口");
    assert_eq!(
        weekly
            .effective_completions
            .iter()
            .map(|event| event.id.as_str())
            .collect::<Vec<_>>(),
        vec![
            child_completion.event_id.as_str(),
            parent_completion.event_id.as_str(),
        ],
        "周报事实按发生时间输出完整父子完成组"
    );
}

#[test]
fn ledger_weekly_facts_include_cross_week_children_when_parent_completes_this_week() {
    const WEEK_FROM_MS: i64 = 10_000;
    const WEEK_TO_MS: i64 = 20_000;

    let store = SqliteLedgerStore::open_in_memory().expect("应打开跨周父子事实账本");
    let clock = ManualClock::new(1_000);
    let mut service = TaskService::new(store, clock.clone(), UuidIdGenerator);
    let parent = service
        .capture_task("cross-week-parent", "跨周发布")
        .expect("应创建父项");
    clock.set(1_500);
    let child = service
        .create_subtask("cross-week-child", &parent.task_id, "上周已完成子项")
        .expect("应创建子项");
    clock.set(2_000);
    let child_completion = service
        .complete_task("cross-week-complete-child", &child.task_id)
        .expect("应在上周完成子项");

    let ongoing_facts = service
        .weekly_facts(WEEK_FROM_MS, WEEK_TO_MS)
        .expect("应读取父项进行中的本周事实");
    assert!(
        ongoing_facts.effective_completions.is_empty(),
        "普通进行中父项不能带入本周之前的子项完成"
    );

    clock.set(12_000);
    let parent_completion = service
        .complete_task("cross-week-complete-parent", &parent.task_id)
        .expect("应在本周完成父项");
    let completed_facts = service
        .weekly_facts(WEEK_FROM_MS, WEEK_TO_MS)
        .expect("应读取父项本周完成后的组事实");
    assert_eq!(
        completed_facts
            .effective_completions
            .iter()
            .map(|event| (event.id.as_str(), event.occurred_at_ms, event.event_type))
            .collect::<Vec<_>>(),
        vec![
            (
                child_completion.event_id.as_str(),
                2_000,
                TaskEventType::SubtaskCompleted,
            ),
            (
                parent_completion.event_id.as_str(),
                12_000,
                TaskEventType::Completed,
            ),
        ],
        "父项本周完成时应补齐所有当前有效的跨周子项完成事实"
    );
}

#[test]
fn ledger_smoke_deleted_subtask_position_survives_append_and_active_reorder() {
    let store = SqliteLedgerStore::open_in_memory().expect("应打开子代办顺序账本");
    let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
    let parent = service
        .capture_task("position-parent", "发布版本")
        .expect("应创建父项");
    let child_a = service
        .create_subtask("position-create-a", &parent.task_id, "检查构建")
        .expect("应创建 A");
    let child_b = service
        .create_subtask("position-create-b", &parent.task_id, "检查签名")
        .expect("应创建 B");
    let child_c = service
        .create_subtask("position-create-c", &parent.task_id, "检查更新")
        .expect("应创建 C");
    service
        .delete_task("position-delete-b", &child_b.task_id)
        .expect("应软删除中间子项");
    let child_d = service
        .create_subtask("position-create-d", &parent.task_id, "发布草稿")
        .expect("新增子项应追加到包括软删位置在内的末尾");

    service
        .reorder_subtasks(
            "position-reorder",
            &parent.task_id,
            &child_d.task_id,
            &[
                child_a.task_id.clone(),
                child_c.task_id.clone(),
                child_d.task_id.clone(),
            ],
            &[
                child_d.task_id.clone(),
                child_c.task_id.clone(),
                child_a.task_id.clone(),
            ],
        )
        .expect("活跃子项应只在原有活跃位置集合内重排");
    let snapshot = service.snapshot().expect("应读取重排后快照");
    assert_eq!(
        snapshot
            .subtasks
            .iter()
            .map(|task| (task.id.as_str(), task.sibling_position))
            .collect::<Vec<_>>(),
        vec![
            (child_d.task_id.as_str(), Some(1)),
            (child_c.task_id.as_str(), Some(3)),
            (child_a.task_id.as_str(), Some(4)),
        ]
    );
    assert!(service.verify_integrity().expect("顺序账本应完整").is_ok());

    let store = service.into_store();
    let deleted = store
        .task_by_id(&child_b.task_id)
        .expect("应读取软删子项")
        .expect("软删子项投影必须保留");
    assert_eq!(deleted.status, TaskStatus::Abandoned);
    assert_eq!(deleted.sibling_position, Some(2));
}

#[test]
fn ledger_smoke_backs_up_and_migrates_a_populated_v1_ledger_to_v5() {
    let path = temporary_database_path("v1-backup-migration");
    let (task_a_id, task_b_id, completion_event_id) = {
        let store = SqliteLedgerStore::open(&path).expect("应建立迁移样本账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let task_a = service
            .capture_task("v1-capture-a", "迁移任务 A")
            .expect("应创建 A");
        let task_b = service
            .capture_task("v1-capture-b", "迁移任务 B")
            .expect("应创建 B");
        let completion = service
            .complete_task("v1-complete-b", &task_b.task_id)
            .expect("应完成非队首 B");
        (task_a.task_id, task_b.task_id, completion.event_id)
    };
    downgrade_fixture_to_v1(&path);

    {
        let store = SqliteLedgerStore::open(&path).expect("真实 v1 文件应备份并升级到 v5");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let snapshot = service.snapshot().expect("应读取升级后快照");
        assert_eq!(snapshot.schema_version, SCHEMA_VERSION);
        assert_eq!(snapshot.queue[0].id, task_a_id);
        assert_eq!(snapshot.completed[0].id, task_b_id);
        assert_eq!(snapshot.events.len(), 3);
        assert_eq!(snapshot.rewards.len(), 1);
        assert_eq!(snapshot.balance, 1);
        assert!(
            service
                .complete_task("v1-complete-b", &task_b_id)
                .expect("旧命令回执应仍可重放")
                .replayed
        );
        service
            .undo_completion("v2-undo-b", &completion_event_id)
            .expect("旧完成事件升级后仍应可撤销");
        service
            .reorder_tasks(
                "v2-reorder-b-a",
                &task_b_id,
                &[task_a_id.clone(), task_b_id.clone()],
                &[task_b_id.clone(), task_a_id.clone()],
            )
            .expect("升级后应能写入新重排事件");
        assert!(service
            .verify_integrity()
            .expect("升级后应校验通过")
            .is_ok());
    }

    let parent = path.parent().expect("测试账本应有父目录");
    let backup_path = std::fs::read_dir(parent)
        .expect("应读取迁移备份目录")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|candidate| {
            candidate
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("ledger.before-v5."))
        })
        .expect("迁移前必须生成 v1 一致性备份");
    let backup = rusqlite::Connection::open_with_flags(
        &backup_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("应打开迁移前备份");
    let backup_version: i64 = backup
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("应读取备份版本");
    let backup_event_count: i64 = backup
        .query_row("SELECT COUNT(*) FROM task_events", [], |row| row.get(0))
        .expect("应读取备份事件");
    assert_eq!(backup_version, 1);
    assert_eq!(backup_event_count, 3);
    drop(backup);
    remove_database_family(&path);
}

#[test]
fn ledger_smoke_backs_up_and_migrates_a_populated_v2_ledger_to_v5() {
    let path = temporary_database_path("v2-backup-migration");
    let (task_a_id, task_b_id) = {
        let store = SqliteLedgerStore::open(&path).expect("应建立 v2 迁移样本账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let task_a = service
            .capture_task("v2-capture-a", "迁移任务 A")
            .expect("应创建 A");
        let task_b = service
            .capture_task("v2-capture-b", "迁移任务 B")
            .expect("应创建 B");
        service
            .complete_task("v2-complete-b", &task_b.task_id)
            .expect("应完成非队首 B");
        (task_a.task_id, task_b.task_id)
    };
    downgrade_fixture_to_v2(&path);

    {
        let store = SqliteLedgerStore::open(&path).expect("真实 v2 文件应备份并升级到 v5");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let snapshot = service.snapshot().expect("应读取升级后快照");
        assert_eq!(snapshot.schema_version, SCHEMA_VERSION);
        assert_eq!(snapshot.queue[0].id, task_a_id);
        assert_eq!(snapshot.completed[0].id, task_b_id);
        assert_eq!(snapshot.events.len(), 3);
        assert_eq!(snapshot.rewards.len(), 1);
        assert_eq!(snapshot.balance, 1);
        service
            .update_task_title("v3-update-title-a", &task_a_id, "迁移任务 A（修改）")
            .expect("升级后应能写入标题修改事件");
        assert_eq!(
            service.snapshot().expect("应读取标题修改后快照").queue[0].title,
            "迁移任务 A（修改）"
        );
        assert!(service
            .verify_integrity()
            .expect("升级后应校验通过")
            .is_ok());
    }

    let parent = path.parent().expect("测试账本应有父目录");
    let backup_path = std::fs::read_dir(parent)
        .expect("应读取迁移备份目录")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|candidate| {
            candidate
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("ledger.before-v5."))
        })
        .expect("迁移前必须生成 v2 一致性备份");
    let backup = rusqlite::Connection::open_with_flags(
        &backup_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("应打开迁移前备份");
    let backup_version: i64 = backup
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("应读取备份版本");
    let backup_quick_check: String = backup
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .expect("应检查备份");
    let backup_event_count: i64 = backup
        .query_row("SELECT COUNT(*) FROM task_events", [], |row| row.get(0))
        .expect("应读取备份事件");
    assert_eq!(backup_version, 2);
    assert_eq!(backup_quick_check, "ok");
    assert_eq!(backup_event_count, 3);
    drop(backup);
    remove_database_family(&path);
}

#[test]
fn ledger_smoke_backs_up_and_migrates_a_populated_v3_ledger_to_v5() {
    let path = temporary_database_path("v3-backup-migration");
    let task_id = {
        let store = SqliteLedgerStore::open(&path).expect("应建立 v3 迁移样本账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let task = service
            .capture_task("v3-capture-a", "迁移任务 A")
            .expect("应创建 A");
        service
            .update_task_title("v3-update-title-a", &task.task_id, "迁移任务 A（修改）")
            .expect("应先留下 v3 可识别的标题事件");
        task.task_id
    };
    downgrade_fixture_to_v3(&path);

    {
        let store = SqliteLedgerStore::open(&path).expect("真实 v3 文件应备份并升级到 v5");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let snapshot = service.snapshot().expect("应读取升级后快照");
        assert_eq!(snapshot.schema_version, SCHEMA_VERSION);
        assert_eq!(snapshot.queue[0].id, task_id);
        assert_eq!(snapshot.queue[0].title, "迁移任务 A（修改）");
        assert_eq!(snapshot.queue[0].deadline_on, None);
        service
            .update_task_deadline("v4-update-deadline-a", &task_id, Some("2026-07-20"))
            .expect("升级后应能写入截止日期修改事件");
        assert_eq!(
            service.snapshot().expect("应读取截止日期快照").queue[0]
                .deadline_on
                .as_deref(),
            Some("2026-07-20")
        );
        assert!(service
            .verify_integrity()
            .expect("升级后应校验通过")
            .is_ok());
    }

    let parent = path.parent().expect("测试账本应有父目录");
    let backup_path = std::fs::read_dir(parent)
        .expect("应读取迁移备份目录")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|candidate| {
            candidate
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("ledger.before-v5."))
        })
        .expect("迁移前必须生成 v3 一致性备份");
    let backup = rusqlite::Connection::open_with_flags(
        &backup_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("应打开迁移前备份");
    let backup_version: i64 = backup
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("应读取备份版本");
    let deadline_column_count: i64 = backup
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('tasks') WHERE name = 'deadline_on'",
            [],
            |row| row.get(0),
        )
        .expect("应检查 v3 备份字段");
    assert_eq!(backup_version, 3);
    assert_eq!(deadline_column_count, 0);
    drop(backup);
    remove_database_family(&path);
}

#[test]
fn ledger_smoke_backs_up_and_migrates_a_populated_v4_ledger_to_v5() {
    let path = temporary_database_path("v4-backup-migration");
    let (task_id, completion_event_id) = {
        let store = SqliteLedgerStore::open(&path).expect("应建立 v4 迁移样本账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let task = service
            .capture_task("v4-capture-a", "迁移任务 A")
            .expect("应创建迁移任务");
        service
            .update_task_deadline("v4-deadline-a", &task.task_id, Some("2026-07-20"))
            .expect("应留下 v4 截止日期事件");
        let completion = service
            .complete_task("v4-complete-a", &task.task_id)
            .expect("应留下 v4 完成事件");
        (task.task_id, completion.event_id)
    };
    downgrade_fixture_to_v4(&path);

    {
        let store = SqliteLedgerStore::open(&path).expect("真实 v4 文件应备份并升级到 v5");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let snapshot = service.snapshot().expect("应读取升级后快照");
        assert_eq!(snapshot.schema_version, SCHEMA_VERSION);
        assert!(snapshot.subtasks.is_empty());
        assert_eq!(snapshot.completed[0].id, task_id);
        assert_eq!(snapshot.completed[0].parent_task_id, None);
        assert_eq!(snapshot.completed[0].sibling_position, None);
        assert_eq!(
            snapshot.completed[0].deadline_on.as_deref(),
            Some("2026-07-20")
        );
        assert!(
            service
                .complete_task("v4-complete-a", &task_id)
                .expect("v4 旧命令回执应仍可重放")
                .replayed
        );
        service
            .undo_completion("v5-undo-a", &completion_event_id)
            .expect("v4 完成事件升级后仍应可撤销");
        let child = service
            .create_subtask("v5-create-child-after-v4", &task_id, "迁移后新增子代办")
            .expect("v4 升级后的父项应支持新增子代办");
        let parent_completion = service
            .complete_task("v5-cascade-after-v4", &task_id)
            .expect("v4 升级后的账本应支持父子原子完成");
        let cascaded = service.snapshot().expect("应读取迁移后的级联完成");
        assert_eq!(cascaded.balance, 1);
        assert_eq!(
            cascaded
                .subtasks
                .iter()
                .find(|task| task.id == child.task_id)
                .map(|task| task.status),
            Some(TaskStatus::Completed)
        );
        assert!(cascaded.events.iter().any(|event| {
            event.task_id == child.task_id
                && event.event_type == TaskEventType::SubtaskCompleted
                && event
                    .metadata
                    .get("cascadeParentEventId")
                    .and_then(|value| value.as_str())
                    == Some(parent_completion.event_id.as_str())
        }));
        assert!(
            service
                .complete_task("v5-cascade-after-v4", &task_id)
                .expect("迁移后的级联完成应可幂等重放")
                .replayed
        );
        assert!(service
            .verify_integrity()
            .expect("v4 升级账本应通过完整性检查")
            .is_ok());
    }

    let parent = path.parent().expect("测试账本应有父目录");
    let backup_path = std::fs::read_dir(parent)
        .expect("应读取迁移备份目录")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|candidate| {
            candidate
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("ledger.before-v5."))
        })
        .expect("迁移前必须生成 v4 一致性备份");
    let backup = rusqlite::Connection::open_with_flags(
        &backup_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("应打开 v4 迁移前备份");
    let backup_version: i64 = backup
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("应读取 v4 备份版本");
    assert_eq!(backup_version, 4);
    drop(backup);
    remove_database_family(&path);
}

fn downgrade_fixture_to_v1(path: &Path) {
    let mut connection = rusqlite::Connection::open(path).expect("应打开迁移样本");
    connection
        .pragma_update(None, "foreign_keys", "OFF")
        .expect("降级夹具前应关闭外键");
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .expect("应开始夹具降级事务");
    transaction
        .execute_batch(
            "CREATE TABLE task_events_v1 (
                sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                id TEXT NOT NULL UNIQUE,
                command_id TEXT NOT NULL UNIQUE,
                task_id TEXT NOT NULL REFERENCES tasks(id),
                title_snapshot TEXT NOT NULL,
                event_type TEXT NOT NULL CHECK(event_type IN (
                    'created', 'completed', 'completion_undone', 'deferred', 'due_recovered',
                    'blocked', 'recovered', 'abandoned', 'reopened'
                )),
                occurred_at_ms INTEGER NOT NULL CHECK(occurred_at_ms >= 0),
                reason TEXT,
                metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
                reverses_event_id TEXT REFERENCES task_events_v1(id)
            ) STRICT;
            INSERT INTO task_events_v1 (
                sequence, id, command_id, task_id, title_snapshot, event_type,
                occurred_at_ms, reason, metadata_json, reverses_event_id
            )
            SELECT sequence, id, command_id, task_id, title_snapshot, event_type,
                   occurred_at_ms, reason, metadata_json, reverses_event_id
            FROM task_events ORDER BY sequence ASC;
            DROP TABLE task_events;
            ALTER TABLE task_events_v1 RENAME TO task_events;
            CREATE INDEX task_events_task_time_index
                ON task_events(task_id, occurred_at_ms, sequence);
             CREATE INDEX task_events_type_time_index
                 ON task_events(event_type, occurred_at_ms, sequence);
             DELETE FROM schema_migrations;
            INSERT INTO schema_migrations (version, description, applied_at_ms)
                VALUES (1, '真实 v1 迁移测试夹具', 1);",
        )
        .expect("应重建 v1 事件约束");
    rebuild_tasks_without_hierarchy_and_deadline(&transaction);
    transaction
        .pragma_update(None, "user_version", 1)
        .expect("应把夹具标记为 v1");
    transaction.commit().expect("应提交 v1 测试夹具");
}

fn downgrade_fixture_to_v2(path: &Path) {
    let mut connection = rusqlite::Connection::open(path).expect("应打开迁移样本");
    connection
        .pragma_update(None, "foreign_keys", "OFF")
        .expect("降级夹具前应关闭外键");
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .expect("应开始夹具降级事务");
    transaction
        .execute_batch(
            "CREATE TABLE task_events_v2 (
                sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                id TEXT NOT NULL UNIQUE,
                command_id TEXT NOT NULL UNIQUE,
                task_id TEXT NOT NULL REFERENCES tasks(id),
                title_snapshot TEXT NOT NULL,
                event_type TEXT NOT NULL CHECK(event_type IN (
                    'created', 'completed', 'completion_undone', 'deferred', 'due_recovered',
                    'blocked', 'recovered', 'abandoned', 'reopened', 'queue_reordered'
                )),
                occurred_at_ms INTEGER NOT NULL CHECK(occurred_at_ms >= 0),
                reason TEXT,
                metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
                reverses_event_id TEXT REFERENCES task_events_v2(id)
            ) STRICT;
            INSERT INTO task_events_v2 (
                sequence, id, command_id, task_id, title_snapshot, event_type,
                occurred_at_ms, reason, metadata_json, reverses_event_id
            )
            SELECT sequence, id, command_id, task_id, title_snapshot, event_type,
                   occurred_at_ms, reason, metadata_json, reverses_event_id
            FROM task_events ORDER BY sequence ASC;
            DROP TABLE task_events;
            ALTER TABLE task_events_v2 RENAME TO task_events;
            CREATE INDEX task_events_task_time_index
                ON task_events(task_id, occurred_at_ms, sequence);
             CREATE INDEX task_events_type_time_index
                 ON task_events(event_type, occurred_at_ms, sequence);
             DELETE FROM schema_migrations;
            INSERT INTO schema_migrations (version, description, applied_at_ms)
                VALUES (2, '真实 v2 迁移测试夹具', 2);",
        )
        .expect("应重建 v2 事件约束");
    rebuild_tasks_without_hierarchy_and_deadline(&transaction);
    transaction
        .pragma_update(None, "user_version", 2)
        .expect("应把夹具标记为 v2");
    transaction.commit().expect("应提交 v2 测试夹具");
}

fn downgrade_fixture_to_v3(path: &Path) {
    let mut connection = rusqlite::Connection::open(path).expect("应打开迁移样本");
    connection
        .pragma_update(None, "foreign_keys", "OFF")
        .expect("降级夹具前应关闭外键");
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .expect("应开始夹具降级事务");
    transaction
        .execute_batch(
            "CREATE TABLE task_events_v3 (
                sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                id TEXT NOT NULL UNIQUE,
                command_id TEXT NOT NULL UNIQUE,
                task_id TEXT NOT NULL REFERENCES tasks(id),
                title_snapshot TEXT NOT NULL,
                event_type TEXT NOT NULL CHECK(event_type IN (
                    'created', 'completed', 'completion_undone', 'deferred', 'due_recovered',
                    'blocked', 'recovered', 'abandoned', 'reopened', 'queue_reordered',
                    'title_updated'
                )),
                occurred_at_ms INTEGER NOT NULL CHECK(occurred_at_ms >= 0),
                reason TEXT,
                metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
                reverses_event_id TEXT REFERENCES task_events_v3(id)
            ) STRICT;
            INSERT INTO task_events_v3 (
                sequence, id, command_id, task_id, title_snapshot, event_type,
                occurred_at_ms, reason, metadata_json, reverses_event_id
            )
            SELECT sequence, id, command_id, task_id, title_snapshot, event_type,
                   occurred_at_ms, reason, metadata_json, reverses_event_id
            FROM task_events ORDER BY sequence ASC;
            DROP TABLE task_events;
            ALTER TABLE task_events_v3 RENAME TO task_events;
            CREATE INDEX task_events_task_time_index
                ON task_events(task_id, occurred_at_ms, sequence);
            CREATE INDEX task_events_type_time_index
                ON task_events(event_type, occurred_at_ms, sequence);
            DELETE FROM schema_migrations;
            INSERT INTO schema_migrations (version, description, applied_at_ms)
                VALUES (3, '真实 v3 迁移测试夹具', 3);",
        )
        .expect("应重建 v3 事件与任务约束");
    rebuild_tasks_without_hierarchy_and_deadline(&transaction);
    transaction
        .pragma_update(None, "user_version", 3)
        .expect("应把夹具标记为 v3");
    transaction.commit().expect("应提交 v3 测试夹具");
}

fn downgrade_fixture_to_v4(path: &Path) {
    let mut connection = rusqlite::Connection::open(path).expect("应打开迁移样本");
    connection
        .pragma_update(None, "foreign_keys", "OFF")
        .expect("降级夹具前应关闭外键");
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .expect("应开始夹具降级事务");
    transaction
        .execute_batch(
            "CREATE TABLE task_events_v4 (
                sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                id TEXT NOT NULL UNIQUE,
                command_id TEXT NOT NULL UNIQUE,
                task_id TEXT NOT NULL REFERENCES tasks(id),
                title_snapshot TEXT NOT NULL,
                event_type TEXT NOT NULL CHECK(event_type IN (
                    'created', 'completed', 'completion_undone', 'deferred', 'due_recovered',
                    'blocked', 'recovered', 'abandoned', 'reopened', 'queue_reordered',
                    'title_updated', 'deadline_updated'
                )),
                occurred_at_ms INTEGER NOT NULL CHECK(occurred_at_ms >= 0),
                reason TEXT,
                metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
                reverses_event_id TEXT REFERENCES task_events_v4(id)
            ) STRICT;
            INSERT INTO task_events_v4 (
                sequence, id, command_id, task_id, title_snapshot, event_type,
                occurred_at_ms, reason, metadata_json, reverses_event_id
            )
            SELECT sequence, id, command_id, task_id, title_snapshot, event_type,
                   occurred_at_ms, reason, metadata_json, reverses_event_id
            FROM task_events ORDER BY sequence ASC;
            DROP TABLE task_events;
            ALTER TABLE task_events_v4 RENAME TO task_events;
            CREATE INDEX task_events_task_time_index
                ON task_events(task_id, occurred_at_ms, sequence);
            CREATE INDEX task_events_type_time_index
                ON task_events(event_type, occurred_at_ms, sequence);
            DELETE FROM schema_migrations;
            INSERT INTO schema_migrations (version, description, applied_at_ms)
                VALUES (4, '真实 v4 迁移测试夹具', 4);",
        )
        .expect("应重建 v4 事件约束");
    rebuild_tasks_without_hierarchy(&transaction);
    transaction
        .pragma_update(None, "user_version", 4)
        .expect("应把夹具标记为 v4");
    transaction.commit().expect("应提交 v4 测试夹具");
}

fn rebuild_tasks_without_hierarchy(transaction: &rusqlite::Transaction<'_>) {
    transaction
        .execute_batch(
            "CREATE TABLE tasks_v4 (
                id TEXT PRIMARY KEY NOT NULL,
                title TEXT NOT NULL CHECK(length(trim(title)) > 0),
                status TEXT NOT NULL CHECK(status IN ('pending', 'blocked', 'completed', 'abandoned')),
                queue_position INTEGER,
                defer_until_ms INTEGER,
                deadline_on TEXT CHECK(deadline_on IS NULL OR deadline_on GLOB
                    '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
                block_reason TEXT,
                abandon_reason TEXT,
                completed_at_ms INTEGER,
                active_completion_event_id TEXT,
                version INTEGER NOT NULL CHECK(version >= 1),
                created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms),
                CHECK(queue_position IS NULL OR queue_position > 0),
                CHECK(
                    (status = 'pending' AND completed_at_ms IS NULL
                        AND active_completion_event_id IS NULL
                        AND ((defer_until_ms IS NULL AND queue_position IS NOT NULL)
                            OR (defer_until_ms IS NOT NULL AND queue_position IS NULL)))
                    OR (status = 'completed' AND queue_position IS NULL
                        AND defer_until_ms IS NULL AND completed_at_ms IS NOT NULL
                        AND active_completion_event_id IS NOT NULL)
                    OR (status IN ('blocked', 'abandoned') AND queue_position IS NULL
                        AND completed_at_ms IS NULL AND active_completion_event_id IS NULL)
                )
            ) STRICT;
            INSERT INTO tasks_v4 (
                id, title, status, queue_position, defer_until_ms, deadline_on,
                block_reason, abandon_reason, completed_at_ms, active_completion_event_id,
                version, created_at_ms, updated_at_ms
            )
            SELECT id, title, status, queue_position, defer_until_ms, deadline_on,
                   block_reason, abandon_reason, completed_at_ms, active_completion_event_id,
                   version, created_at_ms, updated_at_ms
            FROM tasks;
            DROP TABLE tasks;
            ALTER TABLE tasks_v4 RENAME TO tasks;
            CREATE UNIQUE INDEX tasks_queue_position_unique
                ON tasks(queue_position) WHERE queue_position IS NOT NULL;
            CREATE INDEX tasks_status_index ON tasks(status);",
        )
        .expect("应重建保留期限字段但不含层级字段的 v4 任务表");
}

fn rebuild_tasks_without_hierarchy_and_deadline(transaction: &rusqlite::Transaction<'_>) {
    transaction
        .execute_batch(
            "CREATE TABLE tasks_legacy (
                id TEXT PRIMARY KEY NOT NULL,
                title TEXT NOT NULL CHECK(length(trim(title)) > 0),
                status TEXT NOT NULL CHECK(status IN ('pending', 'blocked', 'completed', 'abandoned')),
                queue_position INTEGER,
                defer_until_ms INTEGER,
                block_reason TEXT,
                abandon_reason TEXT,
                completed_at_ms INTEGER,
                active_completion_event_id TEXT,
                version INTEGER NOT NULL CHECK(version >= 1),
                created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms),
                CHECK(queue_position IS NULL OR queue_position > 0),
                CHECK(
                    (status = 'pending' AND completed_at_ms IS NULL
                        AND active_completion_event_id IS NULL
                        AND ((defer_until_ms IS NULL AND queue_position IS NOT NULL)
                            OR (defer_until_ms IS NOT NULL AND queue_position IS NULL)))
                    OR (status = 'completed' AND queue_position IS NULL
                        AND defer_until_ms IS NULL AND completed_at_ms IS NOT NULL
                        AND active_completion_event_id IS NOT NULL)
                    OR (status IN ('blocked', 'abandoned') AND queue_position IS NULL
                        AND completed_at_ms IS NULL AND active_completion_event_id IS NULL)
                )
            ) STRICT;
            INSERT INTO tasks_legacy (
                id, title, status, queue_position, defer_until_ms, block_reason,
                abandon_reason, completed_at_ms, active_completion_event_id,
                version, created_at_ms, updated_at_ms
            )
            SELECT id, title, status, queue_position, defer_until_ms, block_reason,
                   abandon_reason, completed_at_ms, active_completion_event_id,
                   version, created_at_ms, updated_at_ms
            FROM tasks;
            DROP TABLE tasks;
            ALTER TABLE tasks_legacy RENAME TO tasks;
            CREATE UNIQUE INDEX tasks_queue_position_unique
                ON tasks(queue_position) WHERE queue_position IS NOT NULL;
            CREATE INDEX tasks_status_index ON tasks(status);",
        )
        .expect("应重建无层级与期限字段的旧任务表");
}

#[test]
fn ledger_smoke_reopens_file_after_clean_shutdown() {
    let path = temporary_database_path("reopen");
    let task_id = {
        let store = SqliteLedgerStore::open(&path).expect("应新建文件账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let task = service
            .capture_task("capture-reopen", "重启恢复")
            .expect("应创建任务");
        service
            .complete_task("complete-reopen", &task.task_id)
            .expect("应完成任务");
        task.task_id
    };
    {
        let store = SqliteLedgerStore::open(&path).expect("应重开文件账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let snapshot = service.snapshot().expect("应读取恢复快照");
        assert_eq!(snapshot.completed[0].id, task_id);
        assert_eq!(snapshot.balance, 1);
        assert!(service.verify_integrity().expect("应校验恢复数据").is_ok());
    }
    remove_database_family(&path);
}

#[test]
fn ledger_smoke_allows_only_one_concurrent_completion() {
    let path = temporary_database_path("concurrent");
    let task_id = {
        let store = SqliteLedgerStore::open(&path).expect("应新建并发测试账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        service
            .capture_task("capture-concurrent", "只完成一次")
            .expect("应创建任务")
            .task_id
    };

    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for suffix in ["a", "b"] {
        let thread_path = path.clone();
        let thread_task_id = task_id.clone();
        let thread_barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            let store = SqliteLedgerStore::open(&thread_path).expect("线程应打开同一账本");
            let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
            thread_barrier.wait();
            service.complete_task(&format!("complete-concurrent-{suffix}"), &thread_task_id)
        }));
    }
    barrier.wait();
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().expect("并发线程不应恐慌"))
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);

    {
        let store = SqliteLedgerStore::open(&path).expect("应重开并发测试账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let snapshot = service.snapshot().expect("应读取并发结果");
        assert_eq!(snapshot.completed.len(), 1);
        assert_eq!(snapshot.events.len(), 2);
        assert_eq!(snapshot.rewards.len(), 1);
        assert_eq!(snapshot.balance, 1);
    }
    remove_database_family(&path);
}

#[test]
fn ledger_smoke_rejects_stale_parent_cascade_after_concurrent_child_creation() {
    let path = temporary_database_path("stale-parent-cascade-after-child-create");
    let (parent_task_id, first_child_id) = {
        let store = SqliteLedgerStore::open(&path).expect("应建立级联并发账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let parent = service
            .capture_task("stale-cascade-parent", "并发父代办")
            .expect("应创建父代办");
        let child = service
            .create_subtask("stale-cascade-child-a", &parent.task_id, "原有子代办")
            .expect("应创建原有子代办");
        (parent.task_id, child.task_id)
    };

    let mut stale_store = SqliteLedgerStore::open(&path).expect("应打开旧快照连接");
    let parent_before = stale_store
        .task_by_id(&parent_task_id)
        .expect("应读取父项")
        .expect("父项应存在");
    let subtasks_before = stale_store
        .subtasks_for_parent(&parent_task_id)
        .expect("应读取旧子项集合");
    let child_event_id = uuid::Uuid::new_v4().to_string();
    let stale_mutation = complete_task_transition(
        &parent_before,
        None,
        &subtasks_before,
        vec![CascadedSubtaskCompletion {
            task_id: first_child_id.clone(),
            event_id: child_event_id,
        }],
        MutationContext {
            command_id: "stale-cascade-complete-parent".to_string(),
            event_id: uuid::Uuid::new_v4().to_string(),
            reward_transaction_id: Some(uuid::Uuid::new_v4().to_string()),
            occurred_at_ms: SystemClock.now_ms(),
        },
    )
    .expect("旧快照应能构造父子级联转换");

    let second_child_id = {
        let store = SqliteLedgerStore::open(&path).expect("应打开并发新增连接");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        service
            .create_subtask("stale-cascade-child-b", &parent_task_id, "并发新增子代办")
            .expect("并发子项应先提交")
            .task_id
    };

    let error = stale_store
        .commit_transition("complete_task", "stale-cascade-fingerprint", stale_mutation)
        .expect_err("读取后新增子项必须让旧父完成转换失败");
    assert_eq!(error.code(), "CONCURRENT_MODIFICATION");
    let after_conflict = stale_store.snapshot().expect("应读取冲突后快照");
    assert_eq!(after_conflict.balance, 0);
    assert!(after_conflict.rewards.is_empty());
    assert!(after_conflict.queue.iter().any(|task| {
        task.id == parent_task_id && task.status == TaskStatus::Pending && task.version == 1
    }));
    assert_eq!(after_conflict.subtasks.len(), 2);
    assert!(after_conflict.subtasks.iter().all(|task| {
        (task.id == first_child_id || task.id == second_child_id)
            && task.status == TaskStatus::Pending
            && task.version == 1
    }));
    assert!(stale_store
        .verify_integrity()
        .expect("冲突回滚后应校验账本")
        .is_ok());

    let mut service = TaskService::new(stale_store, SystemClock, UuidIdGenerator);
    let retried = service
        .complete_task("stale-cascade-complete-parent", &parent_task_id)
        .expect("刷新子项集合后同一主命令应可重试");
    assert!(!retried.replayed);
    let completed = service.snapshot().expect("应读取重试完成快照");
    assert_eq!(completed.balance, 1);
    assert!(completed.queue.is_empty());
    assert_eq!(completed.subtasks.len(), 2);
    assert!(completed
        .subtasks
        .iter()
        .all(|task| task.status == TaskStatus::Completed));
    assert!(service
        .verify_integrity()
        .expect("重试完成后应校验账本")
        .is_ok());
    drop(service);
    remove_database_family(&path);
}

#[test]
fn ledger_smoke_serializes_create_subtask_against_parent_completion() {
    let path = temporary_database_path("concurrent-create-subtask-complete-parent");
    let parent_task_id = {
        let store = SqliteLedgerStore::open(&path).expect("应建立父子创建竞争账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        service
            .capture_task("capture-create-complete-parent", "并发父代办")
            .expect("应创建父代办")
            .task_id
    };

    let barrier = Arc::new(Barrier::new(3));
    let create_path = path.clone();
    let create_parent_id = parent_task_id.clone();
    let create_barrier = Arc::clone(&barrier);
    let create_handle = std::thread::spawn(move || {
        let store = SqliteLedgerStore::open(&create_path).expect("创建线程应打开同一账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        create_barrier.wait();
        service.create_subtask(
            "concurrent-create-subtask",
            &create_parent_id,
            "并发新增子代办",
        )
    });
    let complete_path = path.clone();
    let complete_parent_id = parent_task_id.clone();
    let complete_barrier = Arc::clone(&barrier);
    let complete_handle = std::thread::spawn(move || {
        let store = SqliteLedgerStore::open(&complete_path).expect("完成线程应打开同一账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        complete_barrier.wait();
        service.complete_task("concurrent-complete-parent", &complete_parent_id)
    });
    barrier.wait();

    let create_result = create_handle.join().expect("创建线程不应恐慌");
    let complete_result = complete_handle.join().expect("完成线程不应恐慌");
    assert!(
        [create_result.is_ok(), complete_result.is_ok()]
            .into_iter()
            .filter(|succeeded| *succeeded)
            .count()
            >= 1,
        "新增子项与完成父项至少应有一个提交成功"
    );

    let mut store = SqliteLedgerStore::open(&path).expect("应重开父子创建竞争账本");
    let parent = store
        .task_by_id(&parent_task_id)
        .expect("应读取父项")
        .expect("父项应存在");
    let snapshot = store.snapshot().expect("应读取竞争结果");
    let parent_completed_outcome = if parent.status == TaskStatus::Completed {
        let reward_recorded = complete_result
            .as_ref()
            .ok()
            .and_then(|receipt| {
                receipt.reward_transaction_id.as_deref().map(|reward_id| {
                    snapshot.rewards.iter().any(|reward| {
                        reward.id == reward_id
                            && reward.task_event_id == receipt.event_id
                            && reward.reward_type == RewardType::TaskCompletion
                            && reward.amount == 1
                    })
                })
            })
            .unwrap_or(false);
        let create_then_cascade = create_result.as_ref().ok().is_some_and(|receipt| {
            snapshot.subtasks.len() == 1
                && snapshot.subtasks.iter().any(|task| {
                    task.id == receipt.task_id
                        && task.parent_task_id.as_deref() == Some(parent_task_id.as_str())
                        && task.status == TaskStatus::Completed
                })
        });
        let complete_then_reject_create =
            is_expected_race_rejection(&create_result) && snapshot.subtasks.is_empty();
        complete_result.is_ok()
            && (create_then_cascade || complete_then_reject_create)
            && snapshot.balance == 1
            && reward_recorded
    } else {
        false
    };
    let child_created_outcome = if parent.status == TaskStatus::Pending {
        let created_task_id = create_result
            .as_ref()
            .ok()
            .map(|receipt| receipt.task_id.as_str());
        create_result.is_ok()
            && is_expected_race_rejection(&complete_result)
            && snapshot.balance == 0
            && snapshot.rewards.is_empty()
            && snapshot
                .subtasks
                .iter()
                .filter(|task| task.parent_task_id.as_deref() == Some(parent_task_id.as_str()))
                .count()
                == 1
            && snapshot.subtasks.iter().any(|task| {
                Some(task.id.as_str()) == created_task_id
                    && task.parent_task_id.as_deref() == Some(parent_task_id.as_str())
                    && task.status == TaskStatus::Pending
                    && task.sibling_position == Some(1)
            })
    } else {
        false
    };
    assert!(
        parent_completed_outcome || child_created_outcome,
        "竞争后只能是父项已完成且已吸收先提交子项，或父项待办且新增一个子项"
    );
    assert!(store.verify_integrity().expect("应校验竞争账本").is_ok());
    drop(store);
    remove_database_family(&path);
}

#[test]
fn ledger_smoke_serializes_child_undo_against_parent_completion() {
    let path = temporary_database_path("concurrent-undo-child-complete-parent");
    let (parent_task_id, child_task_id, child_completion_event_id) = {
        let store = SqliteLedgerStore::open(&path).expect("应建立撤销与完成竞争账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let parent = service
            .capture_task("capture-undo-complete-parent", "撤销竞争父代办")
            .expect("应创建父项");
        let child = service
            .create_subtask(
                "create-undo-complete-child",
                &parent.task_id,
                "撤销竞争子代办",
            )
            .expect("应创建子项");
        let completion = service
            .complete_task("complete-undo-complete-child", &child.task_id)
            .expect("应先完成子项");
        assert!(completion.reward_transaction_id.is_none());
        (parent.task_id, child.task_id, completion.event_id)
    };

    let barrier = Arc::new(Barrier::new(3));
    let complete_path = path.clone();
    let complete_parent_id = parent_task_id.clone();
    let complete_barrier = Arc::clone(&barrier);
    let complete_handle = std::thread::spawn(move || {
        let store = SqliteLedgerStore::open(&complete_path).expect("完成线程应打开同一账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        complete_barrier.wait();
        service.complete_task(
            "concurrent-complete-parent-after-child",
            &complete_parent_id,
        )
    });
    let undo_path = path.clone();
    let undo_completion_id = child_completion_event_id.clone();
    let undo_barrier = Arc::clone(&barrier);
    let undo_handle = std::thread::spawn(move || {
        let store = SqliteLedgerStore::open(&undo_path).expect("撤销线程应打开同一账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        undo_barrier.wait();
        service.undo_completion("concurrent-undo-child", &undo_completion_id)
    });
    barrier.wait();

    let complete_result = complete_handle.join().expect("完成线程不应恐慌");
    let undo_result = undo_handle.join().expect("撤销线程不应恐慌");
    assert_eq!(
        [complete_result.is_ok(), undo_result.is_ok()]
            .into_iter()
            .filter(|succeeded| *succeeded)
            .count(),
        1,
        "撤销子项与完成父项只能有一个提交成功"
    );

    let mut store = SqliteLedgerStore::open(&path).expect("应重开撤销与完成竞争账本");
    let parent = store
        .task_by_id(&parent_task_id)
        .expect("应读取父项")
        .expect("父项应存在");
    let child = store
        .task_by_id(&child_task_id)
        .expect("应读取子项")
        .expect("子项应存在");
    let snapshot = store.snapshot().expect("应读取竞争结果");
    let parent_completed_outcome = if parent.status == TaskStatus::Completed {
        let reward_recorded = complete_result
            .as_ref()
            .ok()
            .and_then(|receipt| {
                receipt.reward_transaction_id.as_deref().map(|reward_id| {
                    snapshot.rewards.iter().any(|reward| {
                        reward.id == reward_id
                            && reward.task_event_id == receipt.event_id
                            && reward.reward_type == RewardType::TaskCompletion
                            && reward.amount == 1
                    })
                })
            })
            .unwrap_or(false);
        child.status == TaskStatus::Completed
            && child.active_completion_event_id.as_deref()
                == Some(child_completion_event_id.as_str())
            && complete_result.is_ok()
            && is_expected_race_rejection(&undo_result)
            && snapshot.balance == 1
            && reward_recorded
    } else {
        false
    };
    let child_undone_outcome = if parent.status == TaskStatus::Pending {
        let undo_event_recorded = undo_result.as_ref().ok().is_some_and(|receipt| {
            receipt.reward_transaction_id.is_none()
                && snapshot.events.iter().any(|event| {
                    event.id == receipt.event_id
                        && event.event_type == TaskEventType::SubtaskCompletionUndone
                        && event.reverses_event_id.as_deref()
                            == Some(child_completion_event_id.as_str())
                })
        });
        child.status == TaskStatus::Pending
            && child.active_completion_event_id.is_none()
            && undo_result.is_ok()
            && is_expected_race_rejection(&complete_result)
            && snapshot.balance == 0
            && snapshot.rewards.is_empty()
            && undo_event_recorded
    } else {
        false
    };
    assert!(
        parent_completed_outcome || child_undone_outcome,
        "竞争后父子只能同时保持已完成，或同时回到待办"
    );
    assert!(store.verify_integrity().expect("应校验竞争账本").is_ok());
    drop(store);
    remove_database_family(&path);
}

#[test]
fn ledger_smoke_keeps_projection_consistent_when_reorder_races_child_delete() {
    let path = temporary_database_path("concurrent-reorder-delete-child");
    let (parent_task_id, child_a_id, child_b_id) = {
        let store = SqliteLedgerStore::open(&path).expect("应建立重排与删除竞争账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let parent = service
            .capture_task("capture-reorder-delete-parent", "重排竞争父代办")
            .expect("应创建父项");
        let child_a = service
            .create_subtask(
                "create-reorder-delete-child-a",
                &parent.task_id,
                "重排竞争子代办 A",
            )
            .expect("应创建子项 A");
        let child_b = service
            .create_subtask(
                "create-reorder-delete-child-b",
                &parent.task_id,
                "重排竞争子代办 B",
            )
            .expect("应创建子项 B");
        (parent.task_id, child_a.task_id, child_b.task_id)
    };

    let barrier = Arc::new(Barrier::new(3));
    let reorder_path = path.clone();
    let reorder_parent_id = parent_task_id.clone();
    let reorder_child_a_id = child_a_id.clone();
    let reorder_child_b_id = child_b_id.clone();
    let reorder_barrier = Arc::clone(&barrier);
    let reorder_handle = std::thread::spawn(move || {
        let store = SqliteLedgerStore::open(&reorder_path).expect("重排线程应打开同一账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        reorder_barrier.wait();
        service.reorder_subtasks(
            "concurrent-reorder-children",
            &reorder_parent_id,
            &reorder_child_b_id,
            &[reorder_child_a_id.clone(), reorder_child_b_id.clone()],
            &[reorder_child_b_id.clone(), reorder_child_a_id.clone()],
        )
    });
    let delete_path = path.clone();
    let delete_child_id = child_a_id.clone();
    let delete_barrier = Arc::clone(&barrier);
    let delete_handle = std::thread::spawn(move || {
        let store = SqliteLedgerStore::open(&delete_path).expect("删除线程应打开同一账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        delete_barrier.wait();
        service.delete_task("concurrent-delete-child", &delete_child_id)
    });
    barrier.wait();

    let reorder_result = reorder_handle.join().expect("重排线程不应恐慌");
    let delete_result = delete_handle.join().expect("删除线程不应恐慌");
    assert!(
        reorder_result.is_ok() || delete_result.is_ok(),
        "重排和删除至少应有一个提交成功"
    );
    assert!(
        reorder_result.is_ok() || is_expected_race_rejection(&reorder_result),
        "重排失败只能是预期的状态或并发拒绝"
    );
    assert!(
        delete_result.is_ok() || is_expected_race_rejection(&delete_result),
        "删除失败只能是预期的状态或并发拒绝"
    );

    let mut store = SqliteLedgerStore::open(&path).expect("应重开重排与删除竞争账本");
    let parent = store
        .task_by_id(&parent_task_id)
        .expect("应读取父项")
        .expect("父项应存在");
    let child_a = store
        .task_by_id(&child_a_id)
        .expect("应读取子项 A")
        .expect("子项 A 应存在");
    let child_b = store
        .task_by_id(&child_b_id)
        .expect("应读取子项 B")
        .expect("子项 B 应存在");
    let snapshot = store.snapshot().expect("应读取竞争结果");
    let active_order: Vec<&str> = snapshot
        .subtasks
        .iter()
        .filter(|task| task.parent_task_id.as_deref() == Some(parent_task_id.as_str()))
        .map(|task| task.id.as_str())
        .collect();
    let successful_receipts_are_reward_free = [&reorder_result, &delete_result]
        .into_iter()
        .filter_map(|result| result.as_ref().ok())
        .all(|receipt| receipt.reward_transaction_id.is_none());
    let reorder_event_consistent = reorder_result.as_ref().ok().is_none_or(|receipt| {
        snapshot.events.iter().any(|event| {
            event.id == receipt.event_id && event.event_type == TaskEventType::SubtasksReordered
        })
    });
    let delete_event_consistent = delete_result.as_ref().ok().is_none_or(|receipt| {
        snapshot.events.iter().any(|event| {
            event.id == receipt.event_id && event.event_type == TaskEventType::Abandoned
        })
    });
    let reordered_outcome = child_a.status == TaskStatus::Pending
        && child_b.status == TaskStatus::Pending
        && child_a.sibling_position == Some(2)
        && child_b.sibling_position == Some(1)
        && active_order == vec![child_b_id.as_str(), child_a_id.as_str()]
        && reorder_result.is_ok()
        && is_expected_race_rejection(&delete_result);
    let deleted_outcome = if child_a.status == TaskStatus::Abandoned
        && child_b.status == TaskStatus::Pending
        && active_order == vec![child_b_id.as_str()]
        && delete_result.is_ok()
    {
        let mut positions = vec![
            child_a.sibling_position.expect("已删除子项仍应保留位置"),
            child_b.sibling_position.expect("保留子项应有位置"),
        ];
        positions.sort_unstable();
        positions == vec![1, 2]
    } else {
        false
    };
    assert_eq!(parent.status, TaskStatus::Pending);
    assert_eq!(snapshot.balance, 0);
    assert!(snapshot.rewards.is_empty());
    assert!(successful_receipts_are_reward_free);
    assert!(reorder_event_consistent);
    assert!(delete_event_consistent);
    assert!(
        reordered_outcome || deleted_outcome,
        "竞争后只能保留重排结果，或保留一致的软删除投影"
    );
    assert!(store.verify_integrity().expect("应校验竞争账本").is_ok());
    drop(store);
    remove_database_family(&path);
}

#[test]
fn ledger_smoke_serializes_balance_and_tail_for_different_tasks() {
    let path = temporary_database_path("cross-task-concurrent");
    let (task_a_id, task_b_id, task_c_id, completion_a_id) = {
        let store = SqliteLedgerStore::open(&path).expect("应建立跨任务并发账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let task_a = service
            .capture_task("capture-cross-a", "任务 A")
            .expect("应创建 A");
        let task_b = service
            .capture_task("capture-cross-b", "任务 B")
            .expect("应创建 B");
        let task_c = service
            .capture_task("capture-cross-c", "任务 C")
            .expect("应创建 C");
        let completion_a = service
            .complete_task("complete-cross-a", &task_a.task_id)
            .expect("应先完成 A");
        (
            task_a.task_id,
            task_b.task_id,
            task_c.task_id,
            completion_a.event_id,
        )
    };

    let mut complete_store = SqliteLedgerStore::open(&path).expect("完成连接应打开");
    let complete_task = complete_store
        .task_by_id(&task_b_id)
        .expect("应读取 B")
        .expect("B 应存在");
    let complete_mutation = complete_task_transition(
        &complete_task,
        None,
        &[],
        Vec::new(),
        MutationContext {
            command_id: "complete-cross-b".to_string(),
            event_id: uuid::Uuid::new_v4().to_string(),
            reward_transaction_id: Some(uuid::Uuid::new_v4().to_string()),
            occurred_at_ms: SystemClock.now_ms(),
        },
    )
    .expect("应生成完成 B 的领域变化");

    let mut undo_store = SqliteLedgerStore::open(&path).expect("撤销连接应打开");
    let undo_event = undo_store
        .event_by_id(&completion_a_id)
        .expect("应读取 A 完成事件")
        .expect("A 完成事件应存在");
    let undo_task = undo_store
        .task_by_id(&task_a_id)
        .expect("应读取 A")
        .expect("A 应存在");
    let stale_balance = undo_store.reward_balance().expect("应读取初始余额");
    assert_eq!(stale_balance, 1);
    let undo_mutation = undo_completion_transition(
        &undo_task,
        None,
        &undo_event,
        MutationContext {
            command_id: "undo-cross-a".to_string(),
            event_id: uuid::Uuid::new_v4().to_string(),
            reward_transaction_id: Some(uuid::Uuid::new_v4().to_string()),
            occurred_at_ms: SystemClock.now_ms(),
        },
        stale_balance,
    )
    .expect("应生成撤销 A 的领域变化");

    let barrier = Arc::new(Barrier::new(3));
    let complete_barrier = Arc::clone(&barrier);
    let complete_handle = std::thread::spawn(move || {
        complete_barrier.wait();
        complete_store.commit_transition(
            "complete_task",
            "cross-complete-fingerprint",
            complete_mutation,
        )
    });
    let undo_barrier = Arc::clone(&barrier);
    let undo_handle = std::thread::spawn(move || {
        undo_barrier.wait();
        undo_store.commit_transition("undo_completion", "cross-undo-fingerprint", undo_mutation)
    });
    barrier.wait();
    complete_handle
        .join()
        .expect("完成线程不应恐慌")
        .expect("完成 B 应提交");
    undo_handle
        .join()
        .expect("撤销线程不应恐慌")
        .expect("撤销 A 应提交");

    {
        let store = SqliteLedgerStore::open(&path).expect("应重开跨任务并发账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let snapshot = service.snapshot().expect("应读取并发后快照");
        assert_eq!(snapshot.balance, 1);
        assert_eq!(snapshot.rewards.len(), 3);
        assert_eq!(snapshot.events.len(), 6);
        assert_eq!(
            snapshot.current_task.as_ref().map(|task| task.id.as_str()),
            Some(task_c_id.as_str())
        );
        assert_eq!(
            snapshot.queue.last().map(|task| task.id.as_str()),
            Some(task_a_id.as_str())
        );
        assert!(service.verify_integrity().expect("应校验并发账本").is_ok());
    }
    remove_database_family(&path);
}

#[test]
fn ledger_smoke_serializes_two_first_opens() {
    let path = temporary_database_path("concurrent-first-open");
    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let thread_path = path.clone();
        let thread_barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            thread_barrier.wait();
            SqliteLedgerStore::open(&thread_path)
        }));
    }
    barrier.wait();
    for handle in handles {
        drop(
            handle
                .join()
                .expect("首次打开线程不应恐慌")
                .expect("并发首次打开都应成功"),
        );
    }
    let mut store = SqliteLedgerStore::open(&path).expect("并发初始化后应能重开");
    assert!(store.verify_integrity().expect("应校验初始化结果").is_ok());
    drop(store);
    remove_database_family(&path);
}

#[test]
fn ledger_smoke_rejects_unknown_schema_without_replacing_file() {
    let path = temporary_database_path("future-schema");
    drop(SqliteLedgerStore::open(&path).expect("应建立当前版本账本"));
    {
        let connection = rusqlite::Connection::open(&path).expect("应打开账本设置版本");
        connection
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .expect("应写入模拟的新版本");
    }
    let error = match SqliteLedgerStore::open(&path) {
        Ok(_) => panic!("未知新版本必须拒绝打开"),
        Err(error) => error,
    };
    assert_eq!(error.code(), "UNSUPPORTED_SCHEMA_VERSION");
    let connection = rusqlite::Connection::open(&path).expect("原文件必须仍可直接读取");
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("版本值必须保留");
    assert_eq!(version, SCHEMA_VERSION + 1);
    drop(connection);
    remove_database_family(&path);
}

#[test]
fn ledger_smoke_refuses_to_take_over_unidentified_sqlite_file() {
    let path = temporary_database_path("foreign-sqlite");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("应创建测试目录");
    }
    {
        let connection = rusqlite::Connection::open(&path).expect("应创建其他 SQLite 文件");
        connection
            .execute("CREATE TABLE other_app_data (id INTEGER PRIMARY KEY)", [])
            .expect("应创建其他应用表");
    }
    let error = match SqliteLedgerStore::open(&path) {
        Ok(_) => panic!("不得接管无标识的其他 SQLite 文件"),
        Err(error) => error,
    };
    assert_eq!(error.code(), "DATA_INTEGRITY_ERROR");
    let connection = rusqlite::Connection::open(&path).expect("原文件必须保留");
    let table_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE name = 'other_app_data'",
            [],
            |row| row.get(0),
        )
        .expect("应读取其他应用表");
    assert_eq!(table_count, 1);
    drop(connection);
    remove_database_family(&path);
}

#[test]
fn ledger_smoke_rejects_corrupted_reward_projection_without_empty_fallback() {
    let path = temporary_database_path("corrupted-ledger");
    {
        let store = SqliteLedgerStore::open(&path).expect("应建立账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let task = service
            .capture_task("capture-corrupt", "校验损坏数据")
            .expect("应创建任务");
        service
            .complete_task("complete-corrupt", &task.task_id)
            .expect("应完成任务");
    }
    {
        let connection = rusqlite::Connection::open(&path).expect("应直接打开模拟损坏");
        connection
            .execute("UPDATE reward_transactions SET balance_after = 9", [])
            .expect("应写入可被领域校验识别的不一致数据");
    }
    let error = match SqliteLedgerStore::open(&path) {
        Ok(_) => panic!("损坏账本必须拒绝打开"),
        Err(error) => error,
    };
    assert_eq!(error.code(), "DATA_INTEGRITY_ERROR");

    let connection = rusqlite::Connection::open(&path).expect("原文件必须仍然存在");
    let stored_balance: i64 = connection
        .query_row(
            "SELECT balance_after FROM reward_transactions LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("损坏记录不应被空库覆盖");
    assert_eq!(stored_balance, 9);
    drop(connection);
    remove_database_family(&path);
}

#[test]
fn ledger_smoke_rejects_orphan_events_detected_by_foreign_key_check() {
    let path = temporary_database_path("orphan-event");
    let task_id = {
        let store = SqliteLedgerStore::open(&path).expect("应建立账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let task = service
            .capture_task("capture-orphan", "制造孤立事件")
            .expect("应创建任务");
        service
            .complete_task("complete-orphan", &task.task_id)
            .expect("应完成任务");
        task.task_id
    };
    {
        let connection = rusqlite::Connection::open(&path).expect("应直接打开模拟损坏");
        connection
            .pragma_update(None, "foreign_keys", "OFF")
            .expect("应关闭外键保护");
        connection
            .execute("DELETE FROM tasks WHERE id = ?1", [&task_id])
            .expect("应删除任务制造孤立事件");
    }
    let error = match SqliteLedgerStore::open(&path) {
        Ok(_) => panic!("孤立事件必须让账本拒绝打开"),
        Err(error) => error,
    };
    assert_eq!(error.code(), "DATA_INTEGRITY_ERROR");
    let connection = rusqlite::Connection::open(&path).expect("原损坏文件必须保留");
    let event_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM task_events", [], |row| row.get(0))
        .expect("孤立事件不应被空库覆盖");
    assert_eq!(event_count, 2);
    drop(connection);
    remove_database_family(&path);
}

#[test]
fn ledger_smoke_rejects_tampered_receipt_json_without_replacing_file() {
    let path = temporary_database_path("tampered-receipt");
    {
        let store = SqliteLedgerStore::open(&path).expect("应建立账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let task = service
            .capture_task("capture-receipt", "校验损坏回执")
            .expect("应创建任务");
        service
            .complete_task("complete-receipt", &task.task_id)
            .expect("应完成任务");
    }
    {
        let connection = rusqlite::Connection::open(&path).expect("应直接打开模拟损坏");
        connection
            .execute(
                "UPDATE command_receipts
                 SET result_json = json_set(
                     result_json, '$.commandId', 'tampered-command', '$.balance', 99
                 )
                 WHERE command_id = 'complete-receipt'",
                [],
            )
            .expect("应篡改仍然合法的回执 JSON");
    }
    let error = match SqliteLedgerStore::open(&path) {
        Ok(_) => panic!("损坏回执必须让账本拒绝打开"),
        Err(error) => error,
    };
    assert_eq!(error.code(), "DATA_INTEGRITY_ERROR");

    let connection = rusqlite::Connection::open(&path).expect("原损坏文件必须保留");
    let stored_command_id: String = connection
        .query_row(
            "SELECT json_extract(result_json, '$.commandId')
             FROM command_receipts WHERE command_id = 'complete-receipt'",
            [],
            |row| row.get(0),
        )
        .expect("篡改后的回执不应被空库覆盖");
    assert_eq!(stored_command_id, "tampered-command");
    drop(connection);
    remove_database_family(&path);
}

#[test]
fn ledger_smoke_rejects_tampered_parent_completion_cascade_links() {
    for (label, create_child, expected_event_count, tamper_sql) in [
        (
            "cascade-child-parent-link",
            true,
            4,
            "UPDATE task_events
             SET metadata_json = json_set(
                 metadata_json, '$.cascadeParentEventId', 'missing-parent-event'
             )
             WHERE command_id GLOB 'cascade/*'",
        ),
        (
            "cascade-child-command",
            true,
            4,
            "UPDATE task_events
             SET command_id = 'cascade/wrong-event-id'
             WHERE command_id GLOB 'cascade/*'",
        ),
        (
            "cascade-parent-index-type",
            false,
            2,
            "UPDATE task_events
             SET metadata_json = json_set(
                 metadata_json, '$.cascadeSubtaskEventIds', 'not-an-array'
             )
             WHERE event_type = 'completed'",
        ),
    ] {
        let path = temporary_database_path(label);
        {
            let store = SqliteLedgerStore::open(&path).expect("应建立级联篡改测试账本");
            let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
            let parent = service
                .capture_task("cascade-tamper-parent", "检查级联完整性")
                .expect("应创建父代办");
            if create_child {
                service
                    .create_subtask("cascade-tamper-child", &parent.task_id, "检查子事件")
                    .expect("应创建子代办");
            }
            service
                .complete_task("cascade-tamper-complete", &parent.task_id)
                .expect("应原子完成父子代办");
            assert!(service
                .verify_integrity()
                .expect("篡改前应通过完整性检查")
                .is_ok());
        }
        {
            let connection = rusqlite::Connection::open(&path).expect("应直接打开模拟级联关联损坏");
            connection
                .execute(tamper_sql, [])
                .expect("应篡改级联完成关联");
        }

        let error = match SqliteLedgerStore::open(&path) {
            Ok(_) => panic!("级联完成关联损坏必须拒绝打开：{label}"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "DATA_INTEGRITY_ERROR", "篡改类型：{label}");
        let connection = rusqlite::Connection::open(&path).expect("原损坏文件必须保留");
        let event_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM task_events", [], |row| row.get(0))
            .expect("应读取损坏后的事件数量");
        assert_eq!(event_count, expected_event_count, "篡改类型：{label}");
        drop(connection);
        remove_database_family(&path);
    }
}

#[test]
fn ledger_smoke_rejects_tampered_title_projection_metadata_and_receipt_mapping() {
    for (label, tamper_sql) in [
        (
            "title-projection",
            "UPDATE tasks SET title = '篡改后的投影标题' WHERE title = '新标题'",
        ),
        (
            "title-metadata",
            "UPDATE task_events
             SET metadata_json = '{\"beforeTitle\":\"错误旧标题\",\"afterTitle\":\"新标题\"}'
             WHERE event_type = 'title_updated'",
        ),
        (
            "title-receipt-type",
            "UPDATE command_receipts
             SET command_type = 'capture_task'
             WHERE command_type = 'update_task_title'",
        ),
    ] {
        let path = temporary_database_path(label);
        {
            let store = SqliteLedgerStore::open(&path).expect("应建立标题篡改测试账本");
            let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
            let task = service
                .capture_task("capture-title-tamper", "旧标题")
                .expect("应创建任务");
            service
                .update_task_title("update-title-tamper", &task.task_id, "新标题")
                .expect("应修改标题");
        }
        {
            let connection = rusqlite::Connection::open(&path).expect("应直接打开模拟损坏");
            connection
                .execute_batch(tamper_sql)
                .expect("应写入结构合法但语义错误的篡改");
        }

        let error = match SqliteLedgerStore::open(&path) {
            Ok(_) => panic!("标题账本篡改必须拒绝打开：{label}"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "DATA_INTEGRITY_ERROR", "篡改类型：{label}");
        let connection = rusqlite::Connection::open(&path).expect("原损坏文件必须保留");
        let event_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM task_events", [], |row| row.get(0))
            .expect("篡改后的事件不应被空库覆盖");
        assert_eq!(event_count, 2, "篡改类型：{label}");
        drop(connection);
        remove_database_family(&path);
    }
}

#[test]
fn ledger_smoke_rejects_tampered_deadline_projection_metadata_and_receipt_mapping() {
    for (label, tamper_sql) in [
        (
            "deadline-projection",
            "UPDATE tasks SET deadline_on = '2026-07-21' WHERE deadline_on = '2026-07-20'",
        ),
        (
            "deadline-invalid-projection",
            "UPDATE tasks SET deadline_on = '2026-02-30' WHERE deadline_on = '2026-07-20'",
        ),
        (
            "deadline-metadata",
            "UPDATE task_events
             SET metadata_json = '{\"beforeDeadlineOn\":\"2026-07-01\",\"afterDeadlineOn\":\"2026-07-20\"}'
             WHERE event_type = 'deadline_updated'",
        ),
        (
            "deadline-metadata-extra-field",
            "UPDATE task_events
             SET metadata_json = '{\"beforeDeadlineOn\":null,\"afterDeadlineOn\":\"2026-07-20\",\"unexpected\":true}'
             WHERE event_type = 'deadline_updated'",
        ),
        (
            "deadline-metadata-missing-field",
            "UPDATE task_events
             SET metadata_json = '{\"afterDeadlineOn\":\"2026-07-20\"}'
             WHERE event_type = 'deadline_updated'",
        ),
        (
            "deadline-receipt-type",
            "UPDATE command_receipts
             SET command_type = 'capture_task'
             WHERE command_type = 'update_task_deadline'",
        ),
    ] {
        let path = temporary_database_path(label);
        {
            let store = SqliteLedgerStore::open(&path).expect("应建立截止日期篡改测试账本");
            let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
            let task = service
                .capture_task("capture-deadline-tamper", "任务")
                .expect("应创建任务");
            service
                .update_task_deadline(
                    "update-deadline-tamper",
                    &task.task_id,
                    Some("2026-07-20"),
                )
                .expect("应设置截止日期");
        }
        {
            let connection = rusqlite::Connection::open(&path).expect("应直接打开模拟损坏");
            connection
                .execute_batch(tamper_sql)
                .expect("应写入结构合法但语义错误的篡改");
        }

        let error = match SqliteLedgerStore::open(&path) {
            Ok(_) => panic!("截止日期账本篡改必须拒绝打开：{label}"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "DATA_INTEGRITY_ERROR", "篡改类型：{label}");
        let connection = rusqlite::Connection::open(&path).expect("原损坏文件必须保留");
        let event_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM task_events", [], |row| row.get(0))
            .expect("篡改后的事件不应被空库覆盖");
        assert_eq!(event_count, 2, "篡改类型：{label}");
        drop(connection);
        remove_database_family(&path);
    }
}

#[test]
fn ledger_smoke_rejects_task_without_created_event_history() {
    let path = temporary_database_path("task-without-history");
    drop(SqliteLedgerStore::open(&path).expect("应建立账本结构"));
    {
        let connection = rusqlite::Connection::open(&path).expect("应直接打开模拟损坏");
        let now_ms = SystemClock.now_ms();
        connection
            .execute(
                "INSERT INTO tasks (
                    id, title, status, queue_position, version,
                    created_at_ms, updated_at_ms
                 ) VALUES ('historyless-task', '没有创建历史', 'pending', 1, 1, ?1, ?1)",
                [now_ms],
            )
            .expect("应制造缺少创建事件的任务投影");
    }
    let error = match SqliteLedgerStore::open(&path) {
        Ok(_) => panic!("没有创建历史的任务必须让账本拒绝打开"),
        Err(error) => error,
    };
    assert_eq!(error.code(), "DATA_INTEGRITY_ERROR");

    let connection = rusqlite::Connection::open(&path).expect("原损坏文件必须保留");
    let task_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))
        .expect("损坏任务不应被空库覆盖");
    assert_eq!(task_count, 1);
    drop(connection);
    remove_database_family(&path);
}

#[test]
fn ledger_snapshot_excludes_deferred_task_from_current_queue() {
    let path = temporary_database_path("deferred-current");
    let (deferred_task_id, current_task_id) = {
        let store = SqliteLedgerStore::open(&path).expect("应建立账本");
        let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
        let deferred_task_id = service
            .capture_task("capture-deferred", "首个任务稍后再做")
            .expect("应创建待延期任务")
            .task_id;
        let current_task_id = service
            .capture_task("capture-current", "延期后的当前任务")
            .expect("应创建当前任务")
            .task_id;
        (deferred_task_id, current_task_id)
    };
    {
        let mut connection = rusqlite::Connection::open(&path).expect("应打开账本模拟延期命令");
        let now_ms = SystemClock.now_ms();
        let transaction = connection.transaction().expect("应开始延期模拟事务");
        transaction
            .execute(
                "UPDATE tasks
                 SET queue_position = NULL, defer_until_ms = ?2,
                     version = version + 1, updated_at_ms = ?3
                 WHERE id = ?1",
                rusqlite::params![deferred_task_id, now_ms + 60_000, now_ms],
            )
            .expect("应更新延期任务投影");
        transaction
            .execute(
                "INSERT INTO task_events (
                    id, command_id, task_id, title_snapshot, event_type,
                    occurred_at_ms, reason, metadata_json, reverses_event_id
                 ) VALUES (
                    'defer-event', 'defer-current', ?1, ?2, 'deferred',
                    ?3, '稍后处理', '{}', NULL
                 )",
                rusqlite::params![deferred_task_id, "首个任务稍后再做", now_ms],
            )
            .expect("应追加延期事件");
        let receipt = StoredReceipt {
            command_id: "defer-current".to_string(),
            task_id: deferred_task_id.clone(),
            event_id: "defer-event".to_string(),
            reward_transaction_id: None,
            current_task_id: Some(current_task_id.clone()),
            balance: 0,
        };
        let result_json = serde_json::to_string(&receipt).expect("应序列化延期命令回执");
        let request_fingerprint =
            serde_json::to_string(&("defer_task", [deferred_task_id.as_str(), "60000"]))
                .expect("应序列化延期命令指纹");
        transaction
            .execute(
                "INSERT INTO command_receipts (
                    command_id, command_type, request_fingerprint,
                    result_json, created_at_ms
                 ) VALUES ('defer-current', 'defer_task', ?1, ?2, ?3)",
                rusqlite::params![request_fingerprint, result_json, now_ms],
            )
            .expect("应保存延期命令回执");
        transaction.commit().expect("应提交完整延期事实");
    }

    let store = SqliteLedgerStore::open(&path).expect("包含延期任务的账本应能重开");
    let mut service = TaskService::new(store, SystemClock, UuidIdGenerator);
    let snapshot = service.snapshot().expect("应读取延期任务快照");
    assert_eq!(snapshot.queue.len(), 1);
    assert_eq!(snapshot.queue[0].id, current_task_id);
    assert_eq!(
        snapshot.current_task.as_ref().map(|task| task.id.as_str()),
        Some(current_task_id.as_str())
    );
    assert!(snapshot
        .queue
        .iter()
        .all(|task| task.id != deferred_task_id));
    assert!(service.verify_integrity().expect("应校验延期投影").is_ok());
    drop(service);
    remove_database_family(&path);
}

fn temporary_database_path(label: &str) -> PathBuf {
    std::env::temp_dir()
        .join(format!(
            "zuoban-ledger-test-{label}-{}",
            uuid::Uuid::new_v4()
        ))
        .join("ledger.sqlite3")
}

fn is_expected_race_rejection<T>(result: &Result<T, LedgerError>) -> bool {
    matches!(
        result,
        Err(error) if matches!(error.code(), "INVALID_TASK_STATE" | "CONCURRENT_MODIFICATION")
    )
}

fn remove_database_family(path: &Path) {
    if let Some(parent) = path.parent() {
        std::fs::remove_dir_all(parent)
            .unwrap_or_else(|error| panic!("清理测试账本目录 {} 失败：{error}", parent.display()));
    }
}
