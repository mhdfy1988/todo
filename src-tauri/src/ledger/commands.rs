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

#[tauri::command(rename_all = "camelCase")]
pub fn create_subtask(
    operation_id: String,
    parent_task_id: String,
    title: String,
    state: State<'_, LedgerState>,
) -> Result<MutationReceipt, CommandError> {
    state
        .create_subtask(&operation_id, &parent_task_id, &title)
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

#[tauri::command(rename_all = "camelCase")]
pub fn reorder_subtasks(
    operation_id: String,
    parent_task_id: String,
    moved_task_id: String,
    expected_task_ids: Vec<String>,
    ordered_task_ids: Vec<String>,
    state: State<'_, LedgerState>,
) -> Result<MutationReceipt, CommandError> {
    state
        .reorder_subtasks(
            &operation_id,
            &parent_task_id,
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

#[cfg(test)]
mod tests {
    use super::{capture_task, create_subtask, ledger_snapshot, reorder_subtasks, LedgerState};
    use serde_json::{json, Value};
    use tauri::{
        ipc::{CallbackFn, InvokeBody},
        test::{
            get_ipc_response, mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY,
        },
        webview::InvokeRequest,
        WebviewWindow, WebviewWindowBuilder,
    };

    fn invoke(
        webview: &WebviewWindow<MockRuntime>,
        command: &str,
        body: Value,
    ) -> Result<Value, Value> {
        get_ipc_response(
            webview,
            InvokeRequest {
                cmd: command.into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: "http://tauri.localhost"
                    .parse()
                    .expect("测试 IPC URL 应有效"),
                body: InvokeBody::Json(body),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        )
        .map(|body| body.deserialize::<Value>().expect("IPC 成功响应应为 JSON"))
    }

    #[test]
    fn subtask_commands_accept_camel_case_and_return_idempotent_receipts() {
        let app = mock_builder()
            .manage(LedgerState::in_memory().expect("测试账本应能初始化"))
            .invoke_handler(tauri::generate_handler![
                capture_task,
                create_subtask,
                reorder_subtasks,
                ledger_snapshot
            ])
            .build(mock_context(noop_assets()))
            .expect("测试 Tauri 应用应能创建");
        let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("测试 WebView 应能创建");

        let parent = invoke(
            &webview,
            "capture_task",
            json!({ "operationId": "ipc-capture-parent", "title": "写周报" }),
        )
        .expect("顶层代办应能通过 IPC 创建");
        let parent_task_id = parent["taskId"].as_str().expect("创建回执应包含 taskId");

        let first_args = json!({
            "operationId": "ipc-create-subtask-a",
            "parentTaskId": parent_task_id,
            "title": "汇总本周完成"
        });
        let first = invoke(&webview, "create_subtask", first_args.clone())
            .expect("第一条子代办应能通过 camelCase IPC 创建");
        let first_task_id = first["taskId"]
            .as_str()
            .expect("子代办创建回执应包含 taskId")
            .to_string();
        assert_eq!(first["replayed"], false);

        let replayed =
            invoke(&webview, "create_subtask", first_args).expect("相同子代办命令应重放成功回执");
        assert_eq!(replayed["taskId"], first_task_id);
        assert_eq!(replayed["replayed"], true);

        let second = invoke(
            &webview,
            "create_subtask",
            json!({
                "operationId": "ipc-create-subtask-b",
                "parentTaskId": parent_task_id,
                "title": "整理本周问题"
            }),
        )
        .expect("第二条子代办应能通过 IPC 创建");
        let second_task_id = second["taskId"]
            .as_str()
            .expect("子代办创建回执应包含 taskId")
            .to_string();

        let reordered = invoke(
            &webview,
            "reorder_subtasks",
            json!({
                "operationId": "ipc-reorder-subtasks",
                "parentTaskId": parent_task_id,
                "movedTaskId": second_task_id,
                "expectedTaskIds": [first_task_id, second_task_id],
                "orderedTaskIds": [second_task_id, first_task_id]
            }),
        )
        .expect("同组子代办应能通过 camelCase IPC 重排");
        assert_eq!(reordered["commandId"], "ipc-reorder-subtasks");
        assert_eq!(reordered["replayed"], false);

        let snapshot =
            invoke(&webview, "ledger_snapshot", json!({})).expect("IPC 应能读取重排后的真实快照");
        let subtask_ids = snapshot["subtasks"]
            .as_array()
            .expect("快照应包含 subtasks 数组")
            .iter()
            .map(|task| {
                task["id"]
                    .as_str()
                    .expect("子代办快照应包含 id")
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(subtask_ids, vec![second_task_id, first_task_id]);
    }
}
