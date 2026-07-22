import test from "node:test";
import assert from "node:assert/strict";

import { TaskListController, moveTaskByOffset } from "../app/task-list-controller.js";

test("键盘上移与下移生成完整的新顺序", () => {
  const original = ["a", "b", "c"];
  assert.deepEqual(moveTaskByOffset(original, "b", -1), ["b", "a", "c"]);
  assert.deepEqual(moveTaskByOffset(original, "b", 1), ["a", "c", "b"]);
  assert.deepEqual(original, ["a", "b", "c"]);
});

test("首尾越界移动保持原顺序且不制造空位", () => {
  assert.deepEqual(moveTaskByOffset(["a", "b"], "a", -1), ["a", "b"]);
  assert.deepEqual(moveTaskByOffset(["a", "b"], "b", 1), ["a", "b"]);
});

test("未知任务不会污染当前顺序", () => {
  assert.deepEqual(moveTaskByOffset(["a", "b"], "missing", 1), ["a", "b"]);
});

test("搜索态阻止拖动开始且不提交重排", () => {
  const harness = createReorderHarness({ canReorder: () => false });
  const event = eventFor(harness.handles[0]);

  harness.emit("dragstart", event);

  assert.equal(event.prevented, true);
  assert.deepEqual(harness.reorderCalls, []);
  assert.equal(harness.rows[0].classList.contains("is-dragging"), false);
});

test("搜索态 Alt 加方向键不提交重排", () => {
  const harness = createReorderHarness({ canReorder: () => false });
  const event = eventFor(harness.handles[1], { key: "ArrowUp", altKey: true });

  harness.emit("keydown", event);

  assert.deepEqual(harness.reorderCalls, []);
});

test("拖动开始后才进入搜索时 drop 和 dragend 仍不提交", () => {
  let searchActive = false;
  const harness = createReorderHarness({ canReorder: () => !searchActive });
  const dragstart = eventFor(harness.handles[0]);

  harness.emit("dragstart", dragstart);
  assert.equal(dragstart.prevented, false);
  assert.equal(harness.rows[0].classList.contains("is-dragging"), true);

  searchActive = true;
  const drop = eventFor(harness.handles[1]);
  harness.emit("drop", drop);
  harness.emit("dragend", eventFor(harness.handles[0]));

  assert.equal(drop.prevented, true);
  assert.deepEqual(harness.reorderCalls, []);
  assert.equal(harness.rows[0].classList.contains("is-dragging"), false);
});

test("搜索中完成或删除最后一个结果后清理待恢复焦点并回到搜索框", () => {
  for (const rememberMethod of ["rememberCompletionFocus", "rememberRemovalFocus"]) {
    let fallbackCount = 0;
    const harness = createTitleEditHarness({
      isSearchActive: () => true,
      focusSearchFallback: () => {
        fallbackCount += 1;
        return true;
      },
    });

    harness.controller[rememberMethod]("task-a");
    harness.list.rows = [];
    harness.controller.restorePendingFocus();
    harness.controller.restorePendingFocus();

    assert.equal(fallbackCount, 1, `${rememberMethod} 应只回退搜索框一次`);
  }
});

test("结果消失前取消搜索也会清理失效的待恢复焦点", () => {
  let searchActive = true;
  let fallbackCount = 0;
  const harness = createTitleEditHarness({
    isSearchActive: () => searchActive,
    focusSearchFallback: () => {
      fallbackCount += 1;
      return true;
    },
  });

  harness.controller.rememberCompletionFocus("task-a");
  harness.list.rows = [];
  searchActive = false;
  harness.controller.restorePendingFocus();
  searchActive = true;
  harness.controller.restorePendingFocus();

  assert.equal(fallbackCount, 0);
});

test("搜索中改名后目标消失会清理待恢复焦点并回到搜索框", async () => {
  let fallbackCount = 0;
  let harness;
  harness = createTitleEditHarness({
    isSearchActive: () => true,
    focusSearchFallback: () => {
      fallbackCount += 1;
      return true;
    },
    onUpdateTitle: async () => {
      harness.list.rows = [];
    },
  });
  harness.emit("dblclick", eventFor(harness.trigger));
  harness.titleEditor.value = "不再匹配搜索的新标题";

  harness.emit("keydown", eventFor(harness.titleEditor, { key: "Enter" }));
  await flushPromises();
  harness.controller.restorePendingFocus();

  assert.equal(fallbackCount, 1);
});

