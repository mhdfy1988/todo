import test from "node:test";
import assert from "node:assert/strict";

import {
  createOutboxStore,
  LocalStorageOutboxStore,
  NORMAL_OUTBOX_KEY,
} from "../app/infrastructure/outbox-store.js";
import {
  TauriGateway,
  WINDOW_STATUS_CHANGED_EVENT,
} from "../app/infrastructure/tauri-gateway.js";

test("normal 与 smoke 操作箱严格隔离", () => {
  const normalOperation = pendingOperation("normal-id");
  const storage = new FakeStorage({
    [NORMAL_OUTBOX_KEY]: JSON.stringify(normalOperation),
  });

  const smokeStore = createOutboxStore("smoke", storage);
  assert.equal(smokeStore.load(), null);
  smokeStore.save(pendingOperation("smoke-id"));
  smokeStore.clear();

  assert.equal(storage.reads, 0);
  assert.equal(storage.writes, 0);
  assert.equal(storage.removals, 0);
  assert.deepEqual(new LocalStorageOutboxStore(storage).load(), normalOperation);
});

test("操作箱 v1 key 保持不变", () => {
  assert.equal(NORMAL_OUTBOX_KEY, "zuoban.ledger.pending-operation.v1");
});

test("损坏的命令 payload 保留原记录并显式阻断恢复", () => {
  const invalidOperation = {
    ...pendingOperation("invalid-payload"),
    command: "complete_task",
    payload: {},
  };
  const storage = new FakeStorage({
    [NORMAL_OUTBOX_KEY]: JSON.stringify(invalidOperation),
  });

  assert.throws(
    () => new LocalStorageOutboxStore(storage).load(),
    /待确认操作结构损坏，需要人工确认后处理/,
  );
  assert.equal(storage.removals, 0);
  assert.equal(storage.values.get(NORMAL_OUTBOX_KEY), JSON.stringify(invalidOperation));
});

test("操作箱接受完整且真实变位的重排命令", () => {
  const operation = reorderOperation({
    expectedTaskIds: ["task-a", "task-b", "task-c"],
    orderedTaskIds: ["task-b", "task-a", "task-c"],
    movedTaskId: "task-b",
  });
  const storage = new FakeStorage({
    [NORMAL_OUTBOX_KEY]: JSON.stringify(operation),
  });

  assert.deepEqual(new LocalStorageOutboxStore(storage).load(), operation);
});

test("操作箱接受有效的删除命令", () => {
  const operation = {
    ...pendingOperation("delete-operation-id"),
    key: "delete:task-id",
    command: "delete_task",
    payload: { taskId: "task-id" },
  };
  const storage = new FakeStorage({
    [NORMAL_OUTBOX_KEY]: JSON.stringify(operation),
  });

  assert.deepEqual(new LocalStorageOutboxStore(storage).load(), operation);
});

test("操作箱接受规范化的修改标题命令", () => {
  const operation = {
    ...pendingOperation("update-title-operation-id"),
    key: "update-title:task-id",
    command: "update_task_title",
    payload: { taskId: "task-id", title: "新标题" },
  };
  const storage = new FakeStorage({
    [NORMAL_OUTBOX_KEY]: JSON.stringify(operation),
  });

  assert.deepEqual(new LocalStorageOutboxStore(storage).load(), operation);
});

for (const deadlineOn of ["2026-07-18", null]) {
  test(`操作箱接受有效的修改截止日期命令：${deadlineOn ?? "清除"}`, () => {
    const operation = {
      ...pendingOperation("update-deadline-operation-id"),
      key: "update-deadline:task-id",
      command: "update_task_deadline",
      payload: { taskId: "task-id", deadlineOn },
    };
    const storage = new FakeStorage({
      [NORMAL_OUTBOX_KEY]: JSON.stringify(operation),
    });

    assert.deepEqual(new LocalStorageOutboxStore(storage).load(), operation);
  });
}

