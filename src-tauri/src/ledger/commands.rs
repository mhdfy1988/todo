use super::{
    domain::{IntegrityReport, LedgerError, LedgerSnapshot, MutationReceipt, WeeklyFacts},
    LedgerState,
};
use serde::Serialize;
use tauri::State;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub code: String,
    pub message: String,
}

impl From<LedgerError> for CommandError {
    fn from(error: LedgerError) -> Self {
        Self {
            code: error.code().to_string(),
            message: error.message().to_string(),
        }
    }
}

#[tauri::command]
pub fn capture_task(
    operation_id: String,
    title: String,
    state: State<'_, LedgerState>,
) -> Result<MutationReceipt, CommandError> {
    state
        .capture_task(&operation_id, &title)
        .map_err(CommandError::from)
}

#[tauri::command]
pub fn complete_task(
    operation_id: String,
    task_id: String,
    state: State<'_, LedgerState>,
) -> Result<MutationReceipt, CommandError> {
    state
        .complete_task(&operation_id, &task_id)
        .map_err(CommandError::from)
}

#[tauri::command]
pub fn update_task_title(
    operation_id: String,
    task_id: String,
    title: String,
    state: State<'_, LedgerState>,
) -> Result<MutationReceipt, CommandError> {
    state
        .update_task_title(&operation_id, &task_id, &title)
        .map_err(CommandError::from)
}

#[tauri::command]
pub fn update_task_deadline(
    operation_id: String,
    task_id: String,
    deadline_on: Option<String>,
    state: State<'_, LedgerState>,
) -> Result<MutationReceipt, CommandError> {
    state
        .update_task_deadline(&operation_id, &task_id, deadline_on.as_deref())
        .map_err(CommandError::from)
}

#[tauri::command]
pub fn delete_task(
    operation_id: String,
    task_id: String,
    state: State<'_, LedgerState>,
) -> Result<MutationReceipt, CommandError> {
    state
        .delete_task(&operation_id, &task_id)
        .map_err(CommandError::from)
}

#[tauri::command]
pub fn reorder_tasks(
    operation_id: String,
    moved_task_id: String,
    expected_task_ids: Vec<String>,
    ordered_task_ids: Vec<String>,
    state: State<'_, LedgerState>,
) -> Result<MutationReceipt, CommandError> {
    state
        .reorder_tasks(
            &operation_id,
            &moved_task_id,
            &expected_task_ids,
            &ordered_task_ids,
        )
        .map_err(CommandError::from)
}

#[tauri::command]
pub fn undo_completion(
    operation_id: String,
    completion_event_id: String,
    state: State<'_, LedgerState>,
) -> Result<MutationReceipt, CommandError> {
    state
        .undo_completion(&operation_id, &completion_event_id)
        .map_err(CommandError::from)
}

#[tauri::command]
pub fn ledger_snapshot(state: State<'_, LedgerState>) -> Result<LedgerSnapshot, CommandError> {
    state.snapshot().map_err(CommandError::from)
}

#[tauri::command]
pub fn weekly_facts(
    from_ms: i64,
    to_ms: i64,
    state: State<'_, LedgerState>,
) -> Result<WeeklyFacts, CommandError> {
    state
        .weekly_facts(from_ms, to_ms)
        .map_err(CommandError::from)
}

#[tauri::command]
pub fn ledger_integrity(state: State<'_, LedgerState>) -> Result<IntegrityReport, CommandError> {
    state.verify_integrity().map_err(CommandError::from)
}
