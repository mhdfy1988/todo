use super::{
    domain::{
        Clock, IntegrityReport, LedgerError, LedgerMutation, LedgerSnapshot, MutationReceipt,
        RewardMutation, RewardTransaction, RewardType, StoredReceipt, SystemClock, Task, TaskEvent,
        TaskEventType, TaskStatus, TaskWrite, WeeklyFacts,
    },
    service::{stored_receipt_from_json, LedgerStore},
};
use rusqlite::{
    params, types::Type, Connection, OptionalExtension, Row, Transaction, TransactionBehavior,
};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

mod integrity;
mod schema;

use integrity::verify_integrity_in;
use schema::migrate;

pub const SCHEMA_VERSION: i64 = 4;

const TASK_SELECT: &str = "SELECT id, title, status, queue_position, defer_until_ms, deadline_on, \
    block_reason, abandon_reason, completed_at_ms, active_completion_event_id, version, \
    created_at_ms, updated_at_ms FROM tasks";
const EVENT_SELECT: &str = "SELECT sequence, id, command_id, task_id, title_snapshot, \
    event_type, occurred_at_ms, reason, metadata_json, reverses_event_id FROM task_events";
const REWARD_SELECT: &str = "SELECT sequence, id, task_event_id, reward_type, amount, \
    balance_after, occurred_at_ms FROM reward_transactions";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailurePoint {
    AfterTaskWrite,
    AfterEventAppend,
    AfterRewardAppend,
    BeforeCommit,
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
enum CommitInterruption {
    None,
    ReturnError(FailurePoint),
    ExitBeforeCommit(i32),
}

pub struct SqliteLedgerStore {
    connection: Connection,
    interruption: CommitInterruption,
}

impl SqliteLedgerStore {
    pub fn open(path: &Path) -> Result<Self, LedgerError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| LedgerError::storage(format!("创建账本目录失败：{error}")))?;
        }
        let connection =
            Connection::open(path).map_err(|error| storage_error("打开 SQLite 账本失败", error))?;
        Self::initialize(connection, true, Some(path))
    }

    pub(crate) fn open_in_memory() -> Result<Self, LedgerError> {
        let connection = Connection::open_in_memory()
            .map_err(|error| storage_error("打开内存 SQLite 账本失败", error))?;
        Self::initialize(connection, false, None)
    }

    fn initialize(
        mut connection: Connection,
        enable_wal: bool,
        database_path: Option<&Path>,
    ) -> Result<Self, LedgerError> {
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(|error| storage_error("设置 SQLite 等待时间失败", error))?;

        if let Some(path) = database_path {
            backup_database_before_migration(&connection, path)?;
        }

        connection
            .pragma_update(None, "foreign_keys", "OFF")
            .map_err(|error| storage_error("迁移前关闭 SQLite 外键失败", error))?;
        let migration_result = migrate(&mut connection);
        let foreign_keys_result = connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(|error| storage_error("启用 SQLite 外键失败", error));
        if let Err(error) = migration_result {
            let _ = foreign_keys_result;
            return Err(error);
        }
        foreign_keys_result?;
        if enable_wal {
            connection
                .pragma_update(None, "journal_mode", "WAL")
                .map_err(|error| storage_error("启用 SQLite WAL 失败", error))?;
        }
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(|error| storage_error("设置 SQLite 同步级别失败", error))?;

        let mut store = Self {
            connection,
            interruption: CommitInterruption::None,
        };
        let report = store.verify_integrity()?;
        if !report.is_ok() {
            return Err(LedgerError::integrity(format!(
                "账本打开校验失败：{}",
                report.failures.join("；")
            )));
        }
        Ok(store)
    }

    #[cfg(test)]
    pub(crate) fn inject_failure_once(&mut self, point: FailurePoint) {
        self.interruption = CommitInterruption::ReturnError(point);
    }

    #[cfg(debug_assertions)]
    pub(crate) fn exit_before_commit(&mut self, exit_code: i32) {
        self.interruption = CommitInterruption::ExitBeforeCommit(exit_code);
    }
}

