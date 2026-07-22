use super::{storage_error, SCHEMA_VERSION};
use crate::ledger::domain::{Clock, LedgerError, SystemClock};
use rusqlite::{params, Connection, Transaction, TransactionBehavior};

const APPLICATION_ID: i64 = 0x5A42_4E31;
const INITIAL_SCHEMA_VERSION: i64 = 1;
const QUEUE_REORDER_SCHEMA_VERSION: i64 = 2;
const TITLE_UPDATE_SCHEMA_VERSION: i64 = 3;
const DEADLINE_UPDATE_SCHEMA_VERSION: i64 = 4;

pub(super) fn migrate(connection: &mut Connection) -> Result<(), LedgerError> {
    loop {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| storage_error("开始数据库迁移检查失败", error))?;
        let application_id: i64 = transaction
            .pragma_query_value(None, "application_id", |row| row.get(0))
            .map_err(|error| storage_error("读取 SQLite application_id 失败", error))?;
        let current_version: i64 = transaction
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|error| storage_error("读取 SQLite user_version 失败", error))?;

        if SCHEMA_VERSION != DEADLINE_UPDATE_SCHEMA_VERSION {
            return Err(LedgerError::integrity(format!(
                "代码声明的数据库版本应为 {DEADLINE_UPDATE_SCHEMA_VERSION}，实际为 {SCHEMA_VERSION}"
            )));
        }
        if current_version > SCHEMA_VERSION {
            return Err(LedgerError::unsupported_schema(format!(
                "数据库版本 {current_version} 高于当前支持的 {SCHEMA_VERSION}"
            )));
        }

        match current_version {
            0 => return create_current_schema(transaction, application_id),
            INITIAL_SCHEMA_VERSION => migrate_v1_to_v2(transaction, application_id)?,
            QUEUE_REORDER_SCHEMA_VERSION => migrate_v2_to_v3(transaction, application_id)?,
            TITLE_UPDATE_SCHEMA_VERSION => migrate_v3_to_v4(transaction, application_id)?,
            DEADLINE_UPDATE_SCHEMA_VERSION => {
                return validate_current_schema(transaction, application_id)
            }
            version => {
                return Err(LedgerError::unsupported_schema(format!(
                    "数据库版本 {version} 不在支持的迁移路径中"
                )))
            }
        }
    }
}

fn create_current_schema(
    transaction: Transaction<'_>,
    application_id: i64,
) -> Result<(), LedgerError> {
    if application_id != 0 && application_id != APPLICATION_ID {
        return Err(LedgerError::integrity(format!(
            "文件不是代办账本，application_id={application_id}"
        )));
    }

    let user_object_count: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema
             WHERE name NOT LIKE 'sqlite_%'
               AND type IN ('table', 'index', 'view', 'trigger')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| storage_error("检查未标识数据库对象失败", error))?;
    if user_object_count != 0 {
        return Err(LedgerError::integrity(format!(
            "未标识数据库中已经存在 {user_object_count} 个用户对象，拒绝接管"
        )));
    }

    transaction
        .execute_batch(CURRENT_SCHEMA_SQL)
        .map_err(|error| storage_error("创建账本表结构失败", error))?;
    record_migration(
        &transaction,
        SCHEMA_VERSION,
        "任务快照、事件、奖励、幂等回执、队列重排、标题修改与截止日期初始结构",
    )?;
    transaction
        .pragma_update(None, "application_id", APPLICATION_ID)
        .map_err(|error| storage_error("写入 SQLite application_id 失败", error))?;
    transaction
        .pragma_update(None, "user_version", SCHEMA_VERSION)
        .map_err(|error| storage_error("写入 SQLite user_version 失败", error))?;
    transaction
        .commit()
        .map_err(|error| storage_error("提交数据库初始化失败", error))
}

