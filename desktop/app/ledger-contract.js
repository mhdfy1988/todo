export const LedgerCommand = Object.freeze({
  CAPTURE: "capture_task",
  CREATE_SUBTASK: "create_subtask",
  COMPLETE: "complete_task",
  DELETE: "delete_task",
  REORDER_TASKS: "reorder_tasks",
  REORDER_SUBTASKS: "reorder_subtasks",
  UPDATE_DEADLINE: "update_task_deadline",
  UPDATE_TITLE: "update_task_title",
  UNDO: "undo_completion",
});

export const MAX_TASK_TITLE_LENGTH = 500;

export const SUPPORTED_LEDGER_COMMANDS = Object.freeze(Object.values(LedgerCommand));

/**
 * @typedef {Object} LedgerSessionGateway
 * @property {() => Promise<Object>} windowStatus
 * @property {() => Promise<"normal"|"smoke">} runtimeProfile
 * @property {() => Promise<Object>} ledgerSnapshot
 * @property {() => Promise<Object>} ledgerIntegrity
 * @property {(profile: "normal"|"smoke") => Promise<void>} reportFrontendReady
 * @property {(operation: import("./state.js").PendingOperation) => Promise<Object>} executeLedgerOperation
 *
 * @typedef {Object} WindowGateway
 * @property {(mode: string, requestFocus: boolean) => Promise<Object>} setWindowMode
 * @property {() => Promise<void>} hideToTray
 * @property {(listener: (status: Object) => void) => Promise<() => void>} subscribeWindowStatus
 */

const DEFINITIVE_COMMAND_REJECTION_CODES = new Set([
  "VALIDATION_ERROR",
  "INVALID_TASK_STATE",
  "TASK_NOT_FOUND",
  "TASK_EVENT_NOT_FOUND",
  "COMMAND_ID_CONFLICT",
  "CONCURRENT_MODIFICATION",
]);

export function isSupportedLedgerCommand(command) {
  return SUPPORTED_LEDGER_COMMANDS.includes(command);
}

export function assertTaskId(value, label = "任务") {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new TypeError(`${label} ID 无效`);
  }
}

export function normalizeTaskTitle(value) {
  if (typeof value !== "string") throw new TypeError("任务标题无效");
  const normalized = value.trim();
  if (normalized.length === 0) throw new TypeError("任务标题不能为空");
  if ([...normalized].length > MAX_TASK_TITLE_LENGTH) {
    throw new TypeError(`任务标题不能超过 ${MAX_TASK_TITLE_LENGTH} 个字符`);
  }
  return normalized;
}

export function assertUpdateTaskTitlePayload(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new TypeError("修改标题命令 payload 无效");
  }
  const keys = Object.keys(value).sort();
  if (keys.length !== 2 || keys[0] !== "taskId" || keys[1] !== "title") {
    throw new TypeError("修改标题命令 payload 字段无效");
  }
  assertTaskId(value.taskId, "待修改任务");
  if (value.taskId !== value.taskId.trim()) {
    throw new TypeError("待修改任务 ID 无效");
  }
  const normalized = normalizeTaskTitle(value.title);
  if (normalized !== value.title) {
    throw new TypeError("修改后的任务标题必须去除首尾空白");
  }
}

export function isValidUpdateTaskTitlePayload(value) {
  try {
    assertUpdateTaskTitlePayload(value);
    return true;
  } catch {
    return false;
  }
}

export function assertCreateSubtaskPayload(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new TypeError("新增子代办命令 payload 无效");
  }
  const keys = Object.keys(value).sort();
  if (keys.length !== 2 || keys[0] !== "parentTaskId" || keys[1] !== "title") {
    throw new TypeError("新增子代办命令 payload 字段无效");
  }
  assertTaskId(value.parentTaskId, "父代办");
  if (value.parentTaskId !== value.parentTaskId.trim()) {
    throw new TypeError("父代办 ID 无效");
  }
  const normalized = normalizeTaskTitle(value.title);
  if (normalized !== value.title) {
    throw new TypeError("子代办标题必须去除首尾空白");
  }
}

export function isValidCreateSubtaskPayload(value) {
  try {
    assertCreateSubtaskPayload(value);
    return true;
  } catch {
    return false;
  }
}

const DATE_ONLY_PATTERN = /^(\d{4})-(\d{2})-(\d{2})$/;

/**
 * 截止日期只接受真实存在的公历日期；null 表示清除期限。
 * @param {unknown} value
 */
export function assertDeadlineOn(value) {
  if (value === null) return;
  if (typeof value !== "string") {
    throw new TypeError("截止日期必须是 YYYY-MM-DD 或 null");
  }
  const match = DATE_ONLY_PATTERN.exec(value);
  if (!match) throw new TypeError("截止日期必须是 YYYY-MM-DD 或 null");
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  if (year === 0) throw new TypeError("截止日期不是有效日期");
  const date = new Date(0);
  date.setUTCHours(0, 0, 0, 0);
  date.setUTCFullYear(year, month - 1, day);
  if (
    date.getUTCFullYear() !== year
    || date.getUTCMonth() !== month - 1
    || date.getUTCDate() !== day
  ) {
    throw new TypeError("截止日期不是有效日期");
  }
}

