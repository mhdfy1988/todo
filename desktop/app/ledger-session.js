import {
  createLedgerStore,
  isLedgerInteractive,
  LedgerPhase,
} from "./state.js";
import {
  assertReorderPayload,
  assertLedgerSnapshot,
  assertTaskId,
  assertUpdateTaskDeadlinePayload,
  assertUpdateTaskTitlePayload,
  isDefinitiveCommandRejection,
  LedgerCommand,
  normalizeTaskTitle,
} from "./ledger-contract.js";

/**
 * 账本应用服务：集中管理操作箱、幂等命令、恢复和快照时序。
 * 视图和窗口事件不得绕过这里直接调用写命令。
 */
export class LedgerSession {
  /**
   * @param {Object} dependencies
   * @param {import("./ledger-contract.js").LedgerSessionGateway} dependencies.gateway
   * @param {(profile: "normal"|"smoke") => Object} dependencies.outboxStoreFactory
   * @param {() => string} [dependencies.operationIdFactory]
   * @param {ReturnType<typeof createLedgerStore>} [dependencies.store]
   */
  constructor({
    gateway,
    outboxStoreFactory,
    operationIdFactory = createOperationId,
    store = createLedgerStore(),
  }) {
    if (!gateway) throw new TypeError("LedgerSession 缺少 TauriGateway");
    if (typeof outboxStoreFactory !== "function") {
      throw new TypeError("LedgerSession 缺少操作箱工厂");
    }
    this.gateway = gateway;
    this.outboxStoreFactory = outboxStoreFactory;
    this.operationIdFactory = operationIdFactory;
    this.store = store;
    this.outboxStore = null;
    this.running = false;
    this.refreshRequestSequence = 0;
    this.refreshAppliedSequence = 0;
  }

  get state() {
    return this.store.getState();
  }

  /** @param {(state: Object) => void} listener */
  subscribe(listener) {
    return this.store.subscribe(listener);
  }

  canMutate() {
    return !this.running && isLedgerInteractive(this.state);
  }

  /**
   * 冷启动：真实快照成功后才上报前端就绪。静态预览不会创建本服务。
   */
  async start() {
    if (!this.#begin(LedgerPhase.LOADING)) return null;
    let result;
    try {
      const status = await this.gateway.windowStatus();
      await this.#ensureRuntimeProfile();
      const synchronization = await this.#recoverAndRefresh();
      result = { status, ...synchronization };
    } catch (error) {
      this.#applyFailure(error, false);
      throw error;
    } finally {
      this.running = false;
    }
    this.store.update({ phase: LedgerPhase.READY, error: null });
    await this.gateway.reportFrontendReady(this.state.profile);
    return result;
  }

  /** 启动和“检查”共享同一条恢复链，避免两套恢复语义漂移。 */
  async runDiagnostics() {
    if (!this.#begin(LedgerPhase.BUSY)) return null;
    let result;
    try {
      const status = await this.gateway.windowStatus();
      await this.#ensureRuntimeProfile();
      const synchronization = await this.#recoverAndRefresh();
      const integrity = await this.gateway.ledgerIntegrity();
      result = { status, integrity, ...synchronization };
    } catch (error) {
      this.#applyFailure(error, false);
      throw error;
    } finally {
      this.running = false;
    }
    this.store.update({ phase: LedgerPhase.READY, error: null });
    return result;
  }

  /** @param {string} title */
  captureTask(title) {
    return this.#runMutation({
      key: `capture:${title}`,
      command: LedgerCommand.CAPTURE,
      payload: { title },
    });
  }