fn migrate_v1_to_v2(transaction: Transaction<'_>, application_id: i64) -> Result<(), LedgerError> {
    require_application_id(application_id)?;
    require_migration_record(&transaction, INITIAL_SCHEMA_VERSION)?;

    let foreign_keys_enabled: i64 = transaction
        .pragma_query_value(None, "foreign_keys", |row| row.get(0))
        .map_err(|error| storage_error("读取 SQLite foreign_keys 失败", error))?;
    if foreign_keys_enabled != 0 {
        return Err(LedgerError::integrity(
            "v1 升级前必须在事务外关闭 SQLite foreign_keys",
        ));
    }

    let before = event_sequence_summary(&transaction, "task_events")?;
    transaction
        .execute_batch(
            "CREATE TABLE task_events_new (
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
                reverses_event_id TEXT REFERENCES task_events_new(id)
            ) STRICT;

            INSERT INTO task_events_new (
                sequence, id, command_id, task_id, title_snapshot, event_type,
                occurred_at_ms, reason, metadata_json, reverses_event_id
            )
            SELECT sequence, id, command_id, task_id, title_snapshot, event_type,
                   occurred_at_ms, reason, metadata_json, reverses_event_id
            FROM task_events
            ORDER BY sequence ASC;",
        )
        .map_err(|error| storage_error("复制 v1 任务事件失败", error))?;

    let copied = event_sequence_summary(&transaction, "task_events_new")?;
    if copied != before {
        return Err(LedgerError::integrity(format!(
            "v1 任务事件复制校验失败：迁移前 {before:?}，复制后 {copied:?}"
        )));
    }

    transaction
        .execute_batch(
            "DROP TABLE task_events;
             ALTER TABLE task_events_new RENAME TO task_events;
             CREATE INDEX task_events_task_time_index
                 ON task_events(task_id, occurred_at_ms, sequence);
             CREATE INDEX task_events_type_time_index
                 ON task_events(event_type, occurred_at_ms, sequence);",
        )
        .map_err(|error| storage_error("替换 v2 任务事件表失败", error))?;

    validate_autoincrement_sequence(&transaction, before.2)?;
    ensure_foreign_keys_valid(&transaction)?;
    record_migration(
        &transaction,
        QUEUE_REORDER_SCHEMA_VERSION,
        "任务事件增加 queue_reordered 队列重排类型",
    )?;
    transaction
        .pragma_update(None, "user_version", QUEUE_REORDER_SCHEMA_VERSION)
        .map_err(|error| storage_error("写入 SQLite user_version 失败", error))?;
    transaction
        .commit()
        .map_err(|error| storage_error("提交 v1 到 v2 数据库迁移失败", error))
}