fn backup_database_before_migration(
    connection: &Connection,
    database_path: &Path,
) -> Result<(), LedgerError> {
    let current_version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| storage_error("读取迁移前数据库版本失败", error))?;
    if current_version <= 0 || current_version >= SCHEMA_VERSION {
        return Ok(());
    }

    let backup_path = next_migration_backup_path(database_path)?;
    let backup_text = backup_path
        .to_str()
        .ok_or_else(|| LedgerError::storage("迁移前备份路径不是有效 UTF-8"))?;
    connection
        .execute("VACUUM INTO ?1", [backup_text])
        .map_err(|error| storage_error("创建账本迁移前备份失败", error))?;

    let backup =
        Connection::open_with_flags(&backup_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| storage_error("打开迁移前备份验证失败", error))?;
    let quick_check: String = backup
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|error| storage_error("校验迁移前备份失败", error))?;
    let backup_version: i64 = backup
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| storage_error("读取迁移前备份版本失败", error))?;
    if quick_check != "ok" || backup_version != current_version {
        return Err(LedgerError::integrity(format!(
            "迁移前备份校验失败：quick_check={quick_check}, user_version={backup_version}，预期版本={current_version}"
        )));
    }
    Ok(())
}

fn next_migration_backup_path(database_path: &Path) -> Result<PathBuf, LedgerError> {
    let parent = database_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = database_path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| LedgerError::storage("无法生成迁移前备份文件名"))?;
    let timestamp = SystemClock.now_ms();
    for suffix in 0..1000_i64 {
        let candidate = parent.join(format!(
            "{stem}.before-v{SCHEMA_VERSION}.{}.sqlite3",
            timestamp.saturating_add(suffix)
        ));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(LedgerError::storage(
        "无法为旧版账本找到可用的迁移前备份文件名",
    ))
}

impl LedgerStore for SqliteLedgerStore {
    fn replay_receipt(
        &self,
        command_id: &str,
        request_fingerprint: &str,
    ) -> Result<Option<MutationReceipt>, LedgerError> {
        replay_receipt_in(&self.connection, command_id, request_fingerprint)
    }

    fn queue(&self) -> Result<Vec<Task>, LedgerError> {
        queue_in(&self.connection)
    }

    fn task_by_id(&self, task_id: &str) -> Result<Option<Task>, LedgerError> {
        let sql = format!("{TASK_SELECT} WHERE id = ?1");
        self.connection
            .query_row(&sql, [task_id], map_task)
            .optional()
            .map_err(|error| storage_error("读取任务失败", error))
    }

    fn event_by_id(&self, event_id: &str) -> Result<Option<TaskEvent>, LedgerError> {
        let sql = format!("{EVENT_SELECT} WHERE id = ?1");
        self.connection
            .query_row(&sql, [event_id], map_event)
            .optional()
            .map_err(|error| storage_error("读取任务事件失败", error))
    }

    fn reward_balance(&self) -> Result<i64, LedgerError> {
        reward_balance_in(&self.connection)
    }

    fn commit_transition(
        &mut self,
        command_type: &str,
        request_fingerprint: &str,
        mut mutation: LedgerMutation,
    ) -> Result<MutationReceipt, LedgerError> {
        let interruption = self.interruption;
        self.interruption = CommitInterruption::None;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| storage_error("开始账本事务失败", error))?;

        if let Some(receipt) = replay_receipt_in(
            &transaction,
            &mutation.event.command_id,
            request_fingerprint,
        )? {
            transaction
                .rollback()
                .map_err(|error| storage_error("结束幂等重放事务失败", error))?;
            return Ok(receipt);
        }

        validate_transition_preconditions(&transaction, &mutation)?;
        materialize_queue_position(&transaction, &mut mutation.task_write)?;
        write_task(&transaction, &mutation.task_write)?;
        interrupt_if_needed(interruption, FailurePoint::AfterTaskWrite)?;

