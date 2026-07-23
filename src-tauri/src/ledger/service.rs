use super::domain::{
    capture_task_transition, complete_task_transition, create_subtask_transition,
    delete_task_transition, normalize_command_id, normalize_deadline_on, normalize_title,
    reorder_queue_transition, reorder_subtasks_transition, undo_completion_transition,
    update_task_deadline_transition, update_task_title_transition, validate_fact_range,
    CascadedSubtaskCompletion, Clock, IdGenerator, IntegrityReport, LedgerError, LedgerMutation,
    LedgerSnapshot, MutationContext, MutationReceipt, StoredReceipt, Task, TaskEvent, TaskStatus,
    WeeklyFacts,
};
use std::collections::HashSet;

const MAX_REORDER_TASKS: usize = 10_000;

pub trait LedgerStore: Send {
    fn replay_receipt(
        &self,
        command_id: &str,
        request_fingerprint: &str,
    ) -> Result<Option<MutationReceipt>, LedgerError>;

    fn queue(&self) -> Result<Vec<Task>, LedgerError>;
    fn subtasks_for_parent(&self, parent_task_id: &str) -> Result<Vec<Task>, LedgerError>;
    fn task_by_id(&self, task_id: &str) -> Result<Option<Task>, LedgerError>;
    fn event_by_id(&self, event_id: &str) -> Result<Option<TaskEvent>, LedgerError>;
    fn reward_balance(&self) -> Result<i64, LedgerError>;

    fn commit_transition(
        &mut self,
        command_type: &str,
        request_fingerprint: &str,
        mutation: LedgerMutation,
    ) -> Result<MutationReceipt, LedgerError>;

    fn snapshot(&mut self) -> Result<LedgerSnapshot, LedgerError>;
    fn weekly_facts(&mut self, from_ms: i64, to_ms: i64) -> Result<WeeklyFacts, LedgerError>;
    fn verify_integrity(&mut self) -> Result<IntegrityReport, LedgerError>;
}

pub struct TaskService<S, C, I> {
    store: S,
    clock: C,
    ids: I,
}

