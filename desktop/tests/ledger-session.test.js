import test from "node:test";
import assert from "node:assert/strict";

import { LedgerSession } from "../app/ledger-session.js";
import { LedgerPhase } from "../app/state.js";
import { MemoryOutboxStore } from "../app/infrastructure/outbox-store.js";

test("写命令调用前已经保存稳定 operationId", async () => {
  const outbox = new RecordingOutboxStore();
  let commandCalls = 0;
  const gateway = createGateway({
    executeLedgerOperation: async (operation) => {
      commandCalls += 1;
      assert.equal(outbox.load()?.operationId, "stable-operation-id");
      assert.equal(outbox.load()?.committed, false);
      assert.equal(operation.operationId, "stable-operation-id");
    },
  });
  const session = createSession({ gateway, outbox, operationId: "stable-operation-id" });

  await session.start();
  const operation = await session.captureTask("整理任务");

  assert.equal(commandCalls, 1);
  assert.equal(operation.operationId, "stable-operation-id");
  assert.deepEqual(outbox.saved.map((item) => item.committed), [false, true]);
  assert.equal(outbox.load(), null);
  assert.equal(session.state.phase, LedgerPhase.READY);
});

test("可以按任务 ID 完成任意待办并复用可靠写入链", async () => {
  const outbox = new RecordingOutboxStore();
  const calls = [];
  const gateway = createGateway({
    executeLedgerOperation: async (operation) => {
      calls.push(structuredClone(operation));
      assert.deepEqual(outbox.load(), operation);
      assert.equal(operation.committed, false);
    },
  });
  const session = createSession({ gateway, outbox, operationId: "complete-any-id" });
  await session.start();

  const operation = await session.completeTask("later-task-id");

  assert.deepEqual(calls, [{
    key: "complete:later-task-id",
    operationId: "complete-any-id",
    command: "complete_task",
    payload: { taskId: "later-task-id" },
    committed: false,
  }]);
  assert.equal(operation.committed, true);
  assert.equal(outbox.load(), null);
  assert.equal(session.state.phase, LedgerPhase.READY);
});

test("删除待办复用稳定 operationId 和操作箱链路", async () => {
  const outbox = new RecordingOutboxStore();
  const calls = [];
  const gateway = createGateway({
    executeLedgerOperation: async (operation) => calls.push(structuredClone(operation)),
  });
  const session = createSession({ gateway, outbox, operationId: "delete-id" });
  await session.start();

  const operation = await session.deleteTask("task-to-delete");

  assert.deepEqual(calls, [{
    key: "delete:task-to-delete",
    operationId: "delete-id",
    command: "delete_task",
    payload: { taskId: "task-to-delete" },
    committed: false,
  }]);
  assert.equal(operation.committed, true);
  assert.equal(outbox.load(), null);
  assert.equal(session.state.phase, LedgerPhase.READY);
});

test("修改标题复用稳定 operationId、规范化标题和操作箱链路", async () => {
  const outbox = new RecordingOutboxStore();
  const calls = [];
  const gateway = createGateway({
    executeLedgerOperation: async (operation) => {
      calls.push(structuredClone(operation));
      assert.deepEqual(outbox.load(), operation);
      assert.equal(operation.committed, false);
    },
  });
  const session = createSession({ gateway, outbox, operationId: "update-title-id" });
  await session.start();

  const operation = await session.updateTaskTitle("task-to-update", "  新标题  ");

  assert.deepEqual(calls, [{
    key: "update-title:task-to-update",
    operationId: "update-title-id",
    command: "update_task_title",
    payload: { taskId: "task-to-update", title: "新标题" },
    committed: false,
  }]);
  assert.equal(operation.committed, true);
  assert.deepEqual(outbox.saved.map((item) => item.committed), [false, true]);
  assert.equal(outbox.load(), null);
  assert.equal(session.state.phase, LedgerPhase.READY);
});