        insert_event(&transaction, &mutation.event)?;
        interrupt_if_needed(interruption, FailurePoint::AfterEventAppend)?;

        if let Some(reward) = &mutation.reward {
            let balance_before = reward_balance_in(&transaction)?;
            let balance_after = balance_before
                .checked_add(reward.amount)
                .ok_or_else(|| LedgerError::integrity("奖励余额计算发生整数溢出"))?;
            if balance_after < 0 {
                return Err(LedgerError::invalid_state(
                    "金币余额不足，无法原子提交这次奖励扣回",
                ));
            }
            insert_reward(&transaction, reward, balance_after)?;
        }
        interrupt_if_needed(interruption, FailurePoint::AfterRewardAppend)?;

        let receipt = StoredReceipt {
            command_id: mutation.event.command_id.clone(),
            task_id: mutation.task_id().to_string(),
            event_id: mutation.event.id.clone(),
            reward_transaction_id: mutation.reward.as_ref().map(|reward| reward.id.clone()),
            current_task_id: current_task_in(&transaction)?.map(|task| task.id),
            balance: reward_balance_in(&transaction)?,
        };
        let result_json = serde_json::to_string(&receipt)
            .map_err(|error| LedgerError::storage(format!("序列化命令回执失败：{error}")))?;
        transaction
            .execute(
                "INSERT INTO command_receipts (
                    command_id, command_type, request_fingerprint, result_json, created_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    receipt.command_id,
                    command_type,
                    request_fingerprint,
                    result_json,
                    mutation.event.occurred_at_ms
                ],
            )
            .map_err(|error| storage_error("写入命令回执失败", error))?;

        interrupt_if_needed(interruption, FailurePoint::BeforeCommit)?;
        transaction
            .commit()
            .map_err(|error| storage_error("提交账本事务失败", error))?;
        Ok(receipt.into_result(false))
    }

    fn snapshot(&mut self) -> Result<LedgerSnapshot, LedgerError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|error| storage_error("开始账本快照读事务失败", error))?;
        let completed_sql = format!(
            "{TASK_SELECT} WHERE status = 'completed' ORDER BY completed_at_ms DESC, id ASC"
        );
        let events_sql = format!("{EVENT_SELECT} ORDER BY sequence DESC LIMIT 100");
        let rewards_sql = format!("{REWARD_SELECT} ORDER BY sequence DESC LIMIT 100");

        let queue = queue_in(&transaction)?;
        let completed = query_tasks(&transaction, &completed_sql, [])?;
        let events = query_events(&transaction, &events_sql, [])?;
        let rewards = query_rewards(&transaction, &rewards_sql, [])?;
        let snapshot = LedgerSnapshot {
            schema_version: SCHEMA_VERSION,
            current_task: current_task_in(&transaction)?,
            queue,
            completed,
            events,
            rewards,
            balance: reward_balance_in(&transaction)?,
        };
        transaction
            .commit()
            .map_err(|error| storage_error("结束账本快照读事务失败", error))?;
        Ok(snapshot)
    }

    fn weekly_facts(&mut self, from_ms: i64, to_ms: i64) -> Result<WeeklyFacts, LedgerError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|error| storage_error("开始周报事实读事务失败", error))?;
        let completion_sql = format!(
            "{EVENT_SELECT} AS event
             WHERE event.event_type = 'completed'
               AND event.occurred_at_ms >= ?1
               AND event.occurred_at_ms < ?2
               AND NOT EXISTS (
                   SELECT 1 FROM task_events AS undo
                   WHERE undo.event_type = 'completion_undone'
                     AND undo.reverses_event_id = event.id
               )
             ORDER BY event.occurred_at_ms ASC, event.sequence ASC"
        );
        let ongoing_sql =
            format!("{TASK_SELECT} WHERE status = 'pending' ORDER BY queue_position ASC");
        let facts = WeeklyFacts {
            from_ms,
            to_ms,
            effective_completions: query_events(
                &transaction,
                &completion_sql,
                params![from_ms, to_ms],
            )?,
            ongoing_tasks: query_tasks(&transaction, &ongoing_sql, [])?,
        };
        transaction
            .commit()
            .map_err(|error| storage_error("结束周报事实读事务失败", error))?;
        Ok(facts)
    }

    fn verify_integrity(&mut self) -> Result<IntegrityReport, LedgerError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|error| storage_error("开始完整性读事务失败", error))?;
        let report = verify_integrity_in(&transaction)?;
        transaction
            .commit()
            .map_err(|error| storage_error("结束完整性读事务失败", error))?;
        Ok(report)
    }
}