impl<S, C, I> TaskService<S, C, I>
where
    S: LedgerStore,
    C: Clock,
    I: IdGenerator,
{
    pub fn new(store: S, clock: C, ids: I) -> Self {
        Self { store, clock, ids }
    }

    pub fn capture_task(
        &mut self,
        command_id: &str,
        title: &str,
    ) -> Result<MutationReceipt, LedgerError> {
        let command_id = normalize_command_id(command_id)?;
        let title = normalize_title(title)?;
        let fingerprint = request_fingerprint("capture_task", &[&title]);
        if let Some(receipt) = self.store.replay_receipt(&command_id, &fingerprint)? {
            return Ok(receipt);
        }

        let mutation = capture_task_transition(
            title,
            self.ids.next_id(),
            MutationContext {
                command_id,
                event_id: self.ids.next_id(),
                reward_transaction_id: None,
                occurred_at_ms: self.clock.now_ms(),
            },
        );
        self.store
            .commit_transition("capture_task", &fingerprint, mutation)
    }

    pub fn create_subtask(
        &mut self,
        command_id: &str,
        parent_task_id: &str,
        title: &str,
    ) -> Result<MutationReceipt, LedgerError> {
        let command_id = normalize_command_id(command_id)?;
        let parent_task_id = normalize_entity_id("parentTaskId", parent_task_id)?;
        let title = normalize_title(title)?;
        let fingerprint = create_subtask_fingerprint(&parent_task_id, &title);
        if let Some(receipt) = self.store.replay_receipt(&command_id, &fingerprint)? {
            return Ok(receipt);
        }

        let parent = self
            .store
            .task_by_id(&parent_task_id)?
            .ok_or_else(|| LedgerError::not_found(format!("父代办不存在：{parent_task_id}")))?;
        let mutation = create_subtask_transition(
            &parent,
            title,
            self.ids.next_id(),
            MutationContext {
                command_id,
                event_id: self.ids.next_id(),
                reward_transaction_id: None,
                occurred_at_ms: self.clock.now_ms(),
            },
        )?;
        self.store
            .commit_transition("create_subtask", &fingerprint, mutation)
    }

    pub fn complete_task(
        &mut self,
        command_id: &str,
        task_id: &str,
    ) -> Result<MutationReceipt, LedgerError> {
        let command_id = normalize_command_id(command_id)?;
        let task_id = normalize_entity_id("taskId", task_id)?;
        let fingerprint = request_fingerprint("complete_task", &[&task_id]);
        if let Some(receipt) = self.store.replay_receipt(&command_id, &fingerprint)? {
            return Ok(receipt);
        }

        let task = self
            .store
            .task_by_id(&task_id)?
            .ok_or_else(|| LedgerError::not_found(format!("任务不存在：{task_id}")))?;
        let parent = self.parent_for_task(&task)?;
        let subtasks = if parent.is_none() {
            self.store.subtasks_for_parent(&task.id)?
        } else {
            Vec::new()
        };
        let cascaded_subtasks = subtasks
            .iter()
            .filter(|subtask| subtask.status == TaskStatus::Pending)
            .map(|subtask| CascadedSubtaskCompletion {
                task_id: subtask.id.clone(),
                event_id: self.ids.next_id(),
            })
            .collect();
        let mutation = complete_task_transition(
            &task,
            parent.as_ref(),
            &subtasks,
            cascaded_subtasks,
            MutationContext {
                command_id,
                event_id: self.ids.next_id(),
                reward_transaction_id: task.parent_task_id.is_none().then(|| self.ids.next_id()),
                occurred_at_ms: self.clock.now_ms(),
            },
        )?;
        self.store
            .commit_transition("complete_task", &fingerprint, mutation)
    }

    pub fn update_task_title(
        &mut self,
        command_id: &str,
        task_id: &str,
        title: &str,
    ) -> Result<MutationReceipt, LedgerError> {
        let command_id = normalize_command_id(command_id)?;
        let task_id = normalize_entity_id("taskId", task_id)?;
        let title = normalize_title(title)?;
        let fingerprint = request_fingerprint("update_task_title", &[&task_id, &title]);
        if let Some(receipt) = self.store.replay_receipt(&command_id, &fingerprint)? {
            return Ok(receipt);
        }

        let task = self
            .store
            .task_by_id(&task_id)?
            .ok_or_else(|| LedgerError::not_found(format!("任务不存在：{task_id}")))?;
        let parent = self.parent_for_task(&task)?;
        let mutation = update_task_title_transition(
            &task,
            parent.as_ref(),
            title,
            MutationContext {
                command_id,
                event_id: self.ids.next_id(),
                reward_transaction_id: None,
                occurred_at_ms: self.clock.now_ms(),
            },
        )?;
        self.store
            .commit_transition("update_task_title", &fingerprint, mutation)
    }

    pub fn update_task_deadline(
        &mut self,
        command_id: &str,
        task_id: &str,
        deadline_on: Option<&str>,
    ) -> Result<MutationReceipt, LedgerError> {
        let command_id = normalize_command_id(command_id)?;
        let task_id = normalize_entity_id("taskId", task_id)?;
        let deadline_on = normalize_deadline_on(deadline_on)?;
        let fingerprint = serde_json::to_string(&(
            "update_task_deadline",
            task_id.as_str(),
            deadline_on.as_deref(),
        ))
        .expect("截止日期命令指纹必须可序列化");
        if let Some(receipt) = self.store.replay_receipt(&command_id, &fingerprint)? {
            return Ok(receipt);
        }

        let task = self
            .store
            .task_by_id(&task_id)?
            .ok_or_else(|| LedgerError::not_found(format!("任务不存在：{task_id}")))?;
        let mutation = update_task_deadline_transition(
            &task,
            deadline_on,
            MutationContext {
                command_id,
                event_id: self.ids.next_id(),
                reward_transaction_id: None,
                occurred_at_ms: self.clock.now_ms(),
            },
        )?;
        self.store
            .commit_transition("update_task_deadline", &fingerprint, mutation)
    }

    pub fn delete_task(
        &mut self,
        command_id: &str,
        task_id: &str,
    ) -> Result<MutationReceipt, LedgerError> {
        let command_id = normalize_command_id(command_id)?;
        let task_id = normalize_entity_id("taskId", task_id)?;
        let fingerprint = request_fingerprint("delete_task", &[&task_id]);
        if let Some(receipt) = self.store.replay_receipt(&command_id, &fingerprint)? {
            return Ok(receipt);
        }

        let task = self
            .store
            .task_by_id(&task_id)?
            .ok_or_else(|| LedgerError::not_found(format!("任务不存在：{task_id}")))?;
        let parent = self.parent_for_task(&task)?;
        let mutation = delete_task_transition(
            &task,
            parent.as_ref(),
            MutationContext {
                command_id,
                event_id: self.ids.next_id(),
                reward_transaction_id: None,
                occurred_at_ms: self.clock.now_ms(),
            },
        )?;
        self.store
            .commit_transition("delete_task", &fingerprint, mutation)
    }

    pub fn reorder_tasks(
        &mut self,
        command_id: &str,
        moved_task_id: &str,
        expected_task_ids: &[String],
        ordered_task_ids: &[String],
    ) -> Result<MutationReceipt, LedgerError> {
        let command_id = normalize_command_id(command_id)?;
        let moved_task_id = normalize_entity_id("movedTaskId", moved_task_id)?;
        let expected_task_ids = normalize_task_id_list("expectedTaskIds", expected_task_ids)?;
        let ordered_task_ids = normalize_task_id_list("orderedTaskIds", ordered_task_ids)?;
        let expected_json = serde_json::to_string(&expected_task_ids)
            .map_err(|error| LedgerError::validation(format!("序列化原顺序失败：{error}")))?;
        let ordered_json = serde_json::to_string(&ordered_task_ids)
            .map_err(|error| LedgerError::validation(format!("序列化新顺序失败：{error}")))?;
        let fingerprint = request_fingerprint(
            "reorder_tasks",
            &[&moved_task_id, &expected_json, &ordered_json],
        );
        if let Some(receipt) = self.store.replay_receipt(&command_id, &fingerprint)? {
            return Ok(receipt);
        }

        let queue = self.store.queue()?;
        let mutation = reorder_queue_transition(
            &queue,
            moved_task_id,
            expected_task_ids,
            ordered_task_ids,
            MutationContext {
                command_id,
                event_id: self.ids.next_id(),
                reward_transaction_id: None,
                occurred_at_ms: self.clock.now_ms(),
            },
        )?;
        self.store
            .commit_transition("reorder_tasks", &fingerprint, mutation)
    }

    pub fn reorder_subtasks(
        &mut self,
        command_id: &str,
        parent_task_id: &str,
        moved_task_id: &str,
        expected_task_ids: &[String],
        ordered_task_ids: &[String],
    ) -> Result<MutationReceipt, LedgerError> {
        let command_id = normalize_command_id(command_id)?;
        let parent_task_id = normalize_entity_id("parentTaskId", parent_task_id)?;
        let moved_task_id = normalize_entity_id("movedTaskId", moved_task_id)?;
        let expected_task_ids = normalize_task_id_list("expectedTaskIds", expected_task_ids)?;
        let ordered_task_ids = normalize_task_id_list("orderedTaskIds", ordered_task_ids)?;
        let fingerprint = reorder_subtasks_fingerprint(
            &parent_task_id,
            &moved_task_id,
            &expected_task_ids,
            &ordered_task_ids,
        )?;
        if let Some(receipt) = self.store.replay_receipt(&command_id, &fingerprint)? {
            return Ok(receipt);
        }

        let parent = self
            .store
            .task_by_id(&parent_task_id)?
            .ok_or_else(|| LedgerError::not_found(format!("父代办不存在：{parent_task_id}")))?;
        let subtasks = self.store.subtasks_for_parent(&parent_task_id)?;
        let mutation = reorder_subtasks_transition(
            &parent,
            &subtasks,
            moved_task_id,
            expected_task_ids,
            ordered_task_ids,
            MutationContext {
                command_id,
                event_id: self.ids.next_id(),
                reward_transaction_id: None,
                occurred_at_ms: self.clock.now_ms(),
            },
        )?;
        self.store
            .commit_transition("reorder_subtasks", &fingerprint, mutation)
    }

    pub fn undo_completion(
        &mut self,
        command_id: &str,
        completion_event_id: &str,
    ) -> Result<MutationReceipt, LedgerError> {
        let command_id = normalize_command_id(command_id)?;
        let completion_event_id = normalize_entity_id("completionEventId", completion_event_id)?;
        let fingerprint = request_fingerprint("undo_completion", &[completion_event_id.as_str()]);
        if let Some(receipt) = self.store.replay_receipt(&command_id, &fingerprint)? {
            return Ok(receipt);
        }

        let completion_event = self
            .store
            .event_by_id(&completion_event_id)?
            .ok_or_else(|| {
                LedgerError::new(
                    "TASK_EVENT_NOT_FOUND",
                    format!("完成事件不存在：{completion_event_id}"),
                )
            })?;
        let task = self
            .store
            .task_by_id(&completion_event.task_id)?
            .ok_or_else(|| {
                LedgerError::integrity(format!(
                    "完成事件 {} 指向了不存在的任务",
                    completion_event.id
                ))
            })?;
        let parent = self.parent_for_task(&task)?;
        let mutation = undo_completion_transition(
            &task,
            parent.as_ref(),
            &completion_event,
            MutationContext {
                command_id,
                event_id: self.ids.next_id(),
                reward_transaction_id: task.parent_task_id.is_none().then(|| self.ids.next_id()),
                occurred_at_ms: self.clock.now_ms(),
            },
            self.store.reward_balance()?,
        )?;
        self.store
            .commit_transition("undo_completion", &fingerprint, mutation)
    }

    pub fn snapshot(&mut self) -> Result<LedgerSnapshot, LedgerError> {
        self.store.snapshot()
    }

    pub fn weekly_facts(&mut self, from_ms: i64, to_ms: i64) -> Result<WeeklyFacts, LedgerError> {
        validate_fact_range(from_ms, to_ms)?;
        self.store.weekly_facts(from_ms, to_ms)
    }

    pub fn verify_integrity(&mut self) -> Result<IntegrityReport, LedgerError> {
        self.store.verify_integrity()
    }

    fn parent_for_task(&self, task: &Task) -> Result<Option<Task>, LedgerError> {
        let Some(parent_task_id) = task.parent_task_id.as_deref() else {
            return Ok(None);
        };
        self.store
            .task_by_id(parent_task_id)?
            .map(Some)
            .ok_or_else(|| {
                LedgerError::integrity(format!(
                    "子代办 {} 指向了不存在的父代办 {}",
                    task.id, parent_task_id
                ))
            })
    }

    #[cfg(test)]
    pub(crate) fn into_store(self) -> S {
        self.store
    }
}