test("双击标题原位进入编辑并选中原文", () => {
  const harness = createTitleEditHarness();

  harness.emit("dblclick", eventFor(harness.trigger));

  assert.equal(harness.titleEditor.className, "task-title-editor");
  assert.equal(harness.titleEditor.value, "原标题");
  assert.equal(harness.titleEditor.maxLength, 500);
  assert.equal(harness.titleEditor.focused, true);
  assert.equal(harness.titleEditor.selected, true);
  assert.equal(harness.deadlineEditor.value, "");
});

test("键盘 Enter 或 F2 可进入编辑，busy 时不可进入", () => {
  for (const key of ["Enter", "F2"]) {
    const harness = createTitleEditHarness();
    const event = eventFor(harness.trigger, { key });
    harness.emit("keydown", event);
    assert.equal(harness.titleEditor.className, "task-title-editor");
    assert.equal(event.prevented, true);
    assert.equal(event.stopped, true);
  }

  const busy = createTitleEditHarness();
  busy.list.setAttribute("aria-busy", "true");
  busy.trigger.disabled = true;
  busy.emit("dblclick", eventFor(busy.trigger));
  assert.equal(busy.control, busy.trigger);
});

test("Enter 保存规范化标题且后续 blur 不会重复提交", async () => {
  const calls = [];
  const harness = createTitleEditHarness({
    onUpdateTitle: async (...args) => calls.push(args),
  });
  harness.emit("dblclick", eventFor(harness.trigger));
  const editor = harness.titleEditor;
  editor.value = "  新标题  ";

  harness.emit("keydown", eventFor(editor, { key: "Enter" }));
  harness.emit("focusout", eventFor(editor));
  await flushPromises();

  assert.deepEqual(calls, [["task-a", "新标题"]]);
  assert.equal(harness.control, harness.trigger);
});

test("输入法组合期间的 Enter 不提交", () => {
  const calls = [];
  const harness = createTitleEditHarness({
    onUpdateTitle: (...args) => calls.push(args),
  });
  harness.emit("dblclick", eventFor(harness.trigger));
  const editor = harness.titleEditor;
  editor.value = "组合输入";

  harness.emit("keydown", eventFor(editor, { key: "Enter", isComposing: true }));

  assert.deepEqual(calls, []);
  assert.equal(harness.titleEditor, editor);
});

test("输入法组合期间的 Escape 不取消标题编辑", () => {
  for (const compositionState of [
    { isComposing: true },
    { keyCode: 229 },
  ]) {
    const harness = createTitleEditHarness();
    harness.emit("dblclick", eventFor(harness.trigger));
    const editor = harness.titleEditor;
    const event = eventFor(editor, { key: "Escape", ...compositionState });

    harness.emit("keydown", event);

    assert.equal(harness.titleEditor, editor);
    assert.equal(event.prevented, false);
    assert.equal(event.stopped, false);
  }
});

test("Escape 取消编辑并阻断窗口级收起快捷键", () => {
  const calls = [];
  const harness = createTitleEditHarness({
    onUpdateTitle: (...args) => calls.push(args),
  });
  harness.emit("dblclick", eventFor(harness.trigger));
  const editor = harness.titleEditor;
  editor.value = "不保存";
  const event = eventFor(editor, { key: "Escape" });

  harness.emit("keydown", event);

  assert.deepEqual(calls, []);
  assert.equal(harness.control, harness.trigger);
  assert.equal(harness.trigger.focused, true);
  assert.equal(event.prevented, true);
  assert.equal(event.stopped, true);
});