fn write_task(transaction: &Transaction<'_>, write: &TaskWrite) -> Result<(), LedgerError> {
    match write {
        TaskWrite::Insert { task, .. } => {
            transaction
                .execute(
                    "INSERT INTO tasks (
                        id, title, status, queue_position, defer_until_ms, deadline_on,
                        block_reason, abandon_reason, completed_at_ms, active_completion_event_id,
                        version, created_at_ms, updated_at_ms
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                    params![
                        task.id,
                        task.title,
                        task.status.as_storage(),
                        task.queue_position,
                        task.defer_until_ms,
                        task.deadline_on,
                        task.block_reason,
                        task.abandon_reason,
                        task.completed_at_ms,
                        task.active_completion_event_id,
                        task.version,
                        task.created_at_ms,
                        task.updated_at_ms
                    ],
                )
                .map_err(|error| storage_error("新建任务快照失败", error))?;
        }
        TaskWrite::Update {
            expected_version,
            task,
            ..
        } => {
            let changed = transaction
                .execute(
                    "UPDATE tasks SET
                        title = ?2, status = ?3, queue_position = ?4, defer_until_ms = ?5,
                        deadline_on = ?6, block_reason = ?7, abandon_reason = ?8,
                        completed_at_ms = ?9, active_completion_event_id = ?10, version = ?11,
                        created_at_ms = ?12, updated_at_ms = ?13
                     WHERE id = ?1 AND version = ?14",
                    params![
                        task.id,
                        task.title,
                        task.status.as_storage(),
                        task.queue_position,
                        task.defer_until_ms,
                        task.deadline_on,
                        task.block_reason,
                        task.abandon_reason,
                        task.completed_at_ms,
                        task.active_completion_event_id,
                        task.version,
                        task.created_at_ms,
                        task.updated_at_ms,
                        expected_version
                    ],
                )
                .map_err(|error| storage_error("更新任务快照失败", error))?;
            if changed != 1 {
                return Err(LedgerError::concurrency_conflict(format!(
                    "任务 {} 已被其他操作更新",
                    task.id
                )));
            }
        }
        TaskWrite::ReorderQueue {
            expected_queue,
            ordered_task_ids,
            occurred_at_ms,
        } => write_queue_reorder(
            transaction,
            expected_queue,
            ordered_task_ids,
            *occurred_at_ms,
        )?,
    }
    Ok(())
}