fn normalize_entity_id(field: &str, value: &str) -> Result<String, LedgerError> {
    let normalized = value.trim();
    if normalized.is_empty() || normalized.len() > 128 {
        return Err(LedgerError::validation(format!(
            "{field} 不能为空且不能超过 128 个字节"
        )));
    }
    if !normalized
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(LedgerError::validation(format!("{field} 包含不允许的字符")));
    }
    Ok(normalized.to_string())
}

fn normalize_task_id_list(field: &str, values: &[String]) -> Result<Vec<String>, LedgerError> {
    if values.len() < 2 || values.len() > MAX_REORDER_TASKS {
        return Err(LedgerError::validation(format!(
            "{field} 必须包含 2 到 {MAX_REORDER_TASKS} 个任务 ID"
        )));
    }
    let normalized = values
        .iter()
        .map(|value| normalize_entity_id(field, value))
        .collect::<Result<Vec<_>, _>>()?;
    let unique: HashSet<&str> = normalized.iter().map(String::as_str).collect();
    if unique.len() != normalized.len() {
        return Err(LedgerError::validation(format!(
            "{field} 不能包含重复任务 ID"
        )));
    }
    Ok(normalized)
}

fn request_fingerprint(command_type: &str, arguments: &[&str]) -> String {
    serde_json::to_string(&(command_type, arguments)).expect("固定字符串组成的命令指纹必须可序列化")
}