test("修改标题在写入操作箱前拒绝无效任务或标题", async () => {
  const outbox = new RecordingOutboxStore();
  let commandCalls = 0;
  const session = createSession({
    gateway: createGateway({
      executeLedgerOperation: async () => { commandCalls += 1; },
    }),
    outbox,
  });
  await session.start();

  assert.throws(() => session.updateTaskTitle("", "新标题"), /ID 无效/);
  assert.throws(() => session.updateTaskTitle("task-id", "   "), /标题不能为空/);
  assert.throws(
    () => session.updateTaskTitle("task-id", "字".repeat(501)),
    /不能超过 500 个字符/,
  );
  assert.equal(commandCalls, 0);
  assert.deepEqual(outbox.saved, []);
});

test("修改截止日期复用稳定 operationId 和操作箱链路", async () => {
  const outbox = new RecordingOutboxStore();
  const calls = [];
  const gateway = createGateway({
    executeLedgerOperation: async (operation) => {
      calls.push(structuredClone(operation));
      assert.deepEqual(outbox.load(), operation);
      assert.equal(operation.committed, false);
    },
  });
  const session = createSession({ gateway, outbox, operationId: "update-deadline-id" });
  await session.start();

  const operation = await session.updateTaskDeadline("task-to-update", "2026-07-18");

  assert.deepEqual(calls, [{
    key: "update-deadline:task-to-update",
    operationId: "update-deadline-id",
    command: "update_task_deadline",
    payload: { taskId: "task-to-update", deadlineOn: "2026-07-18" },
    committed: false,
  }]);
  assert.equal(operation.committed, true);
  assert.deepEqual(outbox.saved.map((item) => item.committed), [false, true]);
  assert.equal(outbox.load(), null);
  assert.equal(session.state.phase, LedgerPhase.READY);
});

test("修改截止日期允许 null 清除，并在写入操作箱前拒绝无效日期", async () => {
  const outbox = new RecordingOutboxStore();
  const calls = [];
  const session = createSession({
    gateway: createGateway({
      executeLedgerOperation: async (operation) => calls.push(structuredClone(operation)),
    }),
    outbox,
    operationId: "clear-deadline-id",
  });
  await session.start();

  await session.updateTaskDeadline("task-id", null);
  assert.equal(calls[0].payload.deadlineOn, null);

  for (const deadlineOn of ["2026-7-18", "2026-02-30", "", undefined]) {
    assert.throws(
      () => session.updateTaskDeadline("task-id", deadlineOn),
      /截止日期/,
    );
  }
  assert.equal(calls.length, 1);
});

test("修改标题响应未知时保留同一 operationId 和规范化 payload", async () => {
  const outbox = new MemoryOutboxStore();
  let commandCalls = 0;
  const session = createSession({
    gateway: createGateway({
      executeLedgerOperation: async () => {
        commandCalls += 1;
        throw new Error("修改标题响应中断");
      },
    }),
    outbox,
    operationId: "unknown-update-title-id",
  });
  await session.start();

  await assert.rejects(
    session.updateTaskTitle("task-id", "  恢复后的标题  "),
    /修改标题响应中断/,
  );

  assert.deepEqual(outbox.load(), {
    key: "update-title:task-id",
    operationId: "unknown-update-title-id",
    command: "update_task_title",
    payload: { taskId: "task-id", title: "恢复后的标题" },
    committed: false,
  });
  assert.equal(session.state.phase, LedgerPhase.RECOVERY);
  assert.equal(session.canMutate(), false);
  assert.equal(await session.completeTask("task-id"), null);
  assert.equal(commandCalls, 1);
});

test("重排保存完整前后顺序并在真实快照刷新后清理操作箱", async () => {
  const outbox = new RecordingOutboxStore();
  const expectedTaskIds = ["task-a", "task-b", "task-c"];
  const orderedTaskIds = ["task-b", "task-a", "task-c"];
  const calls = [];
  let snapshotCalls = 0;
  const gateway = createGateway({
    executeLedgerOperation: async (operation) => {
      calls.push(structuredClone(operation));
      assert.equal(outbox.load()?.operationId, "reorder-id");
      assert.equal(outbox.load()?.committed, false);
    },
    ledgerSnapshot: async () => {
      snapshotCalls += 1;
      return snapshot();
    },
  });
  const session = createSession({ gateway, outbox, operationId: "reorder-id" });
  await session.start();

  const operationPromise = session.reorderTasks("task-b", expectedTaskIds, orderedTaskIds);
  expectedTaskIds.reverse();
  orderedTaskIds.reverse();
  const operation = await operationPromise;

  assert.deepEqual(calls, [{
    key: "reorder:task-b",
    operationId: "reorder-id",
    command: "reorder_tasks",
    payload: {
      movedTaskId: "task-b",
      expectedTaskIds: ["task-a", "task-b", "task-c"],
      orderedTaskIds: ["task-b", "task-a", "task-c"],
    },
    committed: false,
  }]);
  assert.deepEqual(operation.payload, calls[0].payload);
  assert.equal(operation.committed, true);
  assert.equal(snapshotCalls, 2);
  assert.deepEqual(outbox.saved.map((item) => item.committed), [false, true]);
  assert.equal(outbox.load(), null);
});