for (const [description, payload] of [
  ["缺少任务 ID", { deadlineOn: "2026-07-18" }],
  ["任务 ID 未规范化", { taskId: " task-id ", deadlineOn: null }],
  ["日期格式不严格", { taskId: "task-id", deadlineOn: "2026-7-18" }],
  ["日期不存在", { taskId: "task-id", deadlineOn: "2026-02-30" }],
  ["日期为空字符串", { taskId: "task-id", deadlineOn: "" }],
  ["含额外字段", { taskId: "task-id", deadlineOn: null, legacyDeadline: null }],
]) {
  test(`操作箱拒绝修改截止日期 payload：${description}`, () => {
    const operation = {
      ...pendingOperation("invalid-update-deadline"),
      key: "update-deadline:task-id",
      command: "update_task_deadline",
      payload,
    };
    const storage = new FakeStorage({
      [NORMAL_OUTBOX_KEY]: JSON.stringify(operation),
    });

    assert.throws(
      () => new LocalStorageOutboxStore(storage).load(),
      /待确认操作结构损坏，需要人工确认后处理/,
    );
    assert.equal(storage.removals, 0);
    assert.equal(storage.values.get(NORMAL_OUTBOX_KEY), JSON.stringify(operation));
  });
}

for (const [description, payload] of [
  ["缺少任务 ID", { title: "新标题" }],
  ["标题为空", { taskId: "task-id", title: "   " }],
  ["标题超过 500 字", { taskId: "task-id", title: "字".repeat(501) }],
  ["标题未规范化", { taskId: "task-id", title: " 新标题 " }],
  ["含额外字段", { taskId: "task-id", title: "新标题", legacyTitle: "旧标题" }],
]) {
  test(`操作箱拒绝修改标题 payload：${description}`, () => {
    const operation = {
      ...pendingOperation("invalid-update-title"),
      key: "update-title:task-id",
      command: "update_task_title",
      payload,
    };
    const storage = new FakeStorage({
      [NORMAL_OUTBOX_KEY]: JSON.stringify(operation),
    });

    assert.throws(
      () => new LocalStorageOutboxStore(storage).load(),
      /待确认操作结构损坏，需要人工确认后处理/,
    );
    assert.equal(storage.removals, 0);
    assert.equal(storage.values.get(NORMAL_OUTBOX_KEY), JSON.stringify(operation));
  });
}

for (const [description, payload] of [
  ["任务不足两个", {
    movedTaskId: "task-a",
    expectedTaskIds: ["task-a"],
    orderedTaskIds: ["task-a"],
  }],
  ["顺序包含重复任务", {
    movedTaskId: "task-a",
    expectedTaskIds: ["task-a", "task-b"],
    orderedTaskIds: ["task-a", "task-a"],
  }],
  ["前后不是同一集合", {
    movedTaskId: "task-a",
    expectedTaskIds: ["task-a", "task-b"],
    orderedTaskIds: ["task-a", "task-c"],
  }],
  ["被移动任务不在顺序中", {
    movedTaskId: "task-c",
    expectedTaskIds: ["task-a", "task-b"],
    orderedTaskIds: ["task-b", "task-a"],
  }],
  ["被移动任务没有变位", {
    movedTaskId: "task-a",
    expectedTaskIds: ["task-a", "task-b", "task-c"],
    orderedTaskIds: ["task-a", "task-c", "task-b"],
  }],
]) {
  test(`操作箱拒绝重排 payload：${description}`, () => {
    const operation = reorderOperation(payload);
    const storage = new FakeStorage({
      [NORMAL_OUTBOX_KEY]: JSON.stringify(operation),
    });

    assert.throws(
      () => new LocalStorageOutboxStore(storage).load(),
      /待确认操作结构损坏，需要人工确认后处理/,
    );
    assert.equal(storage.removals, 0);
    assert.equal(storage.values.get(NORMAL_OUTBOX_KEY), JSON.stringify(operation));
  });
}

test("正常操作箱清理失败必须显式抛出", () => {
  const storage = new FakeStorage();
  storage.removeError = new Error("浏览器拒绝删除");
  const outbox = new LocalStorageOutboxStore(storage);

  assert.throws(() => outbox.clear(), /无法清理待确认操作：浏览器拒绝删除/);
});