test("blur 保存有效变化，空白或未变化则取消", async () => {
  const calls = [];
  const changed = createTitleEditHarness({
    onUpdateTitle: async (...args) => calls.push(args),
  });
  changed.emit("dblclick", eventFor(changed.trigger));
  changed.titleEditor.value = "blur 新标题";
  changed.emit("focusout", eventFor(changed.titleEditor));
  await flushPromises();

  for (const value of ["   ", " 原标题 "]) {
    const unchanged = createTitleEditHarness({
      onUpdateTitle: async (...args) => calls.push(args),
    });
    unchanged.emit("dblclick", eventFor(unchanged.trigger));
    unchanged.titleEditor.value = value;
    unchanged.emit("focusout", eventFor(unchanged.titleEditor));
    assert.equal(unchanged.control, unchanged.trigger);
  }

  assert.deepEqual(calls, [["task-a", "blur 新标题"]]);
});

test("blur 保存后不从用户刚切换到的控件抢回焦点", async () => {
  const harness = createTitleEditHarness();
  harness.emit("dblclick", eventFor(harness.trigger));
  harness.titleEditor.value = "失焦保存";

  harness.emit("focusout", eventFor(harness.titleEditor));
  await flushPromises();
  harness.controller.restorePendingFocus();

  assert.equal(harness.trigger.focused, false);
});

test("空白标题按 Enter 留在编辑态且不跨写入边界", () => {
  const calls = [];
  const harness = createTitleEditHarness({
    onUpdateTitle: (...args) => calls.push(args),
  });
  harness.emit("dblclick", eventFor(harness.trigger));
  const editor = harness.titleEditor;
  editor.value = "   ";

  harness.emit("keydown", eventFor(editor, { key: "Enter" }));

  assert.deepEqual(calls, []);
  assert.equal(harness.titleEditor, editor);
  assert.equal(editor.getAttribute("aria-invalid"), "true");
});

test("标题提交后只在真实 READY 控件恢复时归还焦点", async () => {
  let resolveSubmission;
  const submission = new Promise((resolve) => { resolveSubmission = resolve; });
  const harness = createTitleEditHarness({
    onUpdateTitle: () => {
      harness.trigger.disabled = true;
      harness.list.setAttribute("aria-busy", "true");
      return submission;
    },
  });
  harness.emit("dblclick", eventFor(harness.trigger));
  harness.titleEditor.value = "新标题";
  harness.emit("keydown", eventFor(harness.titleEditor, { key: "Enter" }));

  resolveSubmission({});
  await flushPromises();
  assert.equal(harness.trigger.focused, false);

  harness.trigger.disabled = false;
  harness.list.setAttribute("aria-busy", "false");
  harness.controller.restorePendingFocus();
  assert.equal(harness.trigger.focused, true);
});

test("焦点在标题与截止日期之间切换时不提前提交", () => {
  const titleCalls = [];
  const deadlineCalls = [];
  const harness = createTitleEditHarness({
    onUpdateTitle: async (...args) => titleCalls.push(args),
    onUpdateDeadline: async (...args) => deadlineCalls.push(args),
  });
  harness.emit("dblclick", eventFor(harness.trigger));

  harness.emit("focusout", eventFor(harness.titleEditor, {
    relatedTarget: harness.deadlineEditor,
  }));

  assert.deepEqual(titleCalls, []);
  assert.deepEqual(deadlineCalls, []);
  assert.equal(harness.deadlineEditor.focused, false);
  assert.ok(harness.titleEditor);
});

test("编辑态选择截止日期只提交期限命令", async () => {
  const titleCalls = [];
  const deadlineCalls = [];
  const harness = createTitleEditHarness({
    onUpdateTitle: async (...args) => titleCalls.push(args),
    onUpdateDeadline: async (...args) => deadlineCalls.push(args),
  });
  harness.emit("dblclick", eventFor(harness.trigger));
  harness.deadlineEditor.value = "2026-07-24";

  harness.emit("change", eventFor(harness.deadlineEditor));
  await flushPromises();

  assert.deepEqual(titleCalls, []);
  assert.deepEqual(deadlineCalls, [["task-a", "2026-07-24"]]);
  assert.equal(harness.control, harness.trigger);
  assert.equal(harness.trigger.focused, true);
});