test("无效重排在写入操作箱前显式拒绝", async () => {
  const outbox = new RecordingOutboxStore();
  let commandCalls = 0;
  const session = createSession({
    gateway: createGateway({
      executeLedgerOperation: async () => {
        commandCalls += 1;
      },
    }),
    outbox,
  });
  await session.start();

  assert.throws(
    () => session.reorderTasks("task-a", ["task-a", "task-b"], ["task-a", "task-b"]),
    /位置没有变化/,
  );
  assert.throws(
    () => session.reorderTasks("task-a", ["task-a", "task-a"], ["task-a", "task-b"]),
    /不能包含重复任务/,
  );
  assert.throws(
    () => session.reorderTasks("task-a", ["task-a", "task-b"], ["task-a", "task-c"]),
    /同一组任务/,
  );

  assert.equal(commandCalls, 0);
  assert.deepEqual(outbox.saved, []);
  assert.equal(session.state.pendingOperation, null);
  assert.equal(session.canMutate(), true);
});

test("IPC 响应未知时保留未提交操作并锁住后续写入", async () => {
  const outbox = new MemoryOutboxStore();
  let commandCalls = 0;
  const gateway = createGateway({
    executeLedgerOperation: async () => {
      commandCalls += 1;
      throw new Error("IPC 响应中断");
    },
  });
  const session = createSession({ gateway, outbox, operationId: "unknown-result-id" });
  await session.start();

  await assert.rejects(session.captureTask("保留现场"), /IPC 响应中断/);

  assert.equal(outbox.load()?.operationId, "unknown-result-id");
  assert.equal(outbox.load()?.committed, false);
  assert.equal(session.state.pendingOperation?.operationId, "unknown-result-id");
  assert.equal(session.state.phase, LedgerPhase.RECOVERY);
  assert.equal(session.canMutate(), false);
  assert.equal(await session.captureTask("不能插队"), null);
  assert.equal(commandCalls, 1);
});

test("重排响应未知时保留原 operationId 和完整顺序并锁住写入", async () => {
  const outbox = new MemoryOutboxStore();
  let commandCalls = 0;
  const session = createSession({
    gateway: createGateway({
      executeLedgerOperation: async () => {
        commandCalls += 1;
        throw new Error("重排响应中断");
      },
    }),
    outbox,
    operationId: "unknown-reorder-id",
  });
  await session.start();

  await assert.rejects(
    session.reorderTasks(
      "task-c",
      ["task-a", "task-b", "task-c"],
      ["task-c", "task-a", "task-b"],
    ),
    /重排响应中断/,
  );

  assert.deepEqual(outbox.load(), {
    key: "reorder:task-c",
    operationId: "unknown-reorder-id",
    command: "reorder_tasks",
    payload: {
      movedTaskId: "task-c",
      expectedTaskIds: ["task-a", "task-b", "task-c"],
      orderedTaskIds: ["task-c", "task-a", "task-b"],
    },
    committed: false,
  });
  assert.equal(session.state.phase, LedgerPhase.RECOVERY);
  assert.equal(session.canMutate(), false);
  assert.equal(await session.completeTask("task-a"), null);
  assert.equal(commandCalls, 1);
});