fn migrate_v2_to_v3(transaction: Transaction<'_>, application_id: i64) -> Result<(), LedgerError> {
    require_application_id(application_id)?;
    require_migration_record(&transaction, QUEUE_REORDER_SCHEMA_VERSION)?;

    let foreign_keys_enabled: i64 = transaction
        .pragma_query_value(None, "foreign_keys", |row| row.get(0))
        .map_err(|error| storage_error("读取 SQLite foreign_keys 失败", error))?;
    if foreign_keys_enabled != 0 {
        return Err(LedgerError::integrity(
            "v2 升级前必须在事务外关闭 SQLite foreign_keys",
        ));
    }

    let before = event_sequence_summary(&transaction, "task_events")?;
    transaction
        .execute_batch(
            "CREATE TABLE task_events_new (
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
                reverses_event_id TEXT REFERENCES task_events_new(id)
            ) STRICT;

            INSERT INTO task_events_new (
                sequence, id, command_id, task_id, title_snapshot, event_type,
                occurred_at_ms, reason, metadata_json, reverses_event_id
            )
            SELECT sequence, id, command_id, task_id, title_snapshot, event_type,
                   occurred_at_ms, reason, metadata_json, reverses_event_id
            FROM task_events
            ORDER BY sequence ASC;",
        )
        .map_err(|error| storage_error("复制 v2 任务事件失败", error))?;

    let copied = event_sequence_summary(&transaction, "task_events_new")?;
    if copied != before {
        return Err(LedgerError::integrity(format!(
            "v2 任务事件复制校验失败：迁移前 {before:?}，复制后 {copied:?}"
        )));
    }

    transaction
        .execute_batch(
            "DROP TABLE task_events;
             ALTER TABLE task_events_new RENAME TO task_events;
             CREATE INDEX task_events_task_time_index
                 ON task_events(task_id, occurred_at_ms, sequence);
             CREATE INDEX task_events_type_time_index
                 ON task_events(event_type, occurred_at_ms, sequence);",
        )
        .map_err(|error| storage_error("替换 v3 任务事件表失败", error))?;

    validate_autoincrement_sequence(&transaction, before.2)?;
    ensure_foreign_keys_valid(&transaction)?;
    record_migration(
        &transaction,
        TITLE_UPDATE_SCHEMA_VERSION,
        "任务事件增加 title_updated 标题修改类型",
    )?;
    transaction
        .pragma_update(None, "user_version", TITLE_UPDATE_SCHEMA_VERSION)
        .map_err(|error| storage_error("写入 SQLite user_version 失败", error))?;
    transaction
        .commit()
        .map_err(|error| storage_error("提交 v2 到 v3 数据库迁移失败", error))
}

fn migrate_v3_to_v4(transaction: Transaction<'_>, application_id: i64) -> Result<(), LedgerError> {
    require_application_id(application_id)?;
    require_migration_record(&transaction, TITLE_UPDATE_SCHEMA_VERSION)?;

    let foreign_keys_enabled: i64 = transaction
        .pragma_query_value(None, "foreign_keys", |row| row.get(0))
        .map_err(|error| storage_error("读取 SQLite foreign_keys 失败", error))?;
    if foreign_keys_enabled != 0 {
        return Err(LedgerError::integrity(
            "v3 升级前必须在事务外关闭 SQLite foreign_keys",
        ));
    }

    transaction
        .execute_batch(
            "ALTER TABLE tasks ADD COLUMN deadline_on TEXT
                 CHECK(deadline_on IS NULL OR deadline_on GLOB
                     '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]');",
        )
        .map_err(|error| storage_error("为 v3 任务增加截止日期字段失败", error))?;

    let before = event_sequence_summary(&transaction, "task_events")?;
    transaction
        .execute_batch(
            "CREATE TABLE task_events_new (
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
                reverses_event_id TEXT REFERENCES task_events_new(id)
            ) STRICT;

            INSERT INTO task_events_new (
                sequence, id, command_id, task_id, title_snapshot, event_type,
                occurred_at_ms, reason, metadata_json, reverses_event_id
            )
            SELECT sequence, id, command_id, task_id, title_snapshot, event_type,
                   occurred_at_ms, reason, metadata_json, reverses_event_id
            FROM task_events
            ORDER BY sequence ASC;",
        )
        .map_err(|error| storage_error("复制 v3 任务事件失败", error))?;

    let copied = event_sequence_summary(&transaction, "task_events_new")?;
    if copied != before {
        return Err(LedgerError::integrity(format!(
            "v3 任务事件复制校验失败：迁移前 {before:?}，复制后 {copied:?}"
        )));
    }

    transaction
        .execute_batch(
            "DROP TABLE task_events;
             ALTER TABLE task_events_new RENAME TO task_events;
             CREATE INDEX task_events_task_time_index
                 ON task_events(task_id, occurred_at_ms, sequence);
             CREATE INDEX task_events_type_time_index
                 ON task_events(event_type, occurred_at_ms, sequence);",
        )
        .map_err(|error| storage_error("替换 v4 任务事件表失败", error))?;

    validate_autoincrement_sequence(&transaction, before.2)?;
    ensure_foreign_keys_valid(&transaction)?;
    record_migration(
        &transaction,
        DEADLINE_UPDATE_SCHEMA_VERSION,
        "任务增加可选截止日期并增加 deadline_updated 事件类型",
    )?;
    transaction
        .pragma_update(None, "user_version", DEADLINE_UPDATE_SCHEMA_VERSION)
        .map_err(|error| storage_error("写入 SQLite user_version 失败", error))?;
    transaction
        .commit()
        .map_err(|error| storage_error("提交 v3 到 v4 数据库迁移失败", error))
}