test("已有期限可直接进入日期编辑并清除", async () => {
  const deadlineCalls = [];
  const harness = createTitleEditHarness({
    deadlineOn: "2026-07-24",
    onUpdateDeadline: async (...args) => deadlineCalls.push(args),
  });

  harness.emit("click", eventFor(harness.deadlineButton));
  assert.equal(harness.deadlineEditor.value, "2026-07-24");
  assert.equal(harness.deadlineEditor.focused, true);
  assert.equal(harness.clearButton.hidden, false);

  harness.emit("click", eventFor(harness.clearButton));
  await flushPromises();
  assert.deepEqual(deadlineCalls, [["task-a", null]]);
  assert.equal(harness.trigger.focused, true);
});

test("胶囊期限入口可直达当前任务的日期编辑", () => {
  const harness = createTitleEditHarness({ deadlineOn: "2026-07-24" });

  assert.equal(harness.controller.beginDeadlineEdit("missing-task"), false);
  assert.equal(harness.controller.beginDeadlineEdit("task-a"), true);
  assert.equal(harness.deadlineEditor.value, "2026-07-24");
  assert.equal(harness.deadlineEditor.focused, true);
});

test("标题和截止日期同时变化时按显式命令顺序串行提交", async () => {
  const calls = [];
  const harness = createTitleEditHarness({
    onUpdateTitle: async (...args) => calls.push(["title", ...args]),
    onUpdateDeadline: async (...args) => calls.push(["deadline", ...args]),
  });
  harness.emit("dblclick", eventFor(harness.trigger));
  harness.titleEditor.value = "新标题";
  harness.deadlineEditor.value = "2026-07-24";

  harness.emit("keydown", eventFor(harness.titleEditor, { key: "Enter" }));
  await flushPromises();

  assert.deepEqual(calls, [
    ["title", "task-a", "新标题"],
    ["deadline", "task-a", "2026-07-24"],
  ]);
});

function createTitleEditHarness({
  deadlineOn = null,
  onUpdateDeadline = async () => ({}),
  onUpdateTitle = async () => ({}),
  isSearchActive = () => false,
  focusSearchFallback = () => false,
} = {}) {
  const document = new FakeDocument();
  const list = new FakeList(document);
  const status = new FakeElement("p", document);
  const row = new FakeRow("task-a", document);
  const trigger = new FakeElement("button", document);
  trigger.type = "button";
  trigger.className = "task-title";
  trigger.dataset.taskId = "task-a";
  trigger.dataset.deadlineOn = deadlineOn ?? "";
  trigger.textContent = "原标题";
  row.setControl(trigger);
  let deadlineButton = null;
  if (deadlineOn) {
    deadlineButton = new FakeElement("button", document);
    deadlineButton.className = "task-deadline";
    deadlineButton.dataset.taskId = "task-a";
    deadlineButton.dataset.deadlineOn = deadlineOn;
    row.setDeadline(deadlineButton);
  }
  list.rows.push(row);
  const errors = [];
  const controller = new TaskListController({
    list,
    status,
    onReorder: async () => ({}),
    onUpdateDeadline,
    onUpdateTitle,
    onError: (error) => errors.push(error),
    isSearchActive,
    focusSearchFallback,
  });
  return {
    controller,
    list,
    row,
    trigger,
    deadlineButton,
    errors,
    emit: (type, event) => list.emit(type, event),
    get control() { return row.control; },
    get titleEditor() { return row.control?.querySelector(".task-title-editor") ?? null; },
    get deadlineEditor() { return row.control?.querySelector(".task-deadline-editor") ?? null; },
    get clearButton() { return row.control?.querySelector(".task-deadline-clear") ?? null; },
  };
}