export function assertUpdateTaskDeadlinePayload(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new TypeError("修改截止日期命令 payload 无效");
  }
  const keys = Object.keys(value).sort();
  if (keys.length !== 2 || keys[0] !== "deadlineOn" || keys[1] !== "taskId") {
    throw new TypeError("修改截止日期命令 payload 字段无效");
  }
  assertTaskId(value.taskId, "待修改任务");
  if (value.taskId !== value.taskId.trim()) {
    throw new TypeError("待修改任务 ID 无效");
  }
  assertDeadlineOn(value.deadlineOn);
}

export function isValidUpdateTaskDeadlinePayload(value) {
  try {
    assertUpdateTaskDeadlinePayload(value);
    return true;
  } catch {
    return false;
  }
}

export function assertReorderPayload(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new TypeError("重排命令 payload 无效");
  }
  const { movedTaskId, expectedTaskIds, orderedTaskIds } = value;
  assertTaskId(movedTaskId, "被移动任务");
  assertTaskIdList(expectedTaskIds, "重排前任务顺序");
  assertTaskIdList(orderedTaskIds, "重排后任务顺序");

  const expectedSet = new Set(expectedTaskIds);
  if (
    expectedSet.size !== orderedTaskIds.length
    || orderedTaskIds.some((taskId) => !expectedSet.has(taskId))
  ) {
    throw new TypeError("重排前后必须包含同一组任务");
  }

  const beforeIndex = expectedTaskIds.indexOf(movedTaskId);
  const afterIndex = orderedTaskIds.indexOf(movedTaskId);
  if (beforeIndex < 0 || afterIndex < 0) {
    throw new TypeError("被移动任务必须同时存在于重排前后顺序中");
  }
  if (beforeIndex === afterIndex) {
    throw new TypeError("被移动任务的位置没有变化");
  }
}

export function isValidReorderPayload(value) {
  try {
    assertReorderPayload(value);
    return true;
  } catch {
    return false;
  }
}

export function assertReorderSubtasksPayload(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new TypeError("子代办重排命令 payload 无效");
  }
  const keys = Object.keys(value).sort();
  const expectedKeys = ["expectedTaskIds", "movedTaskId", "orderedTaskIds", "parentTaskId"];
  if (keys.length !== expectedKeys.length || keys.some((key, index) => key !== expectedKeys[index])) {
    throw new TypeError("子代办重排命令 payload 字段无效");
  }
  assertTaskId(value.parentTaskId, "父代办");
  if (value.parentTaskId !== value.parentTaskId.trim()) {
    throw new TypeError("父代办 ID 无效");
  }
  assertReorderPayload({
    movedTaskId: value.movedTaskId,
    expectedTaskIds: value.expectedTaskIds,
    orderedTaskIds: value.orderedTaskIds,
  });
}

export function isValidReorderSubtasksPayload(value) {
  try {
    assertReorderSubtasksPayload(value);
    return true;
  } catch {
    return false;
  }
}

function assertTaskIdList(value, label) {
  if (!Array.isArray(value) || value.length < 2) {
    throw new TypeError(`${label}至少需要两个任务`);
  }
  if (value.some((taskId) => typeof taskId !== "string" || taskId.trim().length === 0)) {
    throw new TypeError(`${label}包含无效任务 ID`);
  }
  if (new Set(value).size !== value.length) {
    throw new TypeError(`${label}不能包含重复任务`);
  }
}

/**
 * 只有明确的领域拒绝才能判定为“命令没有写入”。
 * 存储、完整性、版本和未知错误都必须保留原 operationId，等待恢复确认。
 */
export function isDefinitiveCommandRejection(error) {
  return Boolean(
    error
    && typeof error === "object"
    && typeof error.code === "string"
    && DEFINITIVE_COMMAND_REJECTION_CODES.has(error.code),
  );
}

/**
 * IPC 返回值属于运行时数据；这里做最小结构校验，避免损坏快照进入视图。
 */
export function assertLedgerSnapshot(value) {
  const valid = value
    && typeof value === "object"
    && (value.currentTask === null || typeof value.currentTask === "object")
    && Array.isArray(value.queue)
    && Array.isArray(value.completed)
    && Array.isArray(value.subtasks)
    && Array.isArray(value.effectiveCompletions)
    && Array.isArray(value.events)
    && Array.isArray(value.rewards)
    && Number.isInteger(value.balance)
    && value.balance >= 0;
  if (!valid) throw new Error("本地账本返回了无效快照");
  return value;
}