test("TauriGateway 保持命令名和 camelCase payload 映射", async () => {
  const calls = [];
  const gateway = new TauriGateway(async (command, payload) => {
    calls.push({ command, payload });
    return {};
  });

  await gateway.executeLedgerOperation({
    ...pendingOperation("capture-id"),
    command: "capture_task",
    payload: { title: "记录任务" },
  });
  await gateway.executeLedgerOperation({
    ...pendingOperation("complete-id"),
    command: "complete_task",
    payload: { taskId: "task-id" },
  });
  await gateway.executeLedgerOperation({
    ...pendingOperation("delete-id"),
    command: "delete_task",
    payload: { taskId: "delete-task-id" },
  });
  await gateway.executeLedgerOperation({
    ...pendingOperation("reorder-id"),
    command: "reorder_tasks",
    payload: {
      movedTaskId: "task-b",
      expectedTaskIds: ["task-a", "task-b", "task-c"],
      orderedTaskIds: ["task-b", "task-a", "task-c"],
    },
  });
  await gateway.executeLedgerOperation({
    ...pendingOperation("update-title-id"),
    command: "update_task_title",
    payload: { taskId: "task-id", title: "新标题" },
  });
  await gateway.executeLedgerOperation({
    ...pendingOperation("update-deadline-id"),
    command: "update_task_deadline",
    payload: { taskId: "task-id", deadlineOn: "2026-07-18" },
  });
  await gateway.executeLedgerOperation({
    ...pendingOperation("undo-id"),
    command: "undo_completion",
    payload: { completionEventId: "event-id" },
  });
  await gateway.setWindowMode("expanded", true);
  await gateway.weeklyFacts(1000, 2000);
  await gateway.reportFrontendReady("smoke");

  assert.deepEqual(calls, [
    {
      command: "capture_task",
      payload: { title: "记录任务", operationId: "capture-id" },
    },
    {
      command: "complete_task",
      payload: { taskId: "task-id", operationId: "complete-id" },
    },
    {
      command: "delete_task",
      payload: { taskId: "delete-task-id", operationId: "delete-id" },
    },
    {
      command: "reorder_tasks",
      payload: {
        movedTaskId: "task-b",
        expectedTaskIds: ["task-a", "task-b", "task-c"],
        orderedTaskIds: ["task-b", "task-a", "task-c"],
        operationId: "reorder-id",
      },
    },
    {
      command: "update_task_title",
      payload: {
        taskId: "task-id",
        title: "新标题",
        operationId: "update-title-id",
      },
    },
    {
      command: "update_task_deadline",
      payload: {
        taskId: "task-id",
        deadlineOn: "2026-07-18",
        operationId: "update-deadline-id",
      },
    },
    {
      command: "undo_completion",
      payload: { completionEventId: "event-id", operationId: "undo-id" },
    },
    {
      command: "set_window_mode",
      payload: { mode: "expanded", requestFocus: true },
    },
    {
      command: "weekly_facts",
      payload: { fromMs: 1000, toMs: 2000 },
    },
    {
      command: "report_frontend_ready",
      payload: { report: { profile: "smoke", ledgerReady: true } },
    },
  ]);
});

test("TauriGateway 将原生窗口状态事件转发给应用层并支持取消监听", async () => {
  let registeredEvent = null;
  let registeredHandler = null;
  let unlistened = false;
  const gateway = new TauriGateway(
    async () => ({}),
    async (eventName, handler) => {
      registeredEvent = eventName;
      registeredHandler = handler;
      return () => { unlistened = true; };
    },
  );
  const statuses = [];

  const unlisten = await gateway.subscribeWindowStatus((status) => statuses.push(status));
  registeredHandler({ payload: { mode: "expanded", visible: true } });
  unlisten();

  assert.equal(registeredEvent, WINDOW_STATUS_CHANGED_EVENT);
  assert.deepEqual(statuses, [{ mode: "expanded", visible: true }]);
  assert.equal(unlistened, true);
});

function pendingOperation(operationId) {
  return {
    key: `capture:${operationId}`,
    operationId,
    command: "capture_task",
    payload: { title: operationId },
    committed: false,
  };
}

function reorderOperation(payload) {
  return {
    key: `reorder:${payload.movedTaskId}`,
    operationId: "reorder-operation-id",
    command: "reorder_tasks",
    payload,
    committed: false,
  };
}

class FakeStorage {
  constructor(initial = {}) {
    this.values = new Map(Object.entries(initial));
    this.reads = 0;
    this.writes = 0;
    this.removals = 0;
  }

  getItem(key) {
    this.reads += 1;
    return this.values.get(key) ?? null;
  }

  setItem(key, value) {
    this.writes += 1;
    this.values.set(key, value);
  }

  removeItem(key) {
    this.removals += 1;
    if (this.removeError) throw this.removeError;
    this.values.delete(key);
  }
}
