use super::{
    query_events, query_rewards, query_tasks, storage_error, EVENT_SELECT, REWARD_SELECT,
    SCHEMA_VERSION, TASK_SELECT,
};
use crate::ledger::{
    domain::{
        normalize_command_id, normalize_deadline_on, normalize_title, IntegrityReport, LedgerError,
        TaskEvent, TaskEventType, TaskStatus,
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

    let cascade_receipt_links = verify_cascade_completion_links(connection, &mut failures)?;
    let invalid_event_receipts: i64 = connection
        .query_row(
            "SELECT COUNT(*)
             FROM task_events AS event
             LEFT JOIN command_receipts AS receipt ON receipt.command_id = event.command_id
             LEFT JOIN reward_transactions AS reward ON reward.task_event_id = event.id
             WHERE (
                     receipt.command_id IS NULL
                     AND NOT (
                         event.event_type = 'subtask_completed'
                         AND event.command_id = 'cascade/' || event.id
                     )
                 )
                 OR (
                     receipt.command_id IS NOT NULL
                     AND (
                         event.command_id GLOB 'cascade/*'
                         OR receipt.command_type != CASE event.event_type
                       WHEN 'created' THEN 'capture_task'
                       WHEN 'subtask_created' THEN 'create_subtask'
                       WHEN 'completed' THEN 'complete_task'
                      WHEN 'subtask_completed' THEN 'complete_task'
                      WHEN 'completion_undone' THEN 'undo_completion'
                      WHEN 'subtask_completion_undone' THEN 'undo_completion'
                      WHEN 'abandoned' THEN 'delete_task'
                      WHEN 'queue_reordered' THEN 'reorder_tasks'
                       WHEN 'title_updated' THEN 'update_task_title'
                       WHEN 'deadline_updated' THEN 'update_task_deadline'
                       WHEN 'subtasks_reordered' THEN 'reorder_subtasks'
                       ELSE receipt.command_type
                      END
                     )
                 )",
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

    let receipt_links = invalid_event_receipts == 0
        && orphan_receipts == 0
        && receipt_semantics
        && cascade_receipt_links;
    if invalid_event_receipts != 0 || orphan_receipts != 0 {
        failures.push(format!(
            "事件回执不一致 {invalid_event_receipts} 条，孤立回执 {orphan_receipts} 条"
        ));
    }

    let all_tasks_sql = format!("{TASK_SELECT} ORDER BY id ASC");
    let tasks = query_tasks(connection, &all_tasks_sql, [])?;
    let task_hierarchy_valid = verify_task_hierarchy(&tasks, &mut failures);
    let mut task_reward_net_values = true;
    let mut task_projection_matches_ledger = true;
    for task in &tasks {
        let expected_root_event = if task.parent_task_id.is_some() {
            "subtask_created"
        } else {
            "created"
        };
        let history_root_is_valid: i64 = connection
            .query_row(
                "SELECT CASE WHEN
                    (SELECT COUNT(*) FROM task_events
                     WHERE task_id = ?1 AND event_type = ?2) = 1
                    AND (SELECT event_type FROM task_events
                         WHERE task_id = ?1 ORDER BY sequence ASC LIMIT 1) = ?2
                 THEN 1 ELSE 0 END",
                params![task.id, expected_root_event],
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
        let reward_net_is_valid = if task.parent_task_id.is_some() {
            net == 0
        } else {
            net == 0 || net == 1
        };
        if !reward_net_is_valid {
            task_reward_net_values = false;
            failures.push(format!("任务 {} 的奖励净值为 {net}", task.id));
        }
        let expected_completed = if task.parent_task_id.is_some() {
            task.active_completion_event_id.is_some()
        } else {
            net == 1
        };
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
                       AND completion.event_type = CASE
                           WHEN ?3 = 1 THEN 'subtask_completed' ELSE 'completed' END
                       AND NOT EXISTS (
                           SELECT 1 FROM task_events AS undo
                            WHERE undo.event_type = CASE
                                WHEN ?3 = 1 THEN 'subtask_completion_undone'
                                ELSE 'completion_undone' END
                             AND undo.reverses_event_id = completion.id
                       )",
                    params![
                        active_event_id,
                        task.id,
                        i64::from(task.parent_task_id.is_some())
                    ],
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
    if !task_hierarchy_valid {
        task_projection_matches_ledger = false;
    }

    let projected_queue_sql = format!(
        "{TASK_SELECT} WHERE status = 'pending' AND defer_until_ms IS NULL \
         AND parent_task_id IS NULL \
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
        task_hierarchy_valid,
        failures,
    })
}

fn verify_cascade_completion_links(
    connection: &Connection,
    failures: &mut Vec<String>,
) -> Result<bool, LedgerError> {
    let failure_count_before = failures.len();
    let events_sql = format!("{EVENT_SELECT} ORDER BY sequence ASC");
    let events = query_events(connection, &events_sql, [])?;
    let events_by_id: HashMap<&str, &TaskEvent> = events
        .iter()
        .map(|event| (event.id.as_str(), event))
        .collect();
    let receipts = {
        let mut statement = connection
            .prepare("SELECT command_id, command_type FROM command_receipts")
            .map_err(|error| storage_error("准备级联完成回执校验失败", error))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| storage_error("读取级联完成回执校验数据失败", error))?;
        rows.collect::<Result<HashMap<_, _>, _>>()
            .map_err(|error| storage_error("解析级联完成回执校验数据失败", error))?
    };
    let mut child_event_ids_by_parent_event = HashMap::<String, Vec<String>>::new();

    for event in &events {
        let metadata = event.metadata.as_object();
        let has_cascade_metadata = metadata.is_some_and(|metadata| {
            metadata.contains_key("cascadeParentEventId")
                || metadata.contains_key("cascadeCommandId")
        });
        let has_cascade_command = event.command_id.starts_with("cascade/");
        if !has_cascade_metadata && !has_cascade_command {
            continue;
        }

        let metadata_is_exact = metadata.is_some_and(|metadata| {
            metadata.len() == 4
                && metadata.contains_key("parentTaskId")
                && metadata.contains_key("parentTitle")
                && metadata.contains_key("cascadeParentEventId")
                && metadata.contains_key("cascadeCommandId")
        });
        if event.event_type != TaskEventType::SubtaskCompleted
            || event.command_id != format!("cascade/{}", event.id)
            || !metadata_is_exact
        {
            failures.push(format!(
                "级联子代办完成事件 {} 的类型、内部命令或 metadata 形态无效",
                event.id
            ));
            continue;
        }

        let metadata = metadata.expect("前置校验已确认 metadata 为对象");
        let parent_task_id = metadata
            .get("parentTaskId")
            .and_then(|value| value.as_str());
        let parent_event_id = metadata
            .get("cascadeParentEventId")
            .and_then(|value| value.as_str());
        let cascade_command_id = metadata
            .get("cascadeCommandId")
            .and_then(|value| value.as_str());
        let (Some(parent_task_id), Some(parent_event_id), Some(cascade_command_id)) =
            (parent_task_id, parent_event_id, cascade_command_id)
        else {
            failures.push(format!(
                "级联子代办完成事件 {} 的父任务、父事件或主命令关联缺失",
                event.id
            ));
            continue;
        };
        child_event_ids_by_parent_event
            .entry(parent_event_id.to_string())
            .or_default()
            .push(event.id.clone());

        let command_is_valid = matches!(
            normalize_command_id(cascade_command_id),
            Ok(ref normalized) if normalized == cascade_command_id
        );
        let parent_event_is_valid = events_by_id.get(parent_event_id).is_some_and(|parent| {
            parent.event_type == TaskEventType::Completed
                && parent.task_id == parent_task_id
                && parent.command_id == cascade_command_id
                && parent.occurred_at_ms == event.occurred_at_ms
                && matches!(
                    (event.sequence, parent.sequence),
                    (Some(child_sequence), Some(parent_sequence))
                        if child_sequence < parent_sequence
                )
        });
        let receipt_is_valid = receipts
            .get(cascade_command_id)
            .is_some_and(|command_type| command_type == "complete_task")
            && !receipts.contains_key(&event.command_id);
        if !command_is_valid || !parent_event_is_valid || !receipt_is_valid {
            failures.push(format!(
                "级联子代办完成事件 {} 未正确关联唯一的父完成事件与主命令回执",
                event.id
            ));
        }
    }

    for event in events
        .iter()
        .filter(|event| event.event_type == TaskEventType::Completed)
    {
        let metadata = event.metadata.as_object();
        let has_index_key =
            metadata.is_some_and(|metadata| metadata.contains_key("cascadeSubtaskEventIds"));
        let indexed_event_ids = metadata
            .and_then(|metadata| metadata.get("cascadeSubtaskEventIds"))
            .and_then(|value| value.as_array());
        let linked_event_ids = child_event_ids_by_parent_event.get(&event.id);
        match (has_index_key, indexed_event_ids, linked_event_ids) {
            (false, None, None) => {}
            (true, Some(indexed), Some(linked)) => {
                let indexed: Option<Vec<&str>> =
                    indexed.iter().map(|value| value.as_str()).collect();
                let linked: Vec<&str> = linked.iter().map(String::as_str).collect();
                let indexed_is_valid = indexed.as_ref().is_some_and(|indexed| {
                    !indexed.is_empty()
                        && indexed.iter().all(|event_id| !event_id.is_empty())
                        && indexed.iter().copied().collect::<HashSet<_>>().len() == indexed.len()
                });
                let metadata_is_exact = metadata.is_some_and(|metadata| metadata.len() == 1);
                if !metadata_is_exact
                    || !indexed_is_valid
                    || indexed.as_deref() != Some(linked.as_slice())
                {
                    failures.push(format!(
                        "父完成事件 {} 的级联子事件索引与实际伴随事件不一致",
                        event.id
                    ));
                }
            }
            _ => failures.push(format!(
                "父完成事件 {} 与级联子完成事件缺少双向索引",
                event.id
            )),
        }
    }

    Ok(failures.len() == failure_count_before)
}

fn verify_task_hierarchy(
    tasks: &[crate::ledger::domain::Task],
    failures: &mut Vec<String>,
) -> bool {
    let failure_count_before = failures.len();
    let by_id: HashMap<&str, &crate::ledger::domain::Task> =
        tasks.iter().map(|task| (task.id.as_str(), task)).collect();
    let mut positions_by_parent = HashMap::<&str, Vec<i64>>::new();

    for task in tasks {
        match task.parent_task_id.as_deref() {
            None => {
                if task.sibling_position.is_some() {
                    failures.push(format!("顶层任务 {} 不应携带同级位置", task.id));
                }
            }
            Some(parent_task_id) => {
                let shape_is_valid = task.queue_position.is_none()
                    && task.defer_until_ms.is_none()
                    && task.deadline_on.is_none()
                    && task.block_reason.is_none()
                    && matches!(task.sibling_position, Some(position) if position > 0)
                    && matches!(
                        task.status,
                        TaskStatus::Pending | TaskStatus::Completed | TaskStatus::Abandoned
                    );
                if !shape_is_valid {
                    failures.push(format!("子代办 {} 的任务投影形态无效", task.id));
                }
                match by_id.get(parent_task_id) {
                    Some(parent)
                        if parent.id != task.id
                            && parent.parent_task_id.is_none()
                            && parent.sibling_position.is_none() => {}
                    Some(_) => failures.push(format!(
                        "子代办 {} 的父项 {} 不是有效顶层代办",
                        task.id, parent_task_id
                    )),
                    None => failures.push(format!(
                        "子代办 {} 指向不存在的父项 {}",
                        task.id, parent_task_id
                    )),
                }
                if let Some(position) = task.sibling_position {
                    positions_by_parent
                        .entry(parent_task_id)
                        .or_default()
                        .push(position);
                }
            }
        }
    }

    for (parent_task_id, positions) in &mut positions_by_parent {
        positions.sort_unstable();
        let expected: Vec<i64> = (1..=positions.len() as i64).collect();
        if *positions != expected {
            failures.push(format!(
                "父代办 {} 的子代办位置不连续或重复：{:?}",
                parent_task_id, positions
            ));
        }
    }

    for parent in tasks
        .iter()
        .filter(|task| task.parent_task_id.is_none() && task.status == TaskStatus::Completed)
    {
        let incomplete_children: Vec<&str> = tasks
            .iter()
            .filter(|task| {
                task.parent_task_id.as_deref() == Some(parent.id.as_str())
                    && !matches!(task.status, TaskStatus::Completed | TaskStatus::Abandoned)
            })
            .map(|task| task.id.as_str())
            .collect();
        if !incomplete_children.is_empty() {
            failures.push(format!(
                "已完成父代办 {} 仍有未完成子代办 {:?}",
                parent.id, incomplete_children
            ));
        }
    }

    failures.len() == failure_count_before
}

fn replay_queue_history(connection: &Connection) -> Result<QueueReplay, LedgerError> {
    let events_sql = format!("{EVENT_SELECT} ORDER BY sequence ASC");
    let events = query_events(connection, &events_sql, [])?;
    let tasks_sql = format!("{TASK_SELECT} ORDER BY id ASC");
    let task_parent_by_id: HashMap<String, Option<String>> =
        query_tasks(connection, &tasks_sql, [])?
            .into_iter()
            .map(|task| (task.id, task.parent_task_id))
            .collect();
    let mut queue = Vec::<String>::new();
    let mut current_task_id_by_sequence = HashMap::new();
    let mut titles_by_task_id = HashMap::new();
    let mut deadlines_by_task_id = HashMap::new();
    let mut failures = Vec::new();

    for event in events {
        let sequence = event.sequence.ok_or_else(|| {
            LedgerError::integrity(format!("任务事件 {} 缺少持久化序号", event.id))
        })?;
        let is_subtask = task_parent_by_id
            .get(&event.task_id)
            .is_some_and(Option::is_some);
        validate_parent_metadata(
            &event,
            task_parent_by_id
                .get(&event.task_id)
                .and_then(Option::as_deref),
            &titles_by_task_id,
            &mut failures,
        );
        apply_title_history(&event, &mut titles_by_task_id, &mut failures);
        apply_deadline_history(&event, &mut deadlines_by_task_id, &mut failures);
        match event.event_type {
            TaskEventType::Created
            | TaskEventType::CompletionUndone
            | TaskEventType::DueRecovered
            | TaskEventType::Recovered
            | TaskEventType::Reopened => {
                if is_subtask {
                    failures.push(format!(
                        "子代办事件 {} 错误使用了顶层事件类型 {}",
                        event.id,
                        event.event_type.as_storage()
                    ));
                } else if queue.iter().any(|task_id| task_id == &event.task_id) {
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
                if is_subtask {
                    if event.event_type != TaskEventType::Abandoned {
                        failures.push(format!(
                            "子代办事件 {} 错误使用了顶层事件类型 {}",
                            event.id,
                            event.event_type.as_storage()
                        ));
                    }
                } else if let Some(index) =
                    queue.iter().position(|task_id| task_id == &event.task_id)
                {
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
            TaskEventType::SubtaskCreated
            | TaskEventType::SubtaskCompleted
            | TaskEventType::SubtaskCompletionUndone
            | TaskEventType::SubtasksReordered => {
                if !is_subtask {
                    failures.push(format!(
                        "顶层任务事件 {} 错误使用了子代办事件类型 {}",
                        event.id,
                        event.event_type.as_storage()
                    ));
                }
            }
            TaskEventType::TitleUpdated | TaskEventType::DeadlineUpdated => {
                if event.event_type == TaskEventType::DeadlineUpdated && is_subtask {
                    failures.push(format!("子代办事件 {} 不应修改期限", event.id));
                } else if !is_subtask && !queue.iter().any(|task_id| task_id == &event.task_id) {
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

fn validate_parent_metadata(
    event: &TaskEvent,
    parent_task_id: Option<&str>,
    titles_by_task_id: &HashMap<String, String>,
    failures: &mut Vec<String>,
) {
    let Some(parent_task_id) = parent_task_id else {
        return;
    };
    let recorded_parent_id = event
        .metadata
        .get("parentTaskId")
        .and_then(|value| value.as_str());
    let recorded_parent_title = event
        .metadata
        .get("parentTitle")
        .and_then(|value| value.as_str());
    if recorded_parent_id != Some(parent_task_id) {
        failures.push(format!(
            "子代办事件 {} 的 parentTaskId 与任务投影不一致",
            event.id
        ));
    }
    match titles_by_task_id.get(parent_task_id) {
        Some(title) if recorded_parent_title == Some(title.as_str()) => {}
        Some(title) => failures.push(format!(
            "子代办事件 {} 的 parentTitle 与当时父标题不一致：历史为 {:?}，事件为 {:?}",
            event.id, title, recorded_parent_title
        )),
        None => failures.push(format!(
            "子代办事件 {} 发生在父代办 {} 创建之前",
            event.id, parent_task_id
        )),
    }
}

fn apply_deadline_history(
    event: &TaskEvent,
    deadlines_by_task_id: &mut HashMap<String, Option<String>>,
    failures: &mut Vec<String>,
) {
    if matches!(
        event.event_type,
        TaskEventType::Created | TaskEventType::SubtaskCreated
    ) {
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
    if matches!(
        event.event_type,
        TaskEventType::Created | TaskEventType::SubtaskCreated
    ) {
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