function createReorderHarness({ canReorder = () => true } = {}) {
  const document = new FakeDocument();
  const list = new FakeList(document);
  const status = new FakeElement("p", document);
  const rows = ["task-a", "task-b"].map((taskId) => new FakeRow(taskId, document));
  const handles = rows.map((row) => {
    const handle = new FakeElement("button", document);
    handle.className = "drag-handle";
    handle.dataset.taskId = row.dataset.taskId;
    handle.draggable = true;
    row.setControl(handle);
    return handle;
  });
  list.rows.push(...rows);
  const reorderCalls = [];
  const controller = new TaskListController({
    list,
    status,
    onReorder: async (...args) => reorderCalls.push(args),
    onUpdateDeadline: async () => ({}),
    onUpdateTitle: async () => ({}),
    onError: () => {},
    canReorder,
  });
  return {
    controller,
    handles,
    list,
    reorderCalls,
    rows,
    emit: (type, event) => list.emit(type, event),
  };
}

function eventFor(target, properties = {}) {
  return {
    target,
    key: "",
    keyCode: 0,
    isComposing: false,
    repeat: false,
    relatedTarget: null,
    prevented: false,
    stopped: false,
    preventDefault() { this.prevented = true; },
    stopPropagation() { this.stopped = true; },
    ...properties,
  };
}

async function flushPromises() {
  for (let index = 0; index < 8; index += 1) await Promise.resolve();
}

class FakeDocument {
  constructor() {
    this.activeElement = null;
  }

  createElement(tagName) {
    return new FakeElement(tagName, this);
  }
}

class FakeElement {
  constructor(tagName, ownerDocument) {
    this.tagName = tagName.toUpperCase();
    this.ownerDocument = ownerDocument;
    this.className = "";
    this.dataset = {};
    this.attributes = new Map();
    this.listeners = new Map();
    this.children = [];
    this.disabled = false;
    this.focused = false;
    this.hidden = false;
    this.selected = false;
    this.parent = null;
    this.textContent = "";
    this.value = "";
    this.classList = {
      add: (...tokens) => {
        const names = new Set(this.className.split(/\s+/).filter(Boolean));
        tokens.forEach((token) => names.add(token));
        this.className = [...names].join(" ");
      },
      remove: (...tokens) => {
        const removed = new Set(tokens);
        this.className = this.className
          .split(/\s+/)
          .filter((name) => name && !removed.has(name))
          .join(" ");
      },
      contains: (token) => this.className.split(/\s+/).includes(token),
    };
  }

  addEventListener(type, listener) {
    this.listeners.set(type, listener);
  }

  setAttribute(name, value) {
    this.attributes.set(name, String(value));
  }

  getAttribute(name) {
    return this.attributes.get(name) ?? null;
  }

  removeAttribute(name) {
    this.attributes.delete(name);
  }

  append(...children) {
    children.forEach((child) => {
      child.parent = this;
      this.children.push(child);
    });
  }

  closest(selector) {
    let candidate = this;
    while (candidate) {
      if (candidate.className.split(/\s+/).includes(selector.slice(1))) return candidate;
      candidate = candidate.parent;
    }
    return null;
  }

  querySelector(selector) {
    if (this.closest(selector) === this) return this;
    for (const child of this.children) {
      const found = child.querySelector(selector);
      if (found) return found;
    }
    return null;
  }

  contains(candidate) {
    return candidate === this || this.children.some((child) => child.contains(candidate));
  }

  replaceWith(replacement) {
    this.parent?.setControl(replacement);
  }

  focus() {
    this.ownerDocument.activeElement = this;
    this.focused = true;
  }

  select() {
    this.selected = true;
  }
}

class FakeRow extends FakeElement {
  constructor(taskId, ownerDocument) {
    super("li", ownerDocument);
    this.className = "task-row";
    this.dataset.taskId = taskId;
    this.control = null;
    this.deadline = null;
  }

  setControl(control) {
    if (this.control) this.control.parent = null;
    this.control = control;
    control.parent = this;
  }

  setDeadline(deadline) {
    this.deadline = deadline;
    deadline.parent = this;
  }

  querySelector(selector) {
    return this.control?.querySelector(selector)
      ?? this.deadline?.querySelector(selector)
      ?? null;
  }
}

class FakeList extends FakeElement {
  constructor(ownerDocument) {
    super("ol", ownerDocument);
    this.rows = [];
  }

  emit(type, event) {
    this.listeners.get(type)?.(event);
  }

  querySelectorAll(selector) {
    return selector === ".task-row" ? this.rows : [];
  }
}
