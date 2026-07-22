use super::{
    query_events, query_rewards, query_tasks, storage_error, EVENT_SELECT, REWARD_SELECT,
    SCHEMA_VERSION, TASK_SELECT,
};
use crate::ledger::{
    domain::{
        normalize_deadline_on, normalize_title, IntegrityReport, LedgerError, TaskEvent,
        TaskEventType, TaskStatus,
    },
    service::stored_receipt_from_json,
};
use rusqlite::{params, Connection};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

#[derive(Debug)]
struct ReceiptIntegrityRow {
    command_id: String,
    result_json: String,
    event_id: Option<String>,
    task_id: Option<String>,
    event_sequence: Option<i64>,
    reward_transaction_id: Option<String>,
    expected_balance: Option<i64>,
}

#[derive(Debug)]
struct QueueReplay {
    current_task_id_by_sequence: HashMap<i64, Option<String>>,
    final_task_ids: Vec<String>,
    final_titles_by_task_id: HashMap<String, String>,
    final_deadlines_by_task_id: HashMap<String, Option<String>>,
    failures: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueueReorderMetadata {
    moved_task_id: String,
    before_task_ids: Vec<String>,
    after_task_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TitleUpdatedMetadata {
    before_title: String,
    after_title: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeadlineUpdatedMetadata {
    before_deadline_on: RequiredNullableDeadline,
    after_deadline_on: RequiredNullableDeadline,
}

#[derive(Debug, Deserialize)]
#[serde(transparent)]
struct RequiredNullableDeadline(Option<String>);

pub(super) fn verify_integrity_in(connection: &Connection) -> Result<IntegrityReport, LedgerError> {
    let mut failures = Vec::new();
    let quick_check_value: String = connection
        .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
        .map_err(|error| storage_error("执行 SQLite quick_check 失败", error))?;
    let sqlite_quick_check = quick_check_value == "ok";
    if !sqlite_quick_check {
        failures.push(format!("SQLite quick_check：{quick_check_value}"));
    }

    let foreign_keys = {
        let mut statement = connection
            .prepare("PRAGMA foreign_key_check")
            .map_err(|error| storage_error("准备 SQLite foreign_key_check 失败", error))?;
        let mut rows = statement
            .query([])
            .map_err(|error| storage_error("执行 SQLite foreign_key_check 失败", error))?;
        rows.next()
            .map_err(|error| storage_error("读取 SQLite foreign_key_check 失败", error))?
            .is_none()
    };
    if !foreign_keys {
        failures.push("SQLite foreign_key_check 发现孤立记录".to_string());
    }

    let rewards_sql = format!("{REWARD_SELECT} ORDER BY sequence ASC");
    let rewards = query_rewards(connection, &rewards_sql, [])?;
    let mut running_balance = 0_i64;
    let mut reward_prefix_balances = true;
    for reward in &rewards {
        running_balance += reward.amount;
        if running_balance < 0 || running_balance != reward.balance_after {
            reward_prefix_balances = false;
            failures.push(format!(
                "奖励交易 {} 的 balanceAfter 与前缀和不一致",
                reward.id
            ));
        }
    }

    let missing_or_mismatched_rewards: i64 = connection
        .query_row(
            "SELECT COUNT(*)
             FROM task_events AS event
             LEFT JOIN reward_transactions AS reward ON reward.task_event_id = event.id
             WHERE event.event_type IN ('completed', 'completion_undone')
               AND (
                   reward.id IS NULL
                   OR (event.event_type = 'completed' AND reward.reward_type != 'task_completion')
                   OR (event.event_type = 'completion_undone' AND reward.reward_type != 'completion_undo')
               )",
            [],
            |row| row.get(0),
        )
        .map_err(|error| storage_error("校验完成事件奖励关联失败", error))?;
    let invalid_reward_links: i64 = connection
        .query_row(
            "SELECT COUNT(*)
             FROM reward_transactions AS reward
             JOIN task_events AS event ON event.id = reward.task_event_id
             WHERE (reward.reward_type = 'task_completion' AND event.event_type != 'completed')
                OR (reward.reward_type = 'completion_undo' AND event.event_type != 'completion_undone')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| storage_error("校验奖励事件关联失败", error))?;
    let event_reward_links = missing_or_mismatched_rewards == 0 && invalid_reward_links == 0;
    if missing_or_mismatched_rewards != 0 {
        failures.push(format!(
            "存在 {missing_or_mismatched_rewards} 条完成或撤销事件缺少正确奖励交易"
        ));
    }
    if invalid_reward_links != 0 {
        failures.push(format!(
            "存在 {invalid_reward_links} 条奖励交易关联了错误事件类型"
        ));
    }

    let invalid_event_receipts: i64 = connection
        .query_row(
            "SELECT COUNT(*)
             FROM task_events AS event
             LEFT JOIN command_receipts AS receipt ON receipt.command_id = event.command_id
             LEFT JOIN reward_transactions AS reward ON reward.task_event_id = event.id
                 WHERE receipt.command_id IS NULL
                 OR receipt.command_type != CASE event.event_type
                     WHEN 'created' THEN 'capture_task'
                     WHEN 'completed' THEN 'complete_task'
                     WHEN 'completion_undone' THEN 'undo_completion'
                      WHEN 'abandoned' THEN 'delete_task'
                      WHEN 'queue_reordered' THEN 'reorder_tasks'
                       WHEN 'title_updated' THEN 'update_task_title'
                       WHEN 'deadline_updated' THEN 'update_task_deadline'
                      ELSE receipt.command_type
                     END",
            [],
            |row| row.get(0),
        )
        .map_err(|error| storage_error("校验任务事件命令回执失败", error))?;
    let orphan_receipts: i64 = connection
        .query_row(
            "SELECT COUNT(*)
             FROM command_receipts AS receipt
             LEFT JOIN task_events AS event ON event.command_id = receipt.command_id
             WHERE event.id IS NULL",
            [],
            |row| row.get(0),
        )
        .map_err(|error| storage_error("校验孤立命令回执失败", error))?;

    let receipt_rows = {
        let mut statement = connection
            .prepare(
                "SELECT receipt.command_id, receipt.result_json,
                        event.id, event.task_id, event.sequence, reward.id,
                        CASE WHEN event.sequence IS NULL THEN NULL ELSE (
                            SELECT COALESCE(SUM(prefix_reward.amount), 0)
                            FROM reward_transactions AS prefix_reward
                            JOIN task_events AS prefix_event
                              ON prefix_event.id = prefix_reward.task_event_id
                            WHERE prefix_event.sequence <= event.sequence
                        ) END
                 FROM command_receipts AS receipt
                 LEFT JOIN task_events AS event ON event.command_id = receipt.command_id
                 LEFT JOIN reward_transactions AS reward ON reward.task_event_id = event.id
                 ORDER BY receipt.command_id ASC",
            )
            .map_err(|error| storage_error("准备命令回执语义校验失败", error))?;
        let rows = statement
            .query_map([], |row| {
                Ok(ReceiptIntegrityRow {
                    command_id: row.get(0)?,
                    result_json: row.get(1)?,
                    event_id: row.get(2)?,
                    task_id: row.get(3)?,
                    event_sequence: row.get(4)?,
                    reward_transaction_id: row.get(5)?,
                    expected_balance: row.get(6)?,
                })
            })
            .map_err(|error| storage_error("读取命令回执语义校验数据失败", error))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| storage_error("解析命令回执语义校验数据失败", error))?
    };

    let queue_replay = replay_queue_history(connection)?;
    let queue_replay_is_valid = queue_replay.failures.is_empty();
    failures.extend(queue_replay.failures.iter().cloned());

    let mut receipt_semantics = queue_replay_is_valid;
    for row in receipt_rows {
        let stored = match stored_receipt_from_json(&row.result_json) {
            Ok(stored) => stored,
            Err(error) => {
                receipt_semantics = false;
                failures.push(format!("命令回执 {} 无法解析：{error}", row.command_id));
                continue;
            }
        };
        let expected_current_task_id = match row.event_sequence {
            Some(sequence) => match queue_replay.current_task_id_by_sequence.get(&sequence) {
                Some(task_id) => task_id.clone(),
                None => {
                    receipt_semantics = false;
                    failures.push(format!(
                        "命令回执 {} 指向的事件序号 {} 未出现在队列历史中",
                        row.command_id, sequence
                    ));
                    continue;
                }
            },
            None => None,
        };
        let fields_match = stored.command_id == row.command_id
            && row.event_id.as_deref() == Some(stored.event_id.as_str())
            && row.task_id.as_deref() == Some(stored.task_id.as_str())
            && stored.reward_transaction_id == row.reward_transaction_id
            && stored.balance >= 0
            && row.expected_balance == Some(stored.balance)
            && stored.current_task_id == expected_current_task_id;
        if !fields_match {
            receipt_semantics = false;
            failures.push(format!(
                "命令回执 {} 的 JSON 与事件时点事实不一致",
                row.command_id
            ));
        }
    }

    let receipt_links = invalid_event_receipts == 0 && orphan_receipts == 0 && receipt_semantics;
    if invalid_event_receipts != 0 || orphan_receipts != 0 {
        failures.push(format!(
            "事件回执不一致 {invalid_event_receipts} 条，孤立回执 {orphan_receipts} 条"
        ));
    }

    let all_tasks_sql = format!("{TASK_SELECT} ORDER BY id ASC");
    let tasks = query_tasks(connection, &all_tasks_sql, [])?;
    let mut task_reward_net_values = true;
    let mut task_projection_matches_ledger = true;
    for task in &tasks {
        let history_root_is_valid: i64 = connection
            .query_row(
                "SELECT CASE WHEN
                    (SELECT COUNT(*) FROM task_events
                     WHERE task_id = ?1 AND event_type = 'created') = 1
                    AND (SELECT event_type FROM task_events
                         WHERE task_id = ?1 ORDER BY sequence ASC LIMIT 1) = 'created'
                 THEN 1 ELSE 0 END",
                [&task.id],
                |row| row.get(0),
            )
            .map_err(|error| storage_error("校验任务历史起点失败", error))?;
        if history_root_is_valid != 1 {
            task_projection_matches_ledger = false;
            failures.push(format!("任务 {} 缺少唯一的创建历史起点", task.id));
        }
        let net: i64 = connection
            .query_row(
                "SELECT COALESCE(SUM(reward.amount), 0)
                 FROM reward_transactions AS reward
                 JOIN task_events AS event ON event.id = reward.task_event_id
                 WHERE event.task_id = ?1",
                [&task.id],
                |row| row.get(0),
            )
            .map_err(|error| storage_error("校验任务奖励净值失败", error))?;
        if !(net == 0 || net == 1) {
            task_reward_net_values = false;
            failures.push(format!("任务 {} 的奖励净值为 {net}", task.id));
        }
        let expected_completed = net == 1;
        let projection_completed = task.status == TaskStatus::Completed
            && task.active_completion_event_id.is_some()
            && task.completed_at_ms.is_some();
        if expected_completed != projection_completed {
            task_projection_matches_ledger = false;
            failures.push(format!("任务 {} 的当前快照与奖励净值不一致", task.id));
        }
        if let Some(active_event_id) = &task.active_completion_event_id {
            let active_count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM task_events AS completion
                     WHERE completion.id = ?1
                       AND completion.task_id = ?2
                       AND completion.event_type = 'completed'
                       AND NOT EXISTS (
                           SELECT 1 FROM task_events AS undo
                           WHERE undo.event_type = 'completion_undone'
                             AND undo.reverses_event_id = completion.id
                       )",
                    params![active_event_id, task.id],
                    |row| row.get(0),
                )
                .map_err(|error| storage_error("校验有效完成事件失败", error))?;
            if active_count != 1 {
                task_projection_matches_ledger = false;
                failures.push(format!("任务 {} 的有效完成事件无效", task.id));
            }
        }
        match queue_replay.final_titles_by_task_id.get(&task.id) {
            Some(title) if title == &task.title => {}
            Some(title) => {
                task_projection_matches_ledger = false;
                failures.push(format!(
                    "任务 {} 的标题投影与事件历史不一致：投影为 {:?}，回放为 {:?}",
                    task.id, task.title, title
                ));
            }
            None => {
                task_projection_matches_ledger = false;
                failures.push(format!("任务 {} 缺少可回放的标题历史", task.id));
            }
        }
        let projection_deadline_is_valid = matches!(
            normalize_deadline_on(task.deadline_on.as_deref()),
            Ok(ref normalized) if normalized == &task.deadline_on
        );
        if !projection_deadline_is_valid {
            task_projection_matches_ledger = false;
            failures.push(format!("任务 {} 的截止日期投影不符合日期规范", task.id));
        }
        match queue_replay.final_deadlines_by_task_id.get(&task.id) {
            Some(deadline_on) if deadline_on == &task.deadline_on => {}
            Some(deadline_on) => {
                task_projection_matches_ledger = false;
                failures.push(format!(
                    "任务 {} 的截止日期投影与事件历史不一致：投影为 {:?}，回放为 {:?}",
                    task.id, task.deadline_on, deadline_on
                ));
            }
            None => {
                task_projection_matches_ledger = false;
                failures.push(format!("任务 {} 缺少可回放的截止日期历史", task.id));
            }
        }
    }

    if !event_reward_links {
        task_projection_matches_ledger = false;
    }
    if !queue_replay_is_valid {
        task_projection_matches_ledger = false;
    }

    let projected_queue_sql = format!(
        "{TASK_SELECT} WHERE status = 'pending' AND defer_until_ms IS NULL \
         ORDER BY queue_position ASC"
    );
    let projected_task_ids: Vec<String> = query_tasks(connection, &projected_queue_sql, [])?
        .into_iter()
        .map(|task| task.id)
        .collect();
    if projected_task_ids != queue_replay.final_task_ids {
        task_projection_matches_ledger = false;
        failures.push(format!(
            "任务队列投影与事件回放不一致：投影为 {:?}，事件回放为 {:?}",
            projected_task_ids, queue_replay.final_task_ids
        ));
    }

    Ok(IntegrityReport {
        schema_version: SCHEMA_VERSION,
        sqlite_quick_check,
        foreign_keys,
        reward_prefix_balances,
        event_reward_links,
        receipt_links,
        task_reward_net_values,
        task_projection_matches_ledger,
        failures,
    })
}

fn replay_queue_history(connection: &Connection) -> Result<QueueReplay, LedgerError> {
    let events_sql = format!("{EVENT_SELECT} ORDER BY sequence ASC");
    let events = query_events(connection, &events_sql, [])?;
    let mut queue = Vec::<String>::new();
    let mut current_task_id_by_sequence = HashMap::new();
    let mut titles_by_task_id = HashMap::new();
    let mut deadlines_by_task_id = HashMap::new();
    let mut failures = Vec::new();

    for event in events {
        let sequence = event.sequence.ok_or_else(|| {
            LedgerError::integrity(format!("任务事件 {} 缺少持久化序号", event.id))
        })?;
        apply_title_history(&event, &mut titles_by_task_id, &mut failures);
        apply_deadline_history(&event, &mut deadlines_by_task_id, &mut failures);
        match event.event_type {
            TaskEventType::Created
            | TaskEventType::CompletionUndone
            | TaskEventType::DueRecovered
            | TaskEventType::Recovered
            | TaskEventType::Reopened => {
                if queue.iter().any(|task_id| task_id == &event.task_id) {
                    failures.push(format!(
                        "任务事件 {}（{}）尝试重复入队任务 {}",
                        event.id,
                        event.event_type.as_storage(),
                        event.task_id
                    ));
                } else {
                    queue.push(event.task_id.clone());
                }
            }
            TaskEventType::Completed
            | TaskEventType::Deferred
            | TaskEventType::Blocked
            | TaskEventType::Abandoned => {
                if let Some(index) = queue.iter().position(|task_id| task_id == &event.task_id) {
                    queue.remove(index);
                } else {
                    failures.push(format!(
                        "任务事件 {}（{}）尝试移出不在队列中的任务 {}",
                        event.id,
                        event.event_type.as_storage(),
                        event.task_id
                    ));
                }
            }
            TaskEventType::QueueReordered => {
                apply_queue_reorder(&event, &mut queue, &mut failures);
            }
            TaskEventType::TitleUpdated | TaskEventType::DeadlineUpdated => {
                if !queue.iter().any(|task_id| task_id == &event.task_id) {
                    failures.push(format!(
                        "任务属性修改事件 {} 尝试修改不在即时待办队列中的任务 {}",
                        event.id, event.task_id
                    ));
                }
            }
        }
        current_task_id_by_sequence.insert(sequence, queue.first().cloned());
    }

    Ok(QueueReplay {
        current_task_id_by_sequence,
        final_task_ids: queue,
        final_titles_by_task_id: titles_by_task_id,
        final_deadlines_by_task_id: deadlines_by_task_id,
        failures,
    })
}

fn apply_deadline_history(
    event: &TaskEvent,
    deadlines_by_task_id: &mut HashMap<String, Option<String>>,
    failures: &mut Vec<String>,
) {
    if event.event_type == TaskEventType::Created {
        if deadlines_by_task_id
            .insert(event.task_id.clone(), None)
            .is_some()
        {
            failures.push(format!(
                "任务事件 {} 尝试重复初始化任务 {} 的截止日期",
                event.id, event.task_id
            ));
        }
        return;
    }
    if event.event_type != TaskEventType::DeadlineUpdated {
        return;
    }

    let metadata_keys_are_exact = event.metadata.as_object().is_some_and(|metadata| {
        metadata.len() == 2
            && metadata.contains_key("beforeDeadlineOn")
            && metadata.contains_key("afterDeadlineOn")
    });
    if !metadata_keys_are_exact {
        failures.push(format!(
            "截止日期修改事件 {} 的 metadata 必须且只能包含 beforeDeadlineOn 与 afterDeadlineOn",
            event.id
        ));
        return;
    }

    let metadata: DeadlineUpdatedMetadata = match serde_json::from_value(event.metadata.clone()) {
        Ok(metadata) => metadata,
        Err(error) => {
            failures.push(format!(
                "截止日期修改事件 {} 的 metadata 无效：{}",
                event.id, error
            ));
            return;
        }
    };
    let before_is_valid = matches!(
        normalize_deadline_on(metadata.before_deadline_on.0.as_deref()),
        Ok(ref normalized) if normalized == &metadata.before_deadline_on.0
    );
    let after_is_valid = matches!(
        normalize_deadline_on(metadata.after_deadline_on.0.as_deref()),
        Ok(ref normalized) if normalized == &metadata.after_deadline_on.0
    );
    if !before_is_valid || !after_is_valid {
        failures.push(format!(
            "截止日期修改事件 {} 的前后日期不符合日期规范",
            event.id
        ));
    }
    if metadata.before_deadline_on.0 == metadata.after_deadline_on.0 {
        failures.push(format!("截止日期修改事件 {} 的日期没有变化", event.id));
    }
    match deadlines_by_task_id.get(&event.task_id) {
        Some(current) if current == &metadata.before_deadline_on.0 => {}
        Some(current) => failures.push(format!(
            "截止日期修改事件 {} 的 beforeDeadlineOn 与历史不一致：历史为 {:?}，事件为 {:?}",
            event.id, current, metadata.before_deadline_on.0
        )),
        None => failures.push(format!(
            "截止日期修改事件 {} 发生在任务 {} 的创建事件之前",
            event.id, event.task_id
        )),
    }
    if after_is_valid {
        deadlines_by_task_id.insert(event.task_id.clone(), metadata.after_deadline_on.0);
    }
}

fn apply_title_history(
    event: &TaskEvent,
    titles_by_task_id: &mut HashMap<String, String>,
    failures: &mut Vec<String>,
) {
    if event.event_type == TaskEventType::Created {
        if titles_by_task_id.contains_key(&event.task_id) {
            failures.push(format!(
                "任务事件 {} 尝试重复初始化任务 {} 的标题",
                event.id, event.task_id
            ));
            return;
        }
        match normalize_title(&event.title_snapshot) {
            Ok(normalized) if normalized == event.title_snapshot => {
                titles_by_task_id.insert(event.task_id.clone(), normalized);
            }
            _ => failures.push(format!("创建事件 {} 的标题快照不符合标题规范", event.id)),
        }
        return;
    }

    if event.event_type == TaskEventType::TitleUpdated {
        let metadata: TitleUpdatedMetadata = match serde_json::from_value(event.metadata.clone()) {
            Ok(metadata) => metadata,
            Err(error) => {
                failures.push(format!(
                    "标题修改事件 {} 的 metadata 无效：{}",
                    event.id, error
                ));
                return;
            }
        };
        let before_is_valid = matches!(
            normalize_title(&metadata.before_title),
            Ok(ref normalized) if normalized == &metadata.before_title
        );
        let after_is_valid = matches!(
            normalize_title(&metadata.after_title),
            Ok(ref normalized) if normalized == &metadata.after_title
        );
        if !before_is_valid || !after_is_valid {
            failures.push(format!(
                "标题修改事件 {} 的前后标题不符合标题规范",
                event.id
            ));
        }
        if metadata.before_title == metadata.after_title {
            failures.push(format!("标题修改事件 {} 的标题没有变化", event.id));
        }
        match titles_by_task_id.get(&event.task_id) {
            Some(current) if current == &metadata.before_title => {}
            Some(current) => failures.push(format!(
                "标题修改事件 {} 的 beforeTitle 与历史不一致：历史为 {:?}，事件为 {:?}",
                event.id, current, metadata.before_title
            )),
            None => failures.push(format!(
                "标题修改事件 {} 发生在任务 {} 的创建事件之前",
                event.id, event.task_id
            )),
        }
        if event.title_snapshot != metadata.after_title {
            failures.push(format!(
                "标题修改事件 {} 的 titleSnapshot 与 afterTitle 不一致",
                event.id
            ));
        }
        if after_is_valid {
            titles_by_task_id.insert(event.task_id.clone(), metadata.after_title);
        }
        return;
    }

    match titles_by_task_id.get(&event.task_id) {
        Some(current) if current == &event.title_snapshot => {}
        Some(current) => failures.push(format!(
            "任务事件 {} 的标题快照与当时标题不一致：历史为 {:?}，事件为 {:?}",
            event.id, current, event.title_snapshot
        )),
        None => failures.push(format!(
            "任务事件 {} 发生在任务 {} 的创建事件之前",
            event.id, event.task_id
        )),
    }
}

fn apply_queue_reorder(event: &TaskEvent, queue: &mut Vec<String>, failures: &mut Vec<String>) {
    let metadata: QueueReorderMetadata = match serde_json::from_value(event.metadata.clone()) {
        Ok(metadata) => metadata,
        Err(error) => {
            failures.push(format!(
                "队列重排事件 {} 的 metadata 无效：{}",
                event.id, error
            ));
            return;
        }
    };
    let failure_count_before = failures.len();

    if metadata.moved_task_id.trim().is_empty() {
        failures.push(format!("队列重排事件 {} 的 movedTaskId 不能为空", event.id));
    }
    if metadata.moved_task_id != event.task_id {
        failures.push(format!(
            "队列重排事件 {} 的 movedTaskId {} 与事件 taskId {} 不一致",
            event.id, metadata.moved_task_id, event.task_id
        ));
    }
    if metadata.before_task_ids.len() < 2 || metadata.after_task_ids.len() < 2 {
        failures.push(format!(
            "队列重排事件 {} 必须包含至少两条调整前后任务",
            event.id
        ));
    }
    if metadata
        .before_task_ids
        .iter()
        .chain(metadata.after_task_ids.iter())
        .any(|task_id| task_id.trim().is_empty())
    {
        failures.push(format!("队列重排事件 {} 的任务 ID 不能为空", event.id));
    }

    let before_set: HashSet<&str> = metadata
        .before_task_ids
        .iter()
        .map(String::as_str)
        .collect();
    let after_set: HashSet<&str> = metadata.after_task_ids.iter().map(String::as_str).collect();
    if before_set.len() != metadata.before_task_ids.len() {
        failures.push(format!(
            "队列重排事件 {} 的 beforeTaskIds 存在重复任务",
            event.id
        ));
    }
    if after_set.len() != metadata.after_task_ids.len() {
        failures.push(format!(
            "队列重排事件 {} 的 afterTaskIds 存在重复任务",
            event.id
        ));
    }
    if metadata.before_task_ids.len() != metadata.after_task_ids.len() || before_set != after_set {
        failures.push(format!(
            "队列重排事件 {} 的 beforeTaskIds 与 afterTaskIds 不是同一任务集合",
            event.id
        ));
    }
    if metadata.before_task_ids != *queue {
        failures.push(format!(
            "队列重排事件 {} 的 beforeTaskIds 与事件发生前队列不一致：记录为 {:?}，回放为 {:?}",
            event.id, metadata.before_task_ids, queue
        ));
    }

    let old_index = metadata
        .before_task_ids
        .iter()
        .position(|task_id| task_id == &metadata.moved_task_id);
    let new_index = metadata
        .after_task_ids
        .iter()
        .position(|task_id| task_id == &metadata.moved_task_id);
    match (old_index, new_index) {
        (Some(old_index), Some(new_index)) if old_index != new_index => {}
        (Some(_), Some(_)) => failures.push(format!(
            "队列重排事件 {} 的 movedTaskId {} 实际位置没有变化",
            event.id, metadata.moved_task_id
        )),
        _ => failures.push(format!(
            "队列重排事件 {} 的 movedTaskId {} 未同时出现在调整前后队列中",
            event.id, metadata.moved_task_id
        )),
    }

    if failures.len() == failure_count_before {
        *queue = metadata.after_task_ids;
    }
}
