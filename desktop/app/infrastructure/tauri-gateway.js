import { LedgerCommand } from "../ledger-contract.js";

export const WINDOW_STATUS_CHANGED_EVENT = "window-status-changed";

/**
 * Tauri IPC 适配器。应用层只依赖这些中文语义方法，不直接拼命令名。
 */
export class TauriGateway {
  constructor(invoke, listen = null) {
    if (typeof invoke !== "function") throw new TypeError("Tauri invoke 不可用");
    this.invoke = invoke;
    this.listen = listen;
  }

  static fromWindow(hostWindow) {
    const invoke = hostWindow?.__TAURI__?.core?.invoke;
    const listen = hostWindow?.__TAURI__?.event?.listen;
    return typeof invoke === "function"
      ? new TauriGateway(
        invoke,
        typeof listen === "function" ? listen.bind(hostWindow.__TAURI__.event) : null,
      )
      : null;
  }

  call(command, payload = {}) {
    return this.invoke(command, payload);
  }

  runtimeProfile() {
    return this.call("runtime_profile");
  }

  checkForUpdate() {
    return this.call("check_for_update");
  }

  installUpdate(expectedVersion) {
    return this.call("install_update", { expectedVersion });
  }

  ledgerSnapshot() {
    return this.call("ledger_snapshot");
  }

  ledgerIntegrity() {
    return this.call("ledger_integrity");
  }

  weeklyFacts(fromMs, toMs) {
    return this.call("weekly_facts", { fromMs, toMs });
  }

  windowStatus() {
    return this.call("window_status");
  }

  setWindowMode(mode, requestFocus) {
    return this.call("set_window_mode", { mode, requestFocus });
  }

  hideToTray() {
    return this.call("hide_to_tray");
  }

  subscribeWindowStatus(listener) {
    if (typeof listener !== "function") throw new TypeError("窗口状态监听器无效");
    if (typeof this.listen !== "function") throw new Error("Tauri 事件监听不可用");
    return this.listen(WINDOW_STATUS_CHANGED_EVENT, (event) => listener(event.payload));
  }

  reportFrontendReady(profile) {
    return this.call("report_frontend_ready", {
      report: { profile, ledgerReady: true },
    });
  }

  /** @param {import("../state.js").PendingOperation} operation */
  executeLedgerOperation(operation) {
    const operationId = operation.operationId;
    switch (operation.command) {
      case LedgerCommand.CAPTURE:
        return this.call("capture_task", {
          title: operation.payload.title,
          operationId,
        });
      case LedgerCommand.COMPLETE:
        return this.call("complete_task", {
          taskId: operation.payload.taskId,
          operationId,
        });
      case LedgerCommand.DELETE:
        return this.call("delete_task", {
          taskId: operation.payload.taskId,
          operationId,
        });
      case LedgerCommand.REORDER_TASKS:
        return this.call("reorder_tasks", {
          movedTaskId: operation.payload.movedTaskId,
          expectedTaskIds: operation.payload.expectedTaskIds,
          orderedTaskIds: operation.payload.orderedTaskIds,
          operationId,
        });
      case LedgerCommand.UPDATE_DEADLINE:
        return this.call("update_task_deadline", {
          taskId: operation.payload.taskId,
          deadlineOn: operation.payload.deadlineOn,
          operationId,
        });
      case LedgerCommand.UPDATE_TITLE:
        return this.call("update_task_title", {
          taskId: operation.payload.taskId,
          title: operation.payload.title,
          operationId,
        });
      case LedgerCommand.UNDO:
        return this.call("undo_completion", {
          completionEventId: operation.payload.completionEventId,
          operationId,
        });
      default:
        throw new Error(`不支持的账本命令：${operation.command}`);
    }
  }
}
