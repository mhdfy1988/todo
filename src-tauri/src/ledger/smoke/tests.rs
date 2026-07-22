use super::*;
use crate::ledger::{
    domain::{
        complete_task_transition, undo_completion_transition, MutationContext, StoredReceipt,
        TaskEventType, TaskStatus,
    },
    service::LedgerStore,
    sqlite::{FailurePoint, SCHEMA_VERSION},
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};

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
fn ledger_smoke_backs_up_and_migrates_a_populated_v1_ledger_to_v4() {
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
        let store = SqliteLedgerStore::open(&path).expect("真实 v1 文件应备份并升级到 v4");
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
                .is_some_and(|name| name.starts_with("ledger.before-v4."))
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
fn ledger_smoke_backs_up_and_migrates_a_populated_v2_ledger_to_v4() {
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
        let store = SqliteLedgerStore::open(&path).expect("真实 v2 文件应备份并升级到 v4");
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
                .is_some_and(|name| name.starts_with("ledger.before-v4."))
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
fn ledger_smoke_backs_up_and_migrates_a_populated_v3_ledger_to_v4() {
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
        let store = SqliteLedgerStore::open(&path).expect("真实 v3 文件应备份并升级到 v4");
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
                .is_some_and(|name| name.starts_with("ledger.before-v4."))
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
             ALTER TABLE tasks DROP COLUMN deadline_on;
             DELETE FROM schema_migrations;
            INSERT INTO schema_migrations (version, description, applied_at_ms)
                VALUES (1, '真实 v1 迁移测试夹具', 1);",
        )
        .expect("应重建 v1 事件约束");
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
             ALTER TABLE tasks DROP COLUMN deadline_on;
             DELETE FROM schema_migrations;
            INSERT INTO schema_migrations (version, description, applied_at_ms)
                VALUES (2, '真实 v2 迁移测试夹具', 2);",
        )
        .expect("应重建 v2 事件约束");
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
            ALTER TABLE tasks DROP COLUMN deadline_on;
            DELETE FROM schema_migrations;
            INSERT INTO schema_migrations (version, description, applied_at_ms)
                VALUES (3, '真实 v3 迁移测试夹具', 3);",
        )
        .expect("应重建 v3 事件与任务约束");
    transaction
        .pragma_update(None, "user_version", 3)
        .expect("应把夹具标记为 v3");
    transaction.commit().expect("应提交 v3 测试夹具");
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

fn remove_database_family(path: &Path) {
    if let Some(parent) = path.parent() {
        std::fs::remove_dir_all(parent)
            .unwrap_or_else(|error| panic!("清理测试账本目录 {} 失败：{error}", parent.display()));
    }
}