fn write_queue_reorder(
    transaction: &Transaction<'_>,
    expected_queue: &[crate::ledger::domain::QueuedTaskVersion],
    ordered_task_ids: &[String],
    occurred_at_ms: i64,
) -> Result<(), LedgerError> {
    let max_position: i64 = transaction
        .query_row(
            "SELECT COALESCE(MAX(queue_position), 0) FROM tasks",
            [],
            |row| row.get(0),
        )
        .map_err(|error| storage_error("读取队列最大位置失败", error))?;
    let offset = max_position
        .checked_add(expected_queue.len() as i64)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| LedgerError::integrity("队列临时位置计算溢出"))?;
    let shifted = transaction
        .execute(
            "UPDATE tasks SET queue_position = queue_position + ?1
             WHERE status = 'pending' AND defer_until_ms IS NULL",
            [offset],
        )
        .map_err(|error| storage_error("移动队列到临时安全位置失败", error))?;
    if shifted != expected_queue.len() {
        return Err(LedgerError::concurrency_conflict(
            "写入顺序前待办集合已经变化",
        ));
    }

    for (index, task_id) in ordered_task_ids.iter().enumerate() {
        let expected = expected_queue
            .iter()
            .find(|item| item.task_id == *task_id)
            .ok_or_else(|| LedgerError::integrity("调整后顺序包含未知任务"))?;
        let changed = transaction
            .execute(
                "UPDATE tasks SET
                    queue_position = ?2,
                    version = version + 1,
                    updated_at_ms = MAX(updated_at_ms, ?3)
                 WHERE id = ?1 AND version = ?4
                   AND status = 'pending' AND defer_until_ms IS NULL",
                params![
                    task_id,
                    index as i64 + 1,
                    occurred_at_ms,
                    expected.expected_version
                ],
            )
            .map_err(|error| storage_error("写入新的任务顺序失败", error))?;
        if changed != 1 {
            return Err(LedgerError::concurrency_conflict(format!(
                "任务 {task_id} 在调整顺序时已经变化"
            )));
        }
    }
    Ok(())
}

fn insert_event(transaction: &Transaction<'_>, event: &TaskEvent) -> Result<(), LedgerError> {
    let metadata_json = serde_json::to_string(&event.metadata)
        .map_err(|error| LedgerError::storage(format!("序列化任务事件元数据失败：{error}")))?;
    transaction
        .execute(
            "INSERT INTO task_events (
                id, command_id, task_id, title_snapshot, event_type, occurred_at_ms,
                reason, metadata_json, reverses_event_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                event.id,
                event.command_id,
                event.task_id,
                event.title_snapshot,
                event.event_type.as_storage(),
                event.occurred_at_ms,
                event.reason,
                metadata_json,
                event.reverses_event_id
            ],
        )
        .map_err(|error| storage_error("追加任务事件失败", error))?;
    Ok(())
}

fn insert_reward(
    transaction: &Transaction<'_>,
    reward: &RewardMutation,
    balance_after: i64,
) -> Result<(), LedgerError> {
    transaction
        .execute(
            "INSERT INTO reward_transactions (
                id, task_event_id, reward_type, amount, balance_after, occurred_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                reward.id,
                reward.task_event_id,
                reward.reward_type.as_storage(),
                reward.amount,
                balance_after,
                reward.occurred_at_ms
            ],
        )
        .map_err(|error| storage_error("追加奖励交易失败", error))?;
    Ok(())
}

fn materialize_queue_position(
    transaction: &Transaction<'_>,
    write: &mut TaskWrite,
) -> Result<(), LedgerError> {
    let (task, place_at_tail) = match write {
        TaskWrite::Insert {
            task,
            place_at_tail,
        }
        | TaskWrite::Update {
            task,
            place_at_tail,
            ..
        } => (task, *place_at_tail),
        TaskWrite::ReorderQueue { .. } => return Ok(()),
    };
    if !place_at_tail {
        return Ok(());
    }
    if task.status != TaskStatus::Pending || task.defer_until_ms.is_some() {
        return Err(LedgerError::integrity("只有立即可执行的待办才能放入队尾"));
    }
    task.queue_position = Some(next_queue_position_in(transaction)?);
    Ok(())
}