fn create_subtask_fingerprint(parent_task_id: &str, title: &str) -> String {
    request_fingerprint("create_subtask", &[parent_task_id, title])
}

fn reorder_subtasks_fingerprint(
    parent_task_id: &str,
    moved_task_id: &str,
    expected_task_ids: &[String],
    ordered_task_ids: &[String],
) -> Result<String, LedgerError> {
    let expected_json = serde_json::to_string(expected_task_ids)
        .map_err(|error| LedgerError::validation(format!("序列化子代办原顺序失败：{error}")))?;
    let ordered_json = serde_json::to_string(ordered_task_ids)
        .map_err(|error| LedgerError::validation(format!("序列化子代办新顺序失败：{error}")))?;
    Ok(request_fingerprint(
        "reorder_subtasks",
        &[parent_task_id, moved_task_id, &expected_json, &ordered_json],
    ))
}

pub(crate) fn stored_receipt_from_json(value: &str) -> Result<StoredReceipt, LedgerError> {
    serde_json::from_str(value)
        .map_err(|error| LedgerError::integrity(format!("命令回执 JSON 损坏：{error}")))
}

#[cfg(test)]
mod tests {
    use super::{create_subtask_fingerprint, reorder_subtasks_fingerprint};

    #[test]
    fn create_subtask_fingerprint_keeps_stable_command_and_parent_scope() {
        let fingerprint = create_subtask_fingerprint("parent-1", "整理本周问题");

        assert_eq!(
            fingerprint,
            r#"["create_subtask",["parent-1","整理本周问题"]]"#
        );
        assert_ne!(
            fingerprint,
            create_subtask_fingerprint("parent-2", "整理本周问题")
        );
    }

    #[test]
    fn reorder_subtasks_fingerprint_keeps_stable_command_and_full_order() {
        let expected = vec!["subtask-a".to_string(), "subtask-b".to_string()];
        let ordered = vec!["subtask-b".to_string(), "subtask-a".to_string()];
        let fingerprint =
            reorder_subtasks_fingerprint("parent-1", "subtask-b", &expected, &ordered)
                .expect("有效子代办顺序应能生成请求指纹");

        assert_eq!(
            fingerprint,
            r#"["reorder_subtasks",["parent-1","subtask-b","[\"subtask-a\",\"subtask-b\"]","[\"subtask-b\",\"subtask-a\"]"]]"#
        );
        assert_ne!(
            fingerprint,
            reorder_subtasks_fingerprint("parent-2", "subtask-b", &expected, &ordered)
                .expect("不同父项也应能生成请求指纹")
        );
    }
}
