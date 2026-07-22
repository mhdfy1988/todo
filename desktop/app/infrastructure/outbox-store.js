import {
  isSupportedLedgerCommand,
  isValidReorderPayload,
  isValidUpdateTaskDeadlinePayload,
  isValidUpdateTaskTitlePayload,
  LedgerCommand,
} from "../ledger-contract.js";

export const NORMAL_OUTBOX_KEY = "zuoban.ledger.pending-operation.v1";

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

export function isValidPendingOperation(value) {
  return Boolean(
    value
    && typeof value === "object"
    && typeof value.key === "string"
    && value.key.length > 0
    && typeof value.operationId === "string"
    && value.operationId.length > 0
    && isSupportedLedgerCommand(value.command)
    && isValidCommandPayload(value.command, value.payload)
    && typeof value.committed === "boolean",
  );
}

function isValidCommandPayload(command, payload) {
  if (!payload || typeof payload !== "object" || Array.isArray(payload)) return false;
  switch (command) {
    case LedgerCommand.CAPTURE:
      return typeof payload.title === "string" && payload.title.trim().length > 0;
    case LedgerCommand.COMPLETE:
    case LedgerCommand.DELETE:
      return typeof payload.taskId === "string" && payload.taskId.length > 0;
    case LedgerCommand.REORDER_TASKS:
      return isValidReorderPayload(payload);
    case LedgerCommand.UPDATE_DEADLINE:
      return isValidUpdateTaskDeadlinePayload(payload);
    case LedgerCommand.UPDATE_TITLE:
      return isValidUpdateTaskTitlePayload(payload);
    case LedgerCommand.UNDO:
      return typeof payload.completionEventId === "string"
        && payload.completionEventId.length > 0;
    default:
      return false;
  }
}

export class LocalStorageOutboxStore {
  constructor(storage, key = NORMAL_OUTBOX_KEY) {
    if (!storage) throw new Error("正常运行模式缺少 localStorage");
    this.storage = storage;
    this.key = key;
  }

  load() {
    try {
      const value = this.storage.getItem(this.key);
      if (!value) return null;
      const parsed = JSON.parse(value);
      if (isValidPendingOperation(parsed)) return parsed;
      throw new Error("待确认操作结构损坏，需要人工确认后处理");
    } catch (error) {
      throw new Error(`无法读取待确认操作：${errorMessage(error)}`);
    }
  }

  save(operation) {
    try {
      this.storage.setItem(this.key, JSON.stringify(operation));
    } catch (error) {
      throw new Error(`无法保存待确认操作：${errorMessage(error)}`);
    }
  }

  clear() {
    try {
      this.storage.removeItem(this.key);
    } catch (error) {
      throw new Error(`无法清理待确认操作：${errorMessage(error)}`);
    }
  }
}

/** 冒烟模式只在当前页面内保存，不读写正常运行凭据。 */
export class MemoryOutboxStore {
  constructor(initialOperation = null) {
    this.operation = initialOperation;
  }

  load() {
    return this.operation;
  }

  save(operation) {
    this.operation = operation;
  }

  clear() {
    this.operation = null;
  }
}

export function createOutboxStore(profile, storage) {
  if (profile === "normal") return new LocalStorageOutboxStore(storage);
  if (profile === "smoke") return new MemoryOutboxStore();
  throw new Error(`不支持的运行配置：${profile}`);
}