fn validate_transition_preconditions(
    transaction: &Transaction<'_>,
    mutation: &LedgerMutation,
) -> Result<(), LedgerError> {
    match &mutation.task_write {
        TaskWrite::Insert { task, .. } | TaskWrite::Update { task, .. } => {
            if mutation.event.event_type == TaskEventType::QueueReordered {
                return Err(LedgerError::integrity(
                    "queue_reordered 事件必须使用队列重排写入",
                ));
            }
            if task.id != mutation.event.task_id {
                return Err(LedgerError::integrity("任务写入与事件目标不一致"));
            }
        }
        TaskWrite::ReorderQueue {
            expected_queue,
            ordered_task_ids,
            ..
        } => {
            if mutation.event.event_type != TaskEventType::QueueReordered {
                return Err(LedgerError::integrity(
                    "队列重排写入缺少 queue_reordered 事件",
                ));
            }
            let actual_queue = queue_in(transaction)?;
            let actual_facts: Vec<_> = actual_queue
                .iter()
                .map(|task| {
                    (
                        task.id.as_str(),
                        task.version,
                        task.queue_position.unwrap_or_default(),
                    )
                })
                .collect();
            let expected_facts: Vec<_> = expected_queue
                .iter()
                .map(|task| {
                    (
                        task.task_id.as_str(),
                        task.expected_version,
                        task.expected_position,
                    )
                })
                .collect();
            if actual_facts != expected_facts {
                return Err(LedgerError::concurrency_conflict(
                    "待办顺序或任务版本已经变化",
                ));
            }
            let mut expected_ids: Vec<_> = expected_queue
                .iter()
                .map(|task| task.task_id.as_str())
                .collect();
            let mut ordered_ids: Vec<_> = ordered_task_ids.iter().map(String::as_str).collect();
            expected_ids.sort_unstable();
            ordered_ids.sort_unstable();
            if expected_ids != ordered_ids {
                return Err(LedgerError::integrity(
                    "调整后顺序与已校验队列不是同一任务集合",
                ));
            }
        }
    }
    Ok(())
}

fn replay_receipt_in(
    connection: &Connection,
    command_id: &str,
    request_fingerprint: &str,
) -> Result<Option<MutationReceipt>, LedgerError> {
    let stored = connection
        .query_row(
            "SELECT request_fingerprint, result_json
             FROM command_receipts WHERE command_id = ?1",
            [command_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| storage_error("读取命令回执失败", error))?;
    let Some((stored_fingerprint, result_json)) = stored else {
        return Ok(None);
    };
    if stored_fingerprint != request_fingerprint {
        return Err(LedgerError::command_conflict(format!(
            "commandId {command_id} 已用于不同请求"
        )));
    }
    Ok(Some(
        stored_receipt_from_json(&result_json)?.into_result(true),
    ))
}

fn current_task_in(connection: &Connection) -> Result<Option<Task>, LedgerError> {
    let sql = format!(
        "{TASK_SELECT} WHERE status = 'pending' AND defer_until_ms IS NULL
         ORDER BY queue_position ASC LIMIT 1"
    );
    connection
        .query_row(&sql, [], map_task)
        .optional()
        .map_err(|error| storage_error("读取当前任务失败", error))
}

fn queue_in(connection: &Connection) -> Result<Vec<Task>, LedgerError> {
    let sql = format!(
        "{TASK_SELECT} WHERE status = 'pending' AND defer_until_ms IS NULL
         ORDER BY queue_position ASC"
    );
    query_tasks(connection, &sql, [])
}

fn next_queue_position_in(connection: &Connection) -> Result<i64, LedgerError> {
    connection
        .query_row(
            "SELECT COALESCE(MAX(queue_position), 0) + 1 FROM tasks",
            [],
            |row| row.get(0),
        )
        .map_err(|error| storage_error("计算任务队尾失败", error))
}

fn reward_balance_in(connection: &Connection) -> Result<i64, LedgerError> {
    connection
        .query_row(
            "SELECT COALESCE(SUM(amount), 0) FROM reward_transactions",
            [],
            |row| row.get(0),
        )
        .map_err(|error| storage_error("计算金币余额失败", error))
}

fn map_task(row: &Row<'_>) -> rusqlite::Result<Task> {
    let status = parse_task_status(row.get::<_, String>(2)?, 2)?;
    Ok(Task {
        id: row.get(0)?,
        title: row.get(1)?,
        status,
        queue_position: row.get(3)?,
        defer_until_ms: row.get(4)?,
        deadline_on: row.get(5)?,
        block_reason: row.get(6)?,
        abandon_reason: row.get(7)?,
        completed_at_ms: row.get(8)?,
        active_completion_event_id: row.get(9)?,
        version: row.get(10)?,
        created_at_ms: row.get(11)?,
        updated_at_ms: row.get(12)?,
    })
}

fn map_event(row: &Row<'_>) -> rusqlite::Result<TaskEvent> {
    let event_type = parse_event_type(row.get::<_, String>(5)?, 5)?;
    let metadata_json: String = row.get(8)?;
    let metadata = serde_json::from_str(&metadata_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(8, Type::Text, Box::new(error))
    })?;
    Ok(TaskEvent {
        sequence: Some(row.get(0)?),
        id: row.get(1)?,
        command_id: row.get(2)?,
        task_id: row.get(3)?,
        title_snapshot: row.get(4)?,
        event_type,
        occurred_at_ms: row.get(6)?,
        reason: row.get(7)?,
        metadata,
        reverses_event_id: row.get(9)?,
    })
}