test("领域错误代表结果已知，会清理操作箱并恢复交互", async () => {
  const outbox = new MemoryOutboxStore();
  const commandError = { code: "INVALID_TASK_STATE", message: "任务状态已变化" };
  const gateway = createGateway({
    executeLedgerOperation: async () => {
      throw commandError;
    },
  });
  const session = createSession({ gateway, outbox });
  await session.start();

  await assert.rejects(session.captureTask("冲突任务"), (error) => error === commandError);

  assert.equal(outbox.load(), null);
  assert.equal(session.state.pendingOperation, null);
  assert.equal(session.state.phase, LedgerPhase.READY);
  assert.equal(session.canMutate(), true);
});

test("存储错误结果未知，保留同一个 operationId 并锁住写入", async () => {
  const outbox = new MemoryOutboxStore();
  const storageError = { code: "STORAGE_ERROR", message: "账本暂时不可写" };
  const gateway = createGateway({
    executeLedgerOperation: async () => {
      throw storageError;
    },
  });
  const session = createSession({ gateway, outbox, operationId: "storage-error-id" });
  await session.start();

  await assert.rejects(session.captureTask("保留存储失败现场"), (error) => error === storageError);

  assert.equal(outbox.load()?.operationId, "storage-error-id");
  assert.equal(session.state.pendingOperation?.operationId, "storage-error-id");
  assert.equal(session.state.phase, LedgerPhase.RECOVERY);
  assert.equal(session.canMutate(), false);
});

test("冷启动恢复遇到结构化存储错误时仍保留原操作", async () => {
  const pending = {
    key: "complete:task-id",
    operationId: "startup-storage-error-id",
    command: "complete_task",
    payload: { taskId: "task-id" },
    committed: false,
  };
  const outbox = new MemoryOutboxStore(pending);
  const storageError = { code: "STORAGE_ERROR", message: "无法读取命令回执" };
  const session = createSession({
    gateway: createGateway({
      executeLedgerOperation: async () => {
        throw storageError;
      },
    }),
    outbox,
  });

  await assert.rejects(session.start(), (error) => error === storageError);

  assert.equal(outbox.load()?.operationId, "startup-storage-error-id");
  assert.equal(session.state.pendingOperation?.operationId, "startup-storage-error-id");
  assert.equal(session.state.phase, LedgerPhase.RECOVERY);
  assert.equal(session.canMutate(), false);
});

test("操作箱清理失败时保留 committed 操作并锁住写入", async () => {
  const outbox = new FailingClearOutboxStore();
  const session = createSession({
    gateway: createGateway(),
    outbox,
    operationId: "clear-error-id",
  });
  await session.start();

  await assert.rejects(session.captureTask("等待可靠清理"), /操作箱清理失败/);

  assert.equal(outbox.load()?.operationId, "clear-error-id");
  assert.equal(outbox.load()?.committed, true);
  assert.equal(session.state.pendingOperation?.operationId, "clear-error-id");
  assert.equal(session.state.phase, LedgerPhase.RECOVERY);
  assert.equal(session.canMutate(), false);
});

test("committed 操作恢复时不重复调用写命令", async () => {
  const committed = {
    key: "capture:已经提交",
    operationId: "committed-id",
    command: "capture_task",
    payload: { title: "已经提交" },
    committed: true,
  };
  const outbox = new MemoryOutboxStore(committed);
  let commandCalls = 0;
  const gateway = createGateway({
    executeLedgerOperation: async () => {
      commandCalls += 1;
    },
  });
  const session = createSession({ gateway, outbox });

  const result = await session.start();

  assert.equal(commandCalls, 0);
  assert.equal(result.recovered, true);
  assert.equal(result.operation.operationId, "committed-id");
  assert.equal(outbox.load(), null);
  assert.equal(session.state.phase, LedgerPhase.READY);
});

test("命令已提交但快照失败时保留 committed 操作供恢复", async () => {
  const outbox = new MemoryOutboxStore();
  let snapshotCalls = 0;
  const gateway = createGateway({
    ledgerSnapshot: async () => {
      snapshotCalls += 1;
      if (snapshotCalls === 1) return snapshot({ balance: 0 });
      throw new Error("快照连接中断");
    },
  });
  const session = createSession({ gateway, outbox, operationId: "committed-before-refresh" });
  await session.start();

  await assert.rejects(session.captureTask("等待刷新"), /快照连接中断/);

  assert.equal(outbox.load()?.operationId, "committed-before-refresh");
  assert.equal(outbox.load()?.committed, true);
  assert.equal(session.state.phase, LedgerPhase.RECOVERY);
  assert.equal(session.canMutate(), false);
});