fn validate_current_schema(
    transaction: Transaction<'_>,
    application_id: i64,
) -> Result<(), LedgerError> {
    require_application_id(application_id)?;
    require_migration_record(&transaction, SCHEMA_VERSION)?;
    let task_events_sql: String = transaction
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'task_events'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| storage_error("读取 task_events 表结构失败", error))?;
    if !task_events_sql.contains("'queue_reordered'")
        || !task_events_sql.contains("'title_updated'")
        || !task_events_sql.contains("'deadline_updated'")
    {
        return Err(LedgerError::integrity(
            "数据库标记为 v4，但 task_events 缺少当前事件类型",
        ));
    }
    let tasks_sql: String = transaction
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'tasks'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| storage_error("读取 tasks 表结构失败", error))?;
    if !tasks_sql.contains("deadline_on")
        || !tasks_sql.contains("[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]")
    {
        return Err(LedgerError::integrity(
            "数据库标记为 v4，但 tasks 缺少受约束的 deadline_on 字段",
        ));
    }
    transaction
        .commit()
        .map_err(|error| storage_error("结束数据库迁移检查失败", error))
}

fn require_application_id(application_id: i64) -> Result<(), LedgerError> {
    if application_id != APPLICATION_ID {
        return Err(LedgerError::integrity(format!(
            "数据库版本存在，但 application_id 不匹配：{application_id}"
        )));
    }
    Ok(())
}

fn require_migration_record(
    transaction: &Transaction<'_>,
    version: i64,
) -> Result<(), LedgerError> {
    let count: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE version = ?1",
            [version],
            |row| row.get(0),
        )
        .map_err(|error| storage_error("读取数据库迁移记录失败", error))?;
    if count != 1 {
        return Err(LedgerError::integrity(format!(
            "数据库缺少唯一的 v{version} 迁移记录"
        )));
    }
    Ok(())
}

fn record_migration(
    transaction: &Transaction<'_>,
    version: i64,
    description: &str,
) -> Result<(), LedgerError> {
    transaction
        .execute(
            "INSERT INTO schema_migrations (version, description, applied_at_ms)
             VALUES (?1, ?2, ?3)",
            params![version, description, SystemClock.now_ms()],
        )
        .map_err(|error| storage_error("记录数据库迁移失败", error))?;
    Ok(())
}