fn map_reward(row: &Row<'_>) -> rusqlite::Result<RewardTransaction> {
    let reward_type = parse_reward_type(row.get::<_, String>(3)?, 3)?;
    Ok(RewardTransaction {
        sequence: Some(row.get(0)?),
        id: row.get(1)?,
        task_event_id: row.get(2)?,
        reward_type,
        amount: row.get(4)?,
        balance_after: row.get(5)?,
        occurred_at_ms: row.get(6)?,
    })
}

fn parse_task_status(value: String, column: usize) -> rusqlite::Result<TaskStatus> {
    TaskStatus::from_storage(&value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(error))
    })
}

fn parse_event_type(value: String, column: usize) -> rusqlite::Result<TaskEventType> {
    TaskEventType::from_storage(&value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(error))
    })
}

fn parse_reward_type(value: String, column: usize) -> rusqlite::Result<RewardType> {
    RewardType::from_storage(&value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(error))
    })
}

fn query_tasks<P: rusqlite::Params>(
    connection: &Connection,
    sql: &str,
    params: P,
) -> Result<Vec<Task>, LedgerError> {
    let mut statement = connection
        .prepare(sql)
        .map_err(|error| storage_error("准备任务查询失败", error))?;
    let rows = statement
        .query_map(params, map_task)
        .map_err(|error| storage_error("执行任务查询失败", error))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| storage_error("解析任务查询结果失败", error))
}

fn query_events<P: rusqlite::Params>(
    connection: &Connection,
    sql: &str,
    params: P,
) -> Result<Vec<TaskEvent>, LedgerError> {
    let mut statement = connection
        .prepare(sql)
        .map_err(|error| storage_error("准备事件查询失败", error))?;
    let rows = statement
        .query_map(params, map_event)
        .map_err(|error| storage_error("执行事件查询失败", error))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| storage_error("解析事件查询结果失败", error))
}

fn query_rewards<P: rusqlite::Params>(
    connection: &Connection,
    sql: &str,
    params: P,
) -> Result<Vec<RewardTransaction>, LedgerError> {
    let mut statement = connection
        .prepare(sql)
        .map_err(|error| storage_error("准备奖励查询失败", error))?;
    let rows = statement
        .query_map(params, map_reward)
        .map_err(|error| storage_error("执行奖励查询失败", error))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| storage_error("解析奖励查询结果失败", error))
}

fn interrupt_if_needed(
    interruption: CommitInterruption,
    current_point: FailurePoint,
) -> Result<(), LedgerError> {
    match interruption {
        CommitInterruption::None => Ok(()),
        CommitInterruption::ReturnError(expected) if expected == current_point => Err(
            LedgerError::injected(format!("在 {current_point:?} 注入事务失败")),
        ),
        CommitInterruption::ExitBeforeCommit(code)
            if current_point == FailurePoint::BeforeCommit =>
        {
            std::process::exit(code)
        }
        _ => Ok(()),
    }
}

fn storage_error(context: &str, error: rusqlite::Error) -> LedgerError {
    LedgerError::storage(format!("{context}：{error}"))
}