test("较早快照晚返回时不能覆盖较新的快照", async () => {
  const older = deferred();
  const newer = deferred();
  let snapshotCalls = 0;
  const gateway = createGateway({
    ledgerSnapshot: async () => {
      snapshotCalls += 1;
      if (snapshotCalls === 1) return snapshot({ balance: 0 });
      if (snapshotCalls === 2) return older.promise;
      return newer.promise;
    },
  });
  const session = createSession({ gateway, outbox: new MemoryOutboxStore() });
  await session.start();

  const olderRefresh = session.refreshLedger();
  const newerRefresh = session.refreshLedger();
  newer.resolve(snapshot({ balance: 2 }));
  await newerRefresh;
  older.resolve(snapshot({ balance: 1 }));
  await olderRefresh;

  assert.equal(session.state.snapshot.balance, 2);
});

test("READY 状态只在内部运行锁释放后发布", async () => {
  const session = createSession({
    gateway: createGateway(),
    outbox: new MemoryOutboxStore(),
  });
  const readyObservations = [];
  session.subscribe((state) => {
    if (state.phase === LedgerPhase.READY) {
      readyObservations.push(session.canMutate());
    }
  });

  await session.start();
  await session.runDiagnostics();
  await session.captureTask("检查就绪发布时序");

  assert.deepEqual(readyObservations, [true, true, true]);
});

test("操作箱结构损坏时启动显式失败且保持写入锁", async () => {
  let readyReports = 0;
  const session = createSession({
    gateway: createGateway({
      reportFrontendReady: async () => {
        readyReports += 1;
      },
    }),
    outbox: {
      load() {
        throw new Error("待确认操作结构损坏，需要人工确认后处理");
      },
      save() {},
      clear() {},
    },
  });

  await assert.rejects(session.start(), /待确认操作结构损坏/);

  assert.equal(readyReports, 0);
  assert.equal(session.state.phase, LedgerPhase.ERROR);
  assert.equal(session.canMutate(), false);
});

for (const profile of ["normal", "smoke"]) {
  test(`${profile} 启动在真实快照后上报 ledgerReady`, async () => {
    const reports = [];
    let session;
    const gateway = createGateway({
      runtimeProfile: async () => profile,
      reportFrontendReady: async (reportedProfile) => {
        assert.equal(session.state.phase, LedgerPhase.READY);
        assert.equal(session.state.snapshotReady, true);
        assert.equal(session.canMutate(), true);
        reports.push({ profile: reportedProfile, ledgerReady: true });
      },
    });
    session = createSession({ gateway, outbox: new MemoryOutboxStore() });

    await session.start();

    assert.deepEqual(reports, [{ profile, ledgerReady: true }]);
    assert.equal(session.state.snapshotReady, true);
  });
}

function createSession({ gateway, outbox, operationId = "operation-id" }) {
  return new LedgerSession({
    gateway,
    outboxStoreFactory: () => outbox,
    operationIdFactory: () => operationId,
  });
}

function createGateway(overrides = {}) {
  return {
    windowStatus: async () => ({
      mode: "capsule",
      focused: false,
      inWorkArea: true,
      alwaysOnTop: true,
    }),
    runtimeProfile: async () => "smoke",
    ledgerSnapshot: async () => snapshot(),
    ledgerIntegrity: async () => ({ failures: [] }),
    executeLedgerOperation: async () => undefined,
    reportFrontendReady: async () => undefined,
    ...overrides,
  };
}

function snapshot({ balance = 0 } = {}) {
  return {
    currentTask: null,
    queue: [],
    completed: [],
    events: [],
    rewards: [],
    balance,
  };
}

class RecordingOutboxStore extends MemoryOutboxStore {
  constructor() {
    super();
    this.saved = [];
  }

  save(operation) {
    this.saved.push(structuredClone(operation));
    super.save(operation);
  }
}

class FailingClearOutboxStore extends MemoryOutboxStore {
  clear() {
    throw new Error("操作箱清理失败");
  }
}

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}