  /** @param {string} taskId */
  completeTask(taskId) {
    assertTaskId(taskId, "待完成任务");
    return this.#runMutation({
      key: `complete:${taskId}`,
      command: LedgerCommand.COMPLETE,
      payload: { taskId },
    });
  }

  /** @param {string} taskId */
  deleteTask(taskId) {
    assertTaskId(taskId, "待删除任务");
    return this.#runMutation({
      key: `delete:${taskId}`,
      command: LedgerCommand.DELETE,
      payload: { taskId },
    });
  }

  /** @param {string} taskId @param {string} title */
  updateTaskTitle(taskId, title) {
    const normalizedTitle = normalizeTaskTitle(title);
    const payload = { taskId, title: normalizedTitle };
    assertUpdateTaskTitlePayload(payload);
    return this.#runMutation({
      key: `update-title:${taskId}`,
      command: LedgerCommand.UPDATE_TITLE,
      payload,
    });
  }

  /** @param {string} taskId @param {string|null} deadlineOn */
  updateTaskDeadline(taskId, deadlineOn) {
    const payload = { taskId, deadlineOn };
    assertUpdateTaskDeadlinePayload(payload);
    return this.#runMutation({
      key: `update-deadline:${taskId}`,
      command: LedgerCommand.UPDATE_DEADLINE,
      payload,
    });
  }

  /**
   * @param {string} movedTaskId
   * @param {string[]} expectedTaskIds
   * @param {string[]} orderedTaskIds
   */
  reorderTasks(movedTaskId, expectedTaskIds, orderedTaskIds) {
    assertReorderPayload({ movedTaskId, expectedTaskIds, orderedTaskIds });
    return this.#runMutation({
      key: `reorder:${movedTaskId}`,
      command: LedgerCommand.REORDER_TASKS,
      payload: {
        movedTaskId,
        expectedTaskIds: [...expectedTaskIds],
        orderedTaskIds: [...orderedTaskIds],
      },
    });
  }

  /** @param {string} completionEventId */
  undoCompletion(completionEventId) {
    return this.#runMutation({
      key: `undo:${completionEventId}`,
      command: LedgerCommand.UNDO,
      payload: { completionEventId },
    });
  }

  /**
   * 公开给并发回归测试，也允许后续只读刷新入口复用。
   * 较早请求即使较晚返回，也不能覆盖较新的快照。
   */
  async refreshLedger() {
    const requestSequence = ++this.refreshRequestSequence;
    const snapshot = assertLedgerSnapshot(await this.gateway.ledgerSnapshot());
    if (requestSequence < this.refreshAppliedSequence) return snapshot;
    this.refreshAppliedSequence = requestSequence;
    this.store.update({ snapshot, snapshotReady: true });
    return snapshot;
  }

  #begin(phase) {
    if (this.running) return false;
    this.running = true;
    this.store.update({ phase, error: null });
    return true;
  }

  async #ensureRuntimeProfile() {
    if (this.state.profile && this.outboxStore) return;
    const profile = await this.gateway.runtimeProfile();
    if (profile !== "normal" && profile !== "smoke") {
      throw new Error(`不支持的运行配置：${profile}`);
    }
    const outboxStore = this.outboxStoreFactory(profile);
    const pendingOperation = outboxStore.load();
    this.outboxStore = outboxStore;
    this.store.update({
      profile,
      pendingOperation,
      phase: pendingOperation ? LedgerPhase.RECOVERY : this.state.phase,
    });
  }

  async #recoverAndRefresh() {
    const operation = this.state.pendingOperation;
    let recoveryError = null;
    let recovered = false;

    if (operation) {
      this.store.update({ phase: LedgerPhase.RECOVERY });
      try {
        await this.#invokePendingOperation(operation);
      } catch (error) {
        if (!isDefinitiveCommandRejection(error)) throw error;
        recoveryError = error;
      }
    }

    await this.refreshLedger();
    if (this.state.pendingOperation === operation && operation) {
      this.#clearPendingOperation();
      recovered = true;
    }
    return { recovered, recoveryError, operation: recovered ? operation : null };
  }

  async #runMutation(specification) {
    if (!this.canMutate()) return null;
    this.running = true;
    this.store.update({ phase: LedgerPhase.BUSY, error: null });
    let succeeded = false;
    const operation = {
      key: specification.key,
      operationId: this.operationIdFactory(),
      command: specification.command,
      payload: specification.payload,
      committed: false,
    };

    try {
      // 必须先可靠保存稳定 ID，再允许写命令跨过 IPC 边界。
      this.#savePendingOperation(operation);
      await this.#invokePendingOperation(operation);
      await this.refreshLedger();
      this.#clearPendingOperation();
      succeeded = true;
      return operation;
    } catch (error) {
      this.#applyFailure(error, true);
      throw error;
    } finally {
      this.running = false;
      if (succeeded) this.store.update({ phase: LedgerPhase.READY, error: null });
    }
  }

  #savePendingOperation(operation) {
    if (!this.outboxStore) throw new Error("运行配置尚未就绪");
    this.outboxStore.save(operation);
    this.store.update({ pendingOperation: operation });
  }

  #clearPendingOperation() {
    if (this.outboxStore) this.outboxStore.clear();
    this.store.update({ pendingOperation: null });
  }

  async #invokePendingOperation(operation) {
    if (operation.committed) return;
    try {
      await this.gateway.executeLedgerOperation(operation);
    } catch (error) {
      // 领域拒绝代表结果已知且零写入；未知通信结果必须保留同一个 ID。
      if (isDefinitiveCommandRejection(error)) this.#clearPendingOperation();
      throw error;
    }
    operation.committed = true;
    this.#savePendingOperation(operation);
  }

  #applyFailure(error, allowReadyWithoutPending) {
    const pendingOperation = this.state.pendingOperation;
    const phase = pendingOperation
      ? LedgerPhase.RECOVERY
      : allowReadyWithoutPending && this.state.snapshotReady
        ? LedgerPhase.READY
        : LedgerPhase.ERROR;
    this.store.update({ phase, error });
  }
}

function createOperationId() {
  return globalThis.crypto?.randomUUID
    ? globalThis.crypto.randomUUID()
    : `op-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}
