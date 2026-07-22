use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    error::Error,
    fmt,
    time::{SystemTime, UNIX_EPOCH},
};

pub const COMPLETION_REWARD: i64 = 1;
pub const MAX_COMMAND_ID_LENGTH: usize = 128;
pub const MAX_TASK_TITLE_LENGTH: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Blocked,
    Completed,
    Abandoned,
}

impl TaskStatus {
    pub(crate) fn as_storage(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Blocked => "blocked",
            Self::Completed => "completed",
            Self::Abandoned => "abandoned",
        }
    }

    pub(crate) fn from_storage(value: &str) -> Result<Self, LedgerError> {
        match value {
            "pending" => Ok(Self::Pending),
            "blocked" => Ok(Self::Blocked),
            "completed" => Ok(Self::Completed),
            "abandoned" => Ok(Self::Abandoned),
            _ => Err(LedgerError::integrity(format!("未知任务状态：{value}"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskEventType {
    Created,
    Completed,
    CompletionUndone,
    Deferred,
    DueRecovered,
    Blocked,
    Recovered,
    Abandoned,
    Reopened,
    QueueReordered,
    TitleUpdated,
    DeadlineUpdated,
}

impl TaskEventType {
    pub(crate) fn as_storage(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Completed => "completed",
            Self::CompletionUndone => "completion_undone",
            Self::Deferred => "deferred",
            Self::DueRecovered => "due_recovered",
            Self::Blocked => "blocked",
            Self::Recovered => "recovered",
            Self::Abandoned => "abandoned",
            Self::Reopened => "reopened",
            Self::QueueReordered => "queue_reordered",
            Self::TitleUpdated => "title_updated",
            Self::DeadlineUpdated => "deadline_updated",
        }
    }

    pub(crate) fn from_storage(value: &str) -> Result<Self, LedgerError> {
        match value {
            "created" => Ok(Self::Created),
            "completed" => Ok(Self::Completed),
            "completion_undone" => Ok(Self::CompletionUndone),
            "deferred" => Ok(Self::Deferred),
            "due_recovered" => Ok(Self::DueRecovered),
            "blocked" => Ok(Self::Blocked),
            "recovered" => Ok(Self::Recovered),
            "abandoned" => Ok(Self::Abandoned),
            "reopened" => Ok(Self::Reopened),
            "queue_reordered" => Ok(Self::QueueReordered),
            "title_updated" => Ok(Self::TitleUpdated),
            "deadline_updated" => Ok(Self::DeadlineUpdated),
            _ => Err(LedgerError::integrity(format!("未知任务事件类型：{value}"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RewardType {
    TaskCompletion,
    CompletionUndo,
}

impl RewardType {
    pub(crate) fn as_storage(self) -> &'static str {
        match self {
            Self::TaskCompletion => "task_completion",
            Self::CompletionUndo => "completion_undo",
        }
    }

    pub(crate) fn from_storage(value: &str) -> Result<Self, LedgerError> {
        match value {
            "task_completion" => Ok(Self::TaskCompletion),
            "completion_undo" => Ok(Self::CompletionUndo),
            _ => Err(LedgerError::integrity(format!("未知奖励交易类型：{value}"))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub title: String,
    pub status: TaskStatus,
    pub queue_position: Option<i64>,
    pub defer_until_ms: Option<i64>,
    pub deadline_on: Option<String>,
    pub block_reason: Option<String>,
    pub abandon_reason: Option<String>,
    pub completed_at_ms: Option<i64>,
    pub active_completion_event_id: Option<String>,
    pub version: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskEvent {
    pub sequence: Option<i64>,
    pub id: String,
    pub command_id: String,
    pub task_id: String,
    pub title_snapshot: String,
    pub event_type: TaskEventType,
    pub occurred_at_ms: i64,
    pub reason: Option<String>,
    pub metadata: serde_json::Value,
    pub reverses_event_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RewardTransaction {
    pub sequence: Option<i64>,
    pub id: String,
    pub task_event_id: String,
    pub reward_type: RewardType,
    pub amount: i64,
    pub balance_after: i64,
    pub occurred_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredReceipt {
    pub command_id: String,
    pub task_id: String,
    pub event_id: String,
    pub reward_transaction_id: Option<String>,
    pub current_task_id: Option<String>,
    pub balance: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationReceipt {
    pub command_id: String,
    pub replayed: bool,
    pub task_id: String,
    pub event_id: String,
    pub reward_transaction_id: Option<String>,
    pub current_task_id: Option<String>,
    pub balance: i64,
}

impl StoredReceipt {
    pub(crate) fn into_result(self, replayed: bool) -> MutationReceipt {
        MutationReceipt {
            command_id: self.command_id,
            replayed,
            task_id: self.task_id,
            event_id: self.event_id,
            reward_transaction_id: self.reward_transaction_id,
            current_task_id: self.current_task_id,
            balance: self.balance,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerSnapshot {
    pub schema_version: i64,
    pub current_task: Option<Task>,
    pub queue: Vec<Task>,
    pub completed: Vec<Task>,
    pub events: Vec<TaskEvent>,
    pub rewards: Vec<RewardTransaction>,
    pub balance: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeeklyFacts {
    pub from_ms: i64,
    pub to_ms: i64,
    pub effective_completions: Vec<TaskEvent>,
    pub ongoing_tasks: Vec<Task>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrityReport {
    pub schema_version: i64,
    pub sqlite_quick_check: bool,
    pub foreign_keys: bool,
    pub reward_prefix_balances: bool,
    pub event_reward_links: bool,
    pub receipt_links: bool,
    pub task_reward_net_values: bool,
    pub task_projection_matches_ledger: bool,
    pub failures: Vec<String>,
}

impl IntegrityReport {
    pub fn is_ok(&self) -> bool {
        self.sqlite_quick_check
            && self.foreign_keys
            && self.reward_prefix_balances
            && self.event_reward_links
            && self.receipt_links
            && self.task_reward_net_values
            && self.task_projection_matches_ledger
            && self.failures.is_empty()
    }
}

#[derive(Debug, Clone)]
pub enum TaskWrite {
    Insert {
        task: Task,
        place_at_tail: bool,
    },
    Update {
        expected_version: i64,
        task: Task,
        place_at_tail: bool,
    },
    ReorderQueue {
        expected_queue: Vec<QueuedTaskVersion>,
        ordered_task_ids: Vec<String>,
        occurred_at_ms: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedTaskVersion {
    pub task_id: String,
    pub expected_version: i64,
    pub expected_position: i64,
}

#[derive(Debug, Clone)]
pub struct LedgerMutation {
    pub task_write: TaskWrite,
    pub event: TaskEvent,
    pub reward: Option<RewardMutation>,
}

impl LedgerMutation {
    pub(crate) fn task_id(&self) -> &str {
        &self.event.task_id
    }
}

#[derive(Debug, Clone)]
pub struct RewardMutation {
    pub id: String,
    pub task_event_id: String,
    pub reward_type: RewardType,
    pub amount: i64,
    pub occurred_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct MutationContext {
    pub command_id: String,
    pub event_id: String,
    pub reward_transaction_id: Option<String>,
    pub occurred_at_ms: i64,
}

pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
            .unwrap_or(0)
    }
}

pub trait IdGenerator: Send + Sync {
    fn next_id(&self) -> String;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct UuidIdGenerator;

impl IdGenerator for UuidIdGenerator {
    fn next_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerError {
    code: &'static str,
    message: String,
}

impl LedgerError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::new("VALIDATION_ERROR", message)
    }

    pub fn invalid_state(message: impl Into<String>) -> Self {
        Self::new("INVALID_TASK_STATE", message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new("TASK_NOT_FOUND", message)
    }

    pub fn command_conflict(message: impl Into<String>) -> Self {
        Self::new("COMMAND_ID_CONFLICT", message)
    }

    pub fn concurrency_conflict(message: impl Into<String>) -> Self {
        Self::new("CONCURRENT_MODIFICATION", message)
    }

    pub fn integrity(message: impl Into<String>) -> Self {
        Self::new("DATA_INTEGRITY_ERROR", message)
    }

    pub fn unsupported_schema(message: impl Into<String>) -> Self {
        Self::new("UNSUPPORTED_SCHEMA_VERSION", message)
    }

    pub fn storage(message: impl Into<String>) -> Self {
        Self::new("STORAGE_ERROR", message)
    }

    pub fn injected(message: impl Into<String>) -> Self {
        Self::new("INJECTED_FAILURE", message)
    }

    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for LedgerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl Error for LedgerError {}

pub fn normalize_command_id(value: &str) -> Result<String, LedgerError> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return Err(LedgerError::validation("commandId 不能为空"));
    }
    if normalized.len() > MAX_COMMAND_ID_LENGTH {
        return Err(LedgerError::validation(format!(
            "commandId 不能超过 {MAX_COMMAND_ID_LENGTH} 个字节"
        )));
    }
    if !normalized
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.'))
    {
        return Err(LedgerError::validation(
            "commandId 只能包含 ASCII 字母、数字、横线、下划线、冒号和点",
        ));
    }
    Ok(normalized.to_string())
}

pub fn normalize_title(value: &str) -> Result<String, LedgerError> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return Err(LedgerError::validation("任务标题不能为空"));
    }
    if normalized.chars().count() > MAX_TASK_TITLE_LENGTH {
        return Err(LedgerError::validation(format!(
            "任务标题不能超过 {MAX_TASK_TITLE_LENGTH} 个字符"
        )));
    }
    Ok(normalized.to_string())
}

pub fn validate_fact_range(from_ms: i64, to_ms: i64) -> Result<(), LedgerError> {
    if from_ms < 0 || to_ms <= from_ms {
        return Err(LedgerError::validation(
            "周报事实时间范围必须满足 0 <= fromMs < toMs",
        ));
    }
    Ok(())
}

pub fn normalize_deadline_on(value: Option<&str>) -> Result<Option<String>, LedgerError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let normalized = value.trim();
    let bytes = normalized.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(index, byte)| index != 4 && index != 7 && !byte.is_ascii_digit())
    {
        return Err(LedgerError::validation(
            "deadlineOn 必须是 YYYY-MM-DD 格式的本地日历日",
        ));
    }

    let year = normalized[0..4]
        .parse::<u32>()
        .map_err(|_| LedgerError::validation("deadlineOn 年份无效"))?;
    let month = normalized[5..7]
        .parse::<u32>()
        .map_err(|_| LedgerError::validation("deadlineOn 月份无效"))?;
    let day = normalized[8..10]
        .parse::<u32>()
        .map_err(|_| LedgerError::validation("deadlineOn 日期无效"))?;
    if year == 0 || !(1..=12).contains(&month) {
        return Err(LedgerError::validation("deadlineOn 不是有效的日历日期"));
    }
    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        2 if leap_year => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if day == 0 || day > days_in_month {
        return Err(LedgerError::validation("deadlineOn 不是有效的日历日期"));
    }
    Ok(Some(normalized.to_string()))
}

pub fn capture_task_transition(
    title: String,
    task_id: String,
    context: MutationContext,
) -> LedgerMutation {
    let MutationContext {
        command_id,
        event_id,
        occurred_at_ms,
        ..
    } = context;
    let task = Task {
        id: task_id.clone(),
        title: title.clone(),
        status: TaskStatus::Pending,
        queue_position: None,
        defer_until_ms: None,
        deadline_on: None,
        block_reason: None,
        abandon_reason: None,
        completed_at_ms: None,
        active_completion_event_id: None,
        version: 1,
        created_at_ms: occurred_at_ms,
        updated_at_ms: occurred_at_ms,
    };
    let event = TaskEvent {
        sequence: None,
        id: event_id,
        command_id,
        task_id,
        title_snapshot: title,
        event_type: TaskEventType::Created,
        occurred_at_ms,
        reason: None,
        metadata: serde_json::json!({}),
        reverses_event_id: None,
    };
    LedgerMutation {
        task_write: TaskWrite::Insert {
            task,
            place_at_tail: true,
        },
        event,
        reward: None,
    }
}

pub fn update_task_title_transition(
    task: &Task,
    title: String,
    context: MutationContext,
) -> Result<LedgerMutation, LedgerError> {
    if task.status != TaskStatus::Pending
        || task.defer_until_ms.is_some()
        || task.queue_position.is_none()
    {
        return Err(LedgerError::invalid_state(format!(
            "任务 {} 当前不可修改标题",
            task.id
        )));
    }
    if task.title == title {
        return Err(LedgerError::invalid_state("任务标题没有变化"));
    }

    let MutationContext {
        command_id,
        event_id,
        occurred_at_ms,
        ..
    } = context;
    let before_title = task.title.clone();
    let mut task_after = task.clone();
    task_after.title = title.clone();
    task_after.version = task.version + 1;
    task_after.updated_at_ms = occurred_at_ms;

    let event = TaskEvent {
        sequence: None,
        id: event_id,
        command_id,
        task_id: task.id.clone(),
        title_snapshot: title.clone(),
        event_type: TaskEventType::TitleUpdated,
        occurred_at_ms,
        reason: None,
        metadata: serde_json::json!({
            "beforeTitle": before_title,
            "afterTitle": title,
        }),
        reverses_event_id: None,
    };

    Ok(LedgerMutation {
        task_write: TaskWrite::Update {
            expected_version: task.version,
            task: task_after,
            place_at_tail: false,
        },
        event,
        reward: None,
    })
}

pub fn update_task_deadline_transition(
    task: &Task,
    deadline_on: Option<String>,
    context: MutationContext,
) -> Result<LedgerMutation, LedgerError> {
    if task.status != TaskStatus::Pending
        || task.defer_until_ms.is_some()
        || task.queue_position.is_none()
    {
        return Err(LedgerError::invalid_state(format!(
            "任务 {} 当前不可修改截止日期",
            task.id
        )));
    }
    if task.deadline_on == deadline_on {
        return Err(LedgerError::invalid_state("任务截止日期没有变化"));
    }

    let MutationContext {
        command_id,
        event_id,
        occurred_at_ms,
        ..
    } = context;
    let before_deadline_on = task.deadline_on.clone();
    let mut task_after = task.clone();
    task_after.deadline_on = deadline_on.clone();
    task_after.version = task.version + 1;
    task_after.updated_at_ms = occurred_at_ms;

    let event = TaskEvent {
        sequence: None,
        id: event_id,
        command_id,
        task_id: task.id.clone(),
        title_snapshot: task.title.clone(),
        event_type: TaskEventType::DeadlineUpdated,
        occurred_at_ms,
        reason: None,
        metadata: serde_json::json!({
            "beforeDeadlineOn": before_deadline_on,
            "afterDeadlineOn": deadline_on,
        }),
        reverses_event_id: None,
    };

    Ok(LedgerMutation {
        task_write: TaskWrite::Update {
            expected_version: task.version,
            task: task_after,
            place_at_tail: false,
        },
        event,
        reward: None,
    })
}

pub fn complete_task_transition(
    task: &Task,
    context: MutationContext,
) -> Result<LedgerMutation, LedgerError> {
    let MutationContext {
        command_id,
        event_id,
        reward_transaction_id,
        occurred_at_ms,
    } = context;
    let reward_transaction_id =
        reward_transaction_id.ok_or_else(|| LedgerError::integrity("完成转换缺少奖励交易 ID"))?;
    if task.status != TaskStatus::Pending
        || task.defer_until_ms.is_some()
        || task.queue_position.is_none()
    {
        return Err(LedgerError::invalid_state(format!(
            "任务 {} 当前不可完成",
            task.id
        )));
    }
    let mut task_after = task.clone();
    task_after.status = TaskStatus::Completed;
    task_after.queue_position = None;
    task_after.completed_at_ms = Some(occurred_at_ms);
    task_after.active_completion_event_id = Some(event_id.clone());
    task_after.version = task.version + 1;
    task_after.updated_at_ms = occurred_at_ms;

    let event = TaskEvent {
        sequence: None,
        id: event_id.clone(),
        command_id,
        task_id: task.id.clone(),
        title_snapshot: task.title.clone(),
        event_type: TaskEventType::Completed,
        occurred_at_ms,
        reason: None,
        metadata: serde_json::json!({}),
        reverses_event_id: None,
    };
    let reward = RewardMutation {
        id: reward_transaction_id,
        task_event_id: event_id,
        reward_type: RewardType::TaskCompletion,
        amount: COMPLETION_REWARD,
        occurred_at_ms,
    };

    Ok(LedgerMutation {
        task_write: TaskWrite::Update {
            expected_version: task.version,
            task: task_after,
            place_at_tail: false,
        },
        event,
        reward: Some(reward),
    })
}

pub fn delete_task_transition(
    task: &Task,
    context: MutationContext,
) -> Result<LedgerMutation, LedgerError> {
    if task.status != TaskStatus::Pending
        || task.defer_until_ms.is_some()
        || task.queue_position.is_none()
    {
        return Err(LedgerError::invalid_state(format!(
            "任务 {} 当前不可删除",
            task.id
        )));
    }

    let MutationContext {
        command_id,
        event_id,
        occurred_at_ms,
        ..
    } = context;
    let reason = "用户删除".to_string();
    let mut task_after = task.clone();
    task_after.status = TaskStatus::Abandoned;
    task_after.queue_position = None;
    task_after.abandon_reason = Some(reason.clone());
    task_after.completed_at_ms = None;
    task_after.active_completion_event_id = None;
    task_after.version = task.version + 1;
    task_after.updated_at_ms = occurred_at_ms;

    let event = TaskEvent {
        sequence: None,
        id: event_id,
        command_id,
        task_id: task.id.clone(),
        title_snapshot: task.title.clone(),
        event_type: TaskEventType::Abandoned,
        occurred_at_ms,
        reason: Some(reason),
        metadata: serde_json::json!({ "action": "delete" }),
        reverses_event_id: None,
    };

    Ok(LedgerMutation {
        task_write: TaskWrite::Update {
            expected_version: task.version,
            task: task_after,
            place_at_tail: false,
        },
        event,
        reward: None,
    })
}

pub fn reorder_queue_transition(
    queue: &[Task],
    moved_task_id: String,
    expected_task_ids: Vec<String>,
    ordered_task_ids: Vec<String>,
    context: MutationContext,
) -> Result<LedgerMutation, LedgerError> {
    if queue.len() < 2 {
        return Err(LedgerError::invalid_state("至少需要两条待办才能调整顺序"));
    }
    let current_task_ids: Vec<&str> = queue.iter().map(|task| task.id.as_str()).collect();
    let expected_ids: Vec<&str> = expected_task_ids.iter().map(String::as_str).collect();
    if current_task_ids != expected_ids {
        return Err(LedgerError::concurrency_conflict(
            "待办顺序已经变化，请刷新后再调整",
        ));
    }
    if ordered_task_ids.len() != expected_task_ids.len() {
        return Err(LedgerError::validation(
            "调整后的任务数量必须与当前队列一致",
        ));
    }
    let expected_set: HashSet<&str> = expected_task_ids.iter().map(String::as_str).collect();
    let ordered_set: HashSet<&str> = ordered_task_ids.iter().map(String::as_str).collect();
    if expected_set.len() != expected_task_ids.len()
        || ordered_set.len() != ordered_task_ids.len()
        || expected_set != ordered_set
    {
        return Err(LedgerError::validation(
            "调整顺序必须提交无重复的完整待办 ID",
        ));
    }
    let old_index = expected_task_ids
        .iter()
        .position(|task_id| task_id == &moved_task_id)
        .ok_or_else(|| LedgerError::validation("movedTaskId 不在当前待办队列中"))?;
    let new_index = ordered_task_ids
        .iter()
        .position(|task_id| task_id == &moved_task_id)
        .ok_or_else(|| LedgerError::validation("movedTaskId 不在调整后队列中"))?;
    if expected_task_ids == ordered_task_ids || old_index == new_index {
        return Err(LedgerError::invalid_state("待办顺序没有变化"));
    }

    let moved_task = queue
        .iter()
        .find(|task| task.id == moved_task_id)
        .ok_or_else(|| LedgerError::integrity("移动任务未出现在已校验队列中"))?;
    let expected_queue = queue
        .iter()
        .map(|task| {
            if task.status != TaskStatus::Pending || task.defer_until_ms.is_some() {
                return Err(LedgerError::integrity(format!(
                    "任务 {} 不是立即可执行待办",
                    task.id
                )));
            }
            Ok(QueuedTaskVersion {
                task_id: task.id.clone(),
                expected_version: task.version,
                expected_position: task.queue_position.ok_or_else(|| {
                    LedgerError::integrity(format!("任务 {} 缺少队列位置", task.id))
                })?,
            })
        })
        .collect::<Result<Vec<_>, LedgerError>>()?;
    let MutationContext {
        command_id,
        event_id,
        occurred_at_ms,
        ..
    } = context;
    let event = TaskEvent {
        sequence: None,
        id: event_id,
        command_id,
        task_id: moved_task.id.clone(),
        title_snapshot: moved_task.title.clone(),
        event_type: TaskEventType::QueueReordered,
        occurred_at_ms,
        reason: None,
        metadata: serde_json::json!({
            "movedTaskId": moved_task_id.clone(),
            "beforeTaskIds": expected_task_ids.clone(),
            "afterTaskIds": ordered_task_ids.clone(),
        }),
        reverses_event_id: None,
    };

    Ok(LedgerMutation {
        task_write: TaskWrite::ReorderQueue {
            expected_queue,
            ordered_task_ids,
            occurred_at_ms,
        },
        event,
        reward: None,
    })
}

pub fn undo_completion_transition(
    task: &Task,
    completion_event: &TaskEvent,
    context: MutationContext,
    balance_before: i64,
) -> Result<LedgerMutation, LedgerError> {
    let MutationContext {
        command_id,
        event_id: undo_event_id,
        reward_transaction_id,
        occurred_at_ms,
    } = context;
    let reward_transaction_id = reward_transaction_id
        .ok_or_else(|| LedgerError::integrity("撤销完成转换缺少奖励交易 ID"))?;
    if task.status != TaskStatus::Completed
        || task.active_completion_event_id.as_deref() != Some(completion_event.id.as_str())
        || completion_event.event_type != TaskEventType::Completed
        || completion_event.task_id != task.id
    {
        return Err(LedgerError::invalid_state(
            "该完成事件已经撤销，或不再是任务的有效完成事件",
        ));
    }
    if balance_before < COMPLETION_REWARD {
        return Err(LedgerError::invalid_state(
            "金币余额不足，无法原子撤销这次完成",
        ));
    }

    let mut task_after = task.clone();
    task_after.status = TaskStatus::Pending;
    task_after.queue_position = None;
    task_after.completed_at_ms = None;
    task_after.active_completion_event_id = None;
    task_after.version = task.version + 1;
    task_after.updated_at_ms = occurred_at_ms;

    let event = TaskEvent {
        sequence: None,
        id: undo_event_id.clone(),
        command_id,
        task_id: task.id.clone(),
        title_snapshot: task.title.clone(),
        event_type: TaskEventType::CompletionUndone,
        occurred_at_ms,
        reason: None,
        metadata: serde_json::json!({}),
        reverses_event_id: Some(completion_event.id.clone()),
    };
    let reward = RewardMutation {
        id: reward_transaction_id,
        task_event_id: undo_event_id,
        reward_type: RewardType::CompletionUndo,
        amount: -COMPLETION_REWARD,
        occurred_at_ms,
    };

    Ok(LedgerMutation {
        task_write: TaskWrite::Update {
            expected_version: task.version,
            task: task_after,
            place_at_tail: true,
        },
        event,
        reward: Some(reward),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending_task() -> Task {
        Task {
            id: "task-1".to_string(),
            title: "写周报".to_string(),
            status: TaskStatus::Pending,
            queue_position: Some(1),
            defer_until_ms: None,
            deadline_on: None,
            block_reason: None,
            abandon_reason: None,
            completed_at_ms: None,
            active_completion_event_id: None,
            version: 1,
            created_at_ms: 10,
            updated_at_ms: 10,
        }
    }

    #[test]
    fn completion_and_undo_create_opposite_reward_mutations() {
        let completion = complete_task_transition(
            &pending_task(),
            MutationContext {
                command_id: "complete-1".to_string(),
                event_id: "event-complete".to_string(),
                reward_transaction_id: Some("reward-plus".to_string()),
                occurred_at_ms: 20,
            },
        )
        .expect("完成转换应成功");
        let (completed_task, completion_event) = match completion.task_write {
            TaskWrite::Update { task, .. } => (task, completion.event),
            TaskWrite::Insert { .. } => panic!("完成不应新建任务"),
            TaskWrite::ReorderQueue { .. } => panic!("完成不应重排队列"),
        };
        assert_eq!(completion.reward.expect("应有奖励").amount, 1);

        let undo = undo_completion_transition(
            &completed_task,
            &completion_event,
            MutationContext {
                command_id: "undo-1".to_string(),
                event_id: "event-undo".to_string(),
                reward_transaction_id: Some("reward-minus".to_string()),
                occurred_at_ms: 30,
            },
            1,
        )
        .expect("撤销转换应成功");
        assert_eq!(undo.reward.expect("应有扣回交易").amount, -1);
        assert_eq!(
            undo.event.reverses_event_id.as_deref(),
            Some("event-complete")
        );
    }

    #[test]
    fn command_id_rejects_unsafe_runtime_input() {
        assert!(normalize_command_id(" op:1.test ").is_ok());
        assert!(normalize_command_id("含中文").is_err());
        assert!(normalize_command_id("a/b").is_err());
    }

    #[test]
    fn title_update_preserves_queue_state_and_records_before_and_after_titles() {
        let mutation = update_task_title_transition(
            &pending_task(),
            "整理周报".to_string(),
            MutationContext {
                command_id: "update-title-1".to_string(),
                event_id: "event-title-updated".to_string(),
                reward_transaction_id: None,
                occurred_at_ms: 20,
            },
        )
        .expect("修改标题转换应成功");

        let updated = match mutation.task_write {
            TaskWrite::Update {
                expected_version,
                task,
                place_at_tail,
            } => {
                assert_eq!(expected_version, 1);
                assert!(!place_at_tail);
                task
            }
            TaskWrite::Insert { .. } => panic!("修改标题不应新建任务"),
            TaskWrite::ReorderQueue { .. } => panic!("修改标题不应重排队列"),
        };
        assert_eq!(updated.title, "整理周报");
        assert_eq!(updated.status, TaskStatus::Pending);
        assert_eq!(updated.queue_position, Some(1));
        assert_eq!(updated.version, 2);
        assert_eq!(updated.updated_at_ms, 20);
        assert_eq!(mutation.event.event_type, TaskEventType::TitleUpdated);
        assert_eq!(mutation.event.title_snapshot, "整理周报");
        assert_eq!(mutation.event.metadata["beforeTitle"], "写周报");
        assert_eq!(mutation.event.metadata["afterTitle"], "整理周报");
        assert!(mutation.reward.is_none());
    }

    #[test]
    fn title_update_rejects_unchanged_or_non_visible_tasks() {
        let context = || MutationContext {
            command_id: "update-title-rejected".to_string(),
            event_id: "event-title-rejected".to_string(),
            reward_transaction_id: None,
            occurred_at_ms: 20,
        };
        assert_eq!(
            update_task_title_transition(&pending_task(), "写周报".to_string(), context())
                .expect_err("相同标题应拒绝")
                .code(),
            "INVALID_TASK_STATE"
        );

        for (status, deferred, queue_position) in [
            (TaskStatus::Completed, false, None),
            (TaskStatus::Abandoned, false, None),
            (TaskStatus::Blocked, false, None),
            (TaskStatus::Pending, true, None),
        ] {
            let mut task = pending_task();
            task.status = status;
            task.defer_until_ms = deferred.then_some(30);
            task.queue_position = queue_position;
            assert_eq!(
                update_task_title_transition(&task, "新标题".to_string(), context())
                    .expect_err("不可见任务应拒绝修改")
                    .code(),
                "INVALID_TASK_STATE"
            );
        }
    }

    #[test]
    fn deadline_validation_accepts_calendar_days_and_rejects_invalid_dates() {
        assert_eq!(
            normalize_deadline_on(Some(" 2024-02-29 ")).expect("闰日应有效"),
            Some("2024-02-29".to_string())
        );
        assert_eq!(normalize_deadline_on(None).expect("无期限应有效"), None);
        for invalid in [
            "0000-01-01",
            "2023-02-29",
            "2024-04-31",
            "2024-13-01",
            "2024-00-10",
            "2024-01-00",
            "2024-1-01",
            "2024/01/01",
            "２０２４-01-01",
        ] {
            assert!(
                normalize_deadline_on(Some(invalid)).is_err(),
                "无效日期应拒绝：{invalid}"
            );
        }
    }

    #[test]
    fn deadline_update_is_a_queue_and_reward_noop_with_audited_nullable_values() {
        let mutation = update_task_deadline_transition(
            &pending_task(),
            Some("2026-07-01".to_string()),
            MutationContext {
                command_id: "update-deadline-1".to_string(),
                event_id: "event-deadline-1".to_string(),
                reward_transaction_id: None,
                occurred_at_ms: 20,
            },
        )
        .expect("过去的有效日历日也应允许设置");
        let updated = match mutation.task_write {
            TaskWrite::Update {
                expected_version,
                task,
                place_at_tail,
            } => {
                assert_eq!(expected_version, 1);
                assert!(!place_at_tail);
                task
            }
            TaskWrite::Insert { .. } => panic!("截止日期修改不应新建任务"),
            TaskWrite::ReorderQueue { .. } => panic!("截止日期修改不应重排队列"),
        };
        assert_eq!(updated.deadline_on.as_deref(), Some("2026-07-01"));
        assert_eq!(updated.status, TaskStatus::Pending);
        assert_eq!(updated.queue_position, Some(1));
        assert_eq!(updated.version, 2);
        assert_eq!(mutation.event.event_type, TaskEventType::DeadlineUpdated);
        assert_eq!(mutation.event.title_snapshot, "写周报");
        assert!(mutation.event.metadata["beforeDeadlineOn"].is_null());
        assert_eq!(mutation.event.metadata["afterDeadlineOn"], "2026-07-01");
        assert!(mutation.reward.is_none());

        let mut already_dated = pending_task();
        already_dated.deadline_on = Some("2026-07-01".to_string());
        let clear = update_task_deadline_transition(
            &already_dated,
            None,
            MutationContext {
                command_id: "clear-deadline-1".to_string(),
                event_id: "event-deadline-clear".to_string(),
                reward_transaction_id: None,
                occurred_at_ms: 30,
            },
        )
        .expect("应允许清除期限");
        assert_eq!(clear.event.metadata["beforeDeadlineOn"], "2026-07-01");
        assert!(clear.event.metadata["afterDeadlineOn"].is_null());
    }

    #[test]
    fn deadline_update_rejects_unchanged_or_non_visible_tasks() {
        let context = || MutationContext {
            command_id: "update-deadline-rejected".to_string(),
            event_id: "event-deadline-rejected".to_string(),
            reward_transaction_id: None,
            occurred_at_ms: 20,
        };
        assert_eq!(
            update_task_deadline_transition(&pending_task(), None, context())
                .expect_err("同为无期限应拒绝")
                .code(),
            "INVALID_TASK_STATE"
        );
        for (status, deferred, queue_position) in [
            (TaskStatus::Completed, false, None),
            (TaskStatus::Abandoned, false, None),
            (TaskStatus::Blocked, false, None),
            (TaskStatus::Pending, true, None),
        ] {
            let mut task = pending_task();
            task.status = status;
            task.defer_until_ms = deferred.then_some(30);
            task.queue_position = queue_position;
            assert_eq!(
                update_task_deadline_transition(&task, Some("2026-07-20".to_string()), context(),)
                    .expect_err("非即时待办应拒绝修改期限")
                    .code(),
                "INVALID_TASK_STATE"
            );
        }
    }

    #[test]
    fn delete_softly_abandons_queued_task_without_reward() {
        let mutation = delete_task_transition(
            &pending_task(),
            MutationContext {
                command_id: "delete-1".to_string(),
                event_id: "event-delete".to_string(),
                reward_transaction_id: None,
                occurred_at_ms: 20,
            },
        )
        .expect("删除转换应成功");

        let deleted = match mutation.task_write {
            TaskWrite::Update {
                expected_version,
                task,
                place_at_tail,
            } => {
                assert_eq!(expected_version, 1);
                assert!(!place_at_tail);
                task
            }
            TaskWrite::Insert { .. } => panic!("删除不应新建任务"),
            TaskWrite::ReorderQueue { .. } => panic!("删除不应重排队列"),
        };
        assert_eq!(deleted.status, TaskStatus::Abandoned);
        assert_eq!(deleted.queue_position, None);
        assert_eq!(deleted.abandon_reason.as_deref(), Some("用户删除"));
        assert_eq!(deleted.version, 2);
        assert_eq!(mutation.event.event_type, TaskEventType::Abandoned);
        assert_eq!(mutation.event.reason.as_deref(), Some("用户删除"));
        assert_eq!(mutation.event.metadata["action"], "delete");
        assert!(mutation.reward.is_none());

        let mut already_deleted = deleted;
        already_deleted.queue_position = None;
        let error = delete_task_transition(
            &already_deleted,
            MutationContext {
                command_id: "delete-2".to_string(),
                event_id: "event-delete-2".to_string(),
                reward_transaction_id: None,
                occurred_at_ms: 30,
            },
        )
        .expect_err("已经删除的任务不能再次删除");
        assert_eq!(error.code(), "INVALID_TASK_STATE");
    }

    #[test]
    fn queue_reorder_records_before_and_after_order() {
        let first = pending_task();
        let mut second = pending_task();
        second.id = "task-2".to_string();
        second.title = "回复邮件".to_string();
        second.queue_position = Some(2);
        let mutation = reorder_queue_transition(
            &[first, second],
            "task-2".to_string(),
            vec!["task-1".to_string(), "task-2".to_string()],
            vec!["task-2".to_string(), "task-1".to_string()],
            MutationContext {
                command_id: "reorder-1".to_string(),
                event_id: "event-reorder".to_string(),
                reward_transaction_id: None,
                occurred_at_ms: 20,
            },
        )
        .expect("重排转换应成功");
        assert_eq!(mutation.event.event_type, TaskEventType::QueueReordered);
        assert_eq!(
            mutation.event.metadata["afterTaskIds"],
            serde_json::json!(["task-2", "task-1"])
        );
    }
}