fn event_sequence_summary(
    transaction: &Transaction<'_>,
    table_name: &str,
) -> Result<(i64, i64, i64), LedgerError> {
    let sql = match table_name {
        "task_events" => {
            "SELECT COUNT(*), COALESCE(MIN(sequence), 0), COALESCE(MAX(sequence), 0)
             FROM task_events"
        }
        "task_events_new" => {
            "SELECT COUNT(*), COALESCE(MIN(sequence), 0), COALESCE(MAX(sequence), 0)
             FROM task_events_new"
        }
        _ => return Err(LedgerError::integrity("未知的任务事件迁移表")),
    };
    transaction
        .query_row(sql, [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(|error| storage_error("读取任务事件序列摘要失败", error))
}

fn validate_autoincrement_sequence(
    transaction: &Transaction<'_>,
    expected_max_sequence: i64,
) -> Result<(), LedgerError> {
    if expected_max_sequence == 0 {
        return Ok(());
    }
    let sequence: i64 = transaction
        .query_row(
            "SELECT seq FROM sqlite_sequence WHERE name = 'task_events'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| storage_error("读取 task_events 自增序列失败", error))?;
    if sequence < expected_max_sequence {
        return Err(LedgerError::integrity(format!(
            "task_events 自增序列 {sequence} 小于已有最大序号 {expected_max_sequence}"
        )));
    }
    Ok(())
}

fn ensure_foreign_keys_valid(transaction: &Transaction<'_>) -> Result<(), LedgerError> {
    let mut statement = transaction
        .prepare("PRAGMA foreign_key_check")
        .map_err(|error| storage_error("准备 SQLite foreign_key_check 失败", error))?;
    let mut rows = statement
        .query([])
        .map_err(|error| storage_error("执行 SQLite foreign_key_check 失败", error))?;
    if rows
        .next()
        .map_err(|error| storage_error("读取 SQLite foreign_key_check 失败", error))?
        .is_some()
    {
        return Err(LedgerError::integrity("v1 到 v2 迁移后存在外键不一致"));
    }
    Ok(())
}

const CURRENT_SCHEMA_SQL: &str = "CREATE TABLE tasks (
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
        (status = 'pending' AND completed_at_ms IS NULL AND active_completion_event_id IS NULL
            AND ((defer_until_ms IS NULL AND queue_position IS NOT NULL)
                OR (defer_until_ms IS NOT NULL AND queue_position IS NULL)))
        OR (status = 'completed' AND queue_position IS NULL AND defer_until_ms IS NULL
            AND completed_at_ms IS NOT NULL AND active_completion_event_id IS NOT NULL)
        OR (status IN ('blocked', 'abandoned') AND queue_position IS NULL
            AND completed_at_ms IS NULL AND active_completion_event_id IS NULL)
    )
) STRICT;

CREATE UNIQUE INDEX tasks_queue_position_unique
    ON tasks(queue_position) WHERE queue_position IS NOT NULL;
CREATE INDEX tasks_status_index ON tasks(status);

CREATE TABLE task_events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    command_id TEXT NOT NULL UNIQUE,
    task_id TEXT NOT NULL REFERENCES tasks(id),
    title_snapshot TEXT NOT NULL,
    event_type TEXT NOT NULL CHECK(event_type IN (
        'created', 'completed', 'completion_undone', 'deferred', 'due_recovered',
        'blocked', 'recovered', 'abandoned', 'reopened', 'queue_reordered', 'title_updated',
        'deadline_updated'
    )),
    occurred_at_ms INTEGER NOT NULL CHECK(occurred_at_ms >= 0),
    reason TEXT,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
    reverses_event_id TEXT REFERENCES task_events(id)
) STRICT;
CREATE INDEX task_events_task_time_index
    ON task_events(task_id, occurred_at_ms, sequence);
CREATE INDEX task_events_type_time_index
    ON task_events(event_type, occurred_at_ms, sequence);

CREATE TABLE reward_transactions (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    task_event_id TEXT NOT NULL UNIQUE REFERENCES task_events(id),
    reward_type TEXT NOT NULL CHECK(reward_type IN ('task_completion', 'completion_undo')),
    amount INTEGER NOT NULL CHECK(
        (reward_type = 'task_completion' AND amount = 1)
        OR (reward_type = 'completion_undo' AND amount = -1)
    ),
    balance_after INTEGER NOT NULL CHECK(balance_after >= 0),
    occurred_at_ms INTEGER NOT NULL CHECK(occurred_at_ms >= 0)
) STRICT;

CREATE TABLE command_receipts (
    command_id TEXT PRIMARY KEY NOT NULL,
    command_type TEXT NOT NULL,
    request_fingerprint TEXT NOT NULL,
    result_json TEXT NOT NULL CHECK(json_valid(result_json)),
    created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0)
) STRICT;

CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY NOT NULL,
    description TEXT NOT NULL,
    applied_at_ms INTEGER NOT NULL CHECK(applied_at_ms >= 0)
) STRICT;";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_database_is_created_at_v4() {
        let mut connection = Connection::open_in_memory().expect("应打开内存数据库");
        migrate(&mut connection).expect("应建立最新结构");

        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("应读取数据库版本");
        let application_id: i64 = connection
            .pragma_query_value(None, "application_id", |row| row.get(0))
            .expect("应读取应用标识");
        let event_table_sql: String = connection
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'task_events'",
                [],
                |row| row.get(0),
            )
            .expect("应读取事件表结构");

        assert_eq!(version, DEADLINE_UPDATE_SCHEMA_VERSION);
        assert_eq!(application_id, APPLICATION_ID);
        assert!(event_table_sql.contains("'queue_reordered'"));
        assert!(event_table_sql.contains("'title_updated'"));
        assert!(event_table_sql.contains("'deadline_updated'"));
    }

    #[test]
    fn v1_database_migrates_to_v4_without_losing_sequence_or_foreign_keys() {
        let mut connection = Connection::open_in_memory().expect("应打开内存数据库");
        connection
            .execute_batch(V1_MIGRATION_FIXTURE_SQL)
            .expect("应建立 v1 迁移样本");
        connection
            .pragma_update(None, "application_id", APPLICATION_ID)
            .expect("应写入应用标识");
        connection
            .pragma_update(None, "user_version", INITIAL_SCHEMA_VERSION)
            .expect("应写入 v1 版本");
        connection
            .pragma_update(None, "foreign_keys", "OFF")
            .expect("迁移前应关闭外键");

        migrate(&mut connection).expect("v1 应连续迁移到 v4");
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .expect("应重新启用外键");

        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("应读取迁移后版本");
        let event_summary: (i64, i64, i64) = connection
            .query_row(
                "SELECT COUNT(*), MIN(sequence), MAX(sequence) FROM task_events",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("应读取迁移后事件");
        let migration_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version IN (1, 2, 3, 4)",
                [],
                |row| row.get(0),
            )
            .expect("应读取迁移记录");
        let event_index_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema
                 WHERE type = 'index' AND name IN (
                    'task_events_task_time_index', 'task_events_type_time_index'
                 )",
                [],
                |row| row.get(0),
            )
            .expect("应读取事件索引");
        let foreign_key_violation_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .expect("应执行外键检查");

        assert_eq!(version, DEADLINE_UPDATE_SCHEMA_VERSION);
        assert_eq!(event_summary, (1, 7, 7));
        assert_eq!(migration_count, 4);
        assert_eq!(event_index_count, 2);
        assert_eq!(foreign_key_violation_count, 0);

        connection
            .execute(
                "INSERT INTO task_events (
                    id, command_id, task_id, title_snapshot, event_type,
                    occurred_at_ms, metadata_json
                 ) VALUES ('event-reorder', 'reorder-one', 'task-one', '任务一',
                           'queue_reordered', 20, '{\"orderedTaskIds\":[\"task-one\"]}')",
                [],
            )
            .expect("迁移后应允许追加队列重排事件");
        let next_sequence: i64 = connection
            .query_row(
                "SELECT sequence FROM task_events WHERE id = 'event-reorder'",
                [],
                |row| row.get(0),
            )
            .expect("应读取新事件序号");
        assert!(next_sequence > 7);

        connection
            .execute(
                "INSERT INTO task_events (
                    id, command_id, task_id, title_snapshot, event_type,
                    occurred_at_ms, metadata_json
                 ) VALUES ('event-title', 'update-title-one', 'task-one', '任务一（修改）',
                           'title_updated', 30,
                           '{\"beforeTitle\":\"任务一\",\"afterTitle\":\"任务一（修改）\"}')",
                [],
            )
            .expect("迁移后应允许追加标题修改事件");
    }

    #[test]
    fn v2_database_migrates_to_v4_and_accepts_current_events() {
        let mut connection = Connection::open_in_memory().expect("应打开内存数据库");
        connection
            .execute_batch(V2_MIGRATION_FIXTURE_SQL)
            .expect("应建立 v2 迁移样本");
        connection
            .pragma_update(None, "application_id", APPLICATION_ID)
            .expect("应写入应用标识");
        connection
            .pragma_update(None, "user_version", QUEUE_REORDER_SCHEMA_VERSION)
            .expect("应写入 v2 版本");
        connection
            .pragma_update(None, "foreign_keys", "OFF")
            .expect("迁移前应关闭外键");

        migrate(&mut connection).expect("v2 应迁移到 v4");
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .expect("应重新启用外键");

        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("应读取迁移后版本");
        let event_summary: (i64, i64, i64) = connection
            .query_row(
                "SELECT COUNT(*), MIN(sequence), MAX(sequence) FROM task_events",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("应读取迁移后事件");
        let migration_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version IN (2, 3, 4)",
                [],
                |row| row.get(0),
            )
            .expect("应读取迁移记录");
        let foreign_key_violation_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .expect("应执行外键检查");

        assert_eq!(version, DEADLINE_UPDATE_SCHEMA_VERSION);
        assert_eq!(event_summary, (1, 5, 5));
        assert_eq!(migration_count, 3);
        assert_eq!(foreign_key_violation_count, 0);
        connection
            .execute(
                "INSERT INTO task_events (
                    id, command_id, task_id, title_snapshot, event_type,
                    occurred_at_ms, metadata_json
                 ) VALUES ('event-title', 'update-title-one', 'task-one', '任务一（修改）',
                           'title_updated', 20,
                           '{\"beforeTitle\":\"任务一\",\"afterTitle\":\"任务一（修改）\"}')",
                [],
            )
            .expect("v4 应允许标题修改事件");
        connection
            .execute(
                "INSERT INTO task_events (
                    id, command_id, task_id, title_snapshot, event_type,
                    occurred_at_ms, metadata_json
                 ) VALUES ('event-deadline', 'update-deadline-one', 'task-one', '任务一（修改）',
                           'deadline_updated', 30,
                           '{\"beforeDeadlineOn\":null,\"afterDeadlineOn\":\"2026-07-20\"}')",
                [],
            )
            .expect("v4 应允许截止日期修改事件");
    }

    #[test]
    fn database_marked_v4_without_current_event_constraints_is_rejected() {
        let mut connection = Connection::open_in_memory().expect("应打开内存数据库");
        connection
            .execute_batch(V2_MIGRATION_FIXTURE_SQL)
            .expect("应建立伪造 v4 样本");
        connection
            .execute_batch(
                "INSERT INTO schema_migrations (version, description, applied_at_ms)
                 VALUES (3, '伪造 v3', 2);
                 INSERT INTO schema_migrations (version, description, applied_at_ms)
                 VALUES (4, '伪造 v4', 3);
                 ALTER TABLE tasks ADD COLUMN deadline_on TEXT
                     CHECK(deadline_on IS NULL OR deadline_on GLOB
                         '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]');",
            )
            .expect("应写入伪造迁移记录");
        connection
            .pragma_update(None, "application_id", APPLICATION_ID)
            .expect("应写入应用标识");
        connection
            .pragma_update(None, "user_version", DEADLINE_UPDATE_SCHEMA_VERSION)
            .expect("应写入伪造 v4 版本");

        let error = migrate(&mut connection).expect_err("缺少当前事件约束的 v4 必须拒绝");
        assert_eq!(error.code(), "DATA_INTEGRITY_ERROR");
    }

    const V1_MIGRATION_FIXTURE_SQL: &str = "
        CREATE TABLE tasks (id TEXT PRIMARY KEY NOT NULL);
        CREATE TABLE task_events (
            sequence INTEGER PRIMARY KEY AUTOINCREMENT,
            id TEXT NOT NULL UNIQUE,
            command_id TEXT NOT NULL UNIQUE,
            task_id TEXT NOT NULL REFERENCES tasks(id),
            title_snapshot TEXT NOT NULL,
            event_type TEXT NOT NULL CHECK(event_type IN (
                'created', 'completed', 'completion_undone', 'deferred', 'due_recovered',
                'blocked', 'recovered', 'abandoned', 'reopened'
            )),
            occurred_at_ms INTEGER NOT NULL,
            reason TEXT,
            metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
            reverses_event_id TEXT REFERENCES task_events(id)
        ) STRICT;
        CREATE INDEX task_events_task_time_index
            ON task_events(task_id, occurred_at_ms, sequence);
        CREATE INDEX task_events_type_time_index
            ON task_events(event_type, occurred_at_ms, sequence);
        CREATE TABLE reward_transactions (
            id TEXT PRIMARY KEY NOT NULL,
            task_event_id TEXT NOT NULL REFERENCES task_events(id)
        ) STRICT;
        CREATE TABLE schema_migrations (
            version INTEGER PRIMARY KEY NOT NULL,
            description TEXT NOT NULL,
            applied_at_ms INTEGER NOT NULL
        ) STRICT;
        INSERT INTO tasks (id) VALUES ('task-one');
        INSERT INTO task_events (
            sequence, id, command_id, task_id, title_snapshot, event_type,
            occurred_at_ms, metadata_json
        ) VALUES (7, 'event-created', 'capture-one', 'task-one', '任务一', 'created', 10, '{}');
        INSERT INTO reward_transactions (id, task_event_id)
            VALUES ('reward-reference', 'event-created');
        INSERT INTO schema_migrations (version, description, applied_at_ms)
            VALUES (1, 'v1 fixture', 10);";

    const V2_MIGRATION_FIXTURE_SQL: &str = "
        CREATE TABLE tasks (id TEXT PRIMARY KEY NOT NULL);
        CREATE TABLE task_events (
            sequence INTEGER PRIMARY KEY AUTOINCREMENT,
            id TEXT NOT NULL UNIQUE,
            command_id TEXT NOT NULL UNIQUE,
            task_id TEXT NOT NULL REFERENCES tasks(id),
            title_snapshot TEXT NOT NULL,
            event_type TEXT NOT NULL CHECK(event_type IN (
                'created', 'completed', 'completion_undone', 'deferred', 'due_recovered',
                'blocked', 'recovered', 'abandoned', 'reopened', 'queue_reordered'
            )),
            occurred_at_ms INTEGER NOT NULL,
            reason TEXT,
            metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
            reverses_event_id TEXT REFERENCES task_events(id)
        ) STRICT;
        CREATE INDEX task_events_task_time_index
            ON task_events(task_id, occurred_at_ms, sequence);
        CREATE INDEX task_events_type_time_index
            ON task_events(event_type, occurred_at_ms, sequence);
        CREATE TABLE reward_transactions (
            id TEXT PRIMARY KEY NOT NULL,
            task_event_id TEXT NOT NULL REFERENCES task_events(id)
        ) STRICT;
        CREATE TABLE schema_migrations (
            version INTEGER PRIMARY KEY NOT NULL,
            description TEXT NOT NULL,
            applied_at_ms INTEGER NOT NULL
        ) STRICT;
        INSERT INTO tasks (id) VALUES ('task-one');
        INSERT INTO task_events (
            sequence, id, command_id, task_id, title_snapshot, event_type,
            occurred_at_ms, metadata_json
        ) VALUES (5, 'event-created', 'capture-one', 'task-one', '任务一', 'created', 10, '{}');
        INSERT INTO reward_transactions (id, task_event_id)
            VALUES ('reward-reference', 'event-created');
        INSERT INTO schema_migrations (version, description, applied_at_ms)
            VALUES (2, 'v2 fixture', 10);";
}
