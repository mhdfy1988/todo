import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";

import { SubtaskController, subtaskIds } from "../app/subtask-controller.js";

test("子代办 ID 只读取当前子列表的直接子行", () => {
  const rows = [
    { dataset: { taskId: "child-a" } },
    { dataset: { taskId: "child-b" } },
    { dataset: {} },
  ];
  const list = {
    querySelectorAll(selector) {
      assert.equal(selector, ":scope > .subtask-row");
      return rows;
    },
  };

  assert.deepEqual(subtaskIds(list), ["child-a", "child-b"]);
});

test("搜索期间拒绝进入单次添加态", () => {
  const controller = new SubtaskController({
    list: fakeList(),
    status: { textContent: "" },
    onCreate: async () => null,
    onUpdateTitle: async () => null,
    onReorder: async () => null,
    onError() {},
    canMutate: () => true,
    isSearchActive: () => true,
  });

  assert.equal(controller.startAdding("parent-id"), false);
});

test("任务组删除确认只有显式危险操作才返回 true", async () => {
  const handlers = new Map();
  const dialog = {
    returnValue: "",
    showModalCalls: 0,
    showModal() { this.showModalCalls += 1; },
    addEventListener(type, handler) { handlers.set(type, handler); },
  };
  const title = { textContent: "" };
  const controller = new SubtaskController({
    list: fakeList(),
    status: { textContent: "" },
    deleteDialog: dialog,
    deleteDialogTitle: title,
    onCreate: async () => null,
    onUpdateTitle: async () => null,
    onReorder: async () => null,
    onError() {},
  });

  const confirmation = controller.confirmGroupDeletion("写周报");
  assert.equal(dialog.showModalCalls, 1);
  assert.equal(title.textContent, "删除“写周报”？");
  dialog.returnValue = "confirm";
  handlers.get("close")();
  assert.equal(await confirmation, true);
});

test("新增输入空白失焦时退出添加态且不创建", async () => {
  const harness = createCaptureHarness();
  assert.equal(harness.controller.startAdding("parent-id"), true);
  const input = harness.captureInput;
  input.value = "   ";

  harness.emit("focusout", eventFor(input));
  await flushPromises();

  assert.deepEqual(harness.createCalls, []);
  assert.deepEqual(harness.errors, []);
  assert.equal(harness.captureInput, null);
  assert.equal(harness.addRow.hidden, false);
});

test("新增输入非空失焦时只创建一次并恢复添加入口", async () => {
  const harness = createCaptureHarness();
  harness.controller.startAdding("parent-id");
  const input = harness.captureInput;
  input.value = "  整理周报材料  ";

  harness.emit("focusout", eventFor(input));
  await flushPromises();

  assert.deepEqual(harness.createCalls, [["parent-id", "整理周报材料"]]);
  assert.deepEqual(harness.errors, []);
  assert.equal(harness.captureInput, null);
  assert.equal(harness.addRow.hidden, false);
  assert.equal(harness.status.textContent, "已添加子代办");
});

test("新增输入按回车并伴随失焦时不会重复创建", async () => {
  let resolveCreate;
  const createResult = new Promise((resolve) => { resolveCreate = resolve; });
  const harness = createCaptureHarness({
    onCreate: async (...args) => {
      harness.createCalls.push(args);
      return createResult;
    },
  });
  harness.controller.startAdding("parent-id");
  const input = harness.captureInput;
  input.value = "验证自动更新";

  harness.emit("keydown", eventFor(input, { key: "Enter" }));
  harness.emit("focusout", eventFor(input));
  assert.deepEqual(harness.createCalls, [["parent-id", "验证自动更新"]]);

  resolveCreate({});
  await flushPromises();

  assert.equal(harness.createCalls.length, 1);
  assert.equal(harness.captureInput, null);
  assert.equal(harness.addRow.hidden, false);
});

test("新增输入保存失败时保留草稿与添加态", async () => {
  const expectedError = new Error("写入失败");
  const harness = createCaptureHarness({
    onCreate: async () => { throw expectedError; },
  });
  harness.controller.startAdding("parent-id");
  const input = harness.captureInput;
  input.value = "保留这条草稿";

  harness.emit("focusout", eventFor(input));
  await flushPromises();

  assert.equal(harness.captureInput?.value, "保留这条草稿");
  assert.equal(harness.captureInput?.getAttribute("aria-invalid"), "true");
  assert.equal(harness.captureInput?.focused, true);
  assert.equal(harness.addRow.hidden, true);
  assert.deepEqual(harness.errors, [expectedError]);
});

test("子代办标题失焦时保存有效修改", async () => {
  const harness = createCaptureHarness();
  harness.emit("dblclick", eventFor(harness.title));
  const editor = harness.subtaskRow.querySelector(".subtask-title-editor");
  editor.value = "修改后的子代办";

  harness.emit("focusout", eventFor(editor));
  await flushPromises();

  assert.deepEqual(harness.updateCalls, [["child-id", "修改后的子代办"]]);
  assert.equal(harness.subtaskRow.querySelector(".subtask-title-editor"), null);
  assert.equal(harness.subtaskRow.querySelector(".subtask-title"), harness.title);
});

test("正式前端保持一级子代办、独立排序域、后端完成决策和单滚动区契约", async () => {
  const [html, styles, views, app, controller] = await Promise.all([
    readFile(new URL("../index.html", import.meta.url), "utf8"),
    readFile(new URL("../styles.css", import.meta.url), "utf8"),
    readFile(new URL("../app/views.js", import.meta.url), "utf8"),
    readFile(new URL("../app.js", import.meta.url), "utf8"),
    readFile(new URL("../app/subtask-controller.js", import.meta.url), "utf8"),
  ]);

  assert.match(app, /new SubtaskController\(\{/);
  assert.match(app, /session\.createSubtask\(parentTaskId, title\)/);
  assert.match(app, /session\.reorderSubtasks\(parentTaskId, movedTaskId, expectedTaskIds, orderedTaskIds\)/);
  assert.doesNotMatch(app, /taskGroupFor|incompleteCount|revealFirstPending\(taskId/);
  assert.match(app, /if \(!isSubtask\) \{\s*taskListController\.rememberCompletionFocus\(taskId\);[\s\S]*?session\.completeTask\(taskId\)/);
  assert.match(views, /className = `subtask-row\$\{completed \? " is-completed" : ""\}`/);
  assert.match(views, /className = "subtask-handle"/);
  assert.match(views, /subtaskList\.setAttribute\("aria-label", `\$\{task\.title\}的子代办`\)/);
  assert.match(views, /title\.disabled = completed/);
  assert.doesNotMatch(controller, /type = "date"|deadlineOn/);
  assert.match(controller, /this\.isSearchActive\(\)/);
  assert.match(controller, /this\.addDraft\.trim\(\)[\s\S]*#submitCapture\(event\.target, \{ restoreFocus: false \}\)/);
  assert.match(controller, /#finishTitleEdit\(\{ keepInvalidEditor: false, restoreFocus: false \}\)/);
  assert.match(controller, /this\.#stopAdding\(restoreFocus\)/);
  assert.match(controller, /this\.captureSubmitting/);
  assert.doesNotMatch(controller, /nextInput\.value = ""|nextInput\.focus\(\)/);
  assert.match(styles, /\.subtask-list\s*\{[^}]*grid-column:\s*1 \/ -1;[^}]*list-style:\s*none;/s);
  assert.doesNotMatch(styles, /\.subtask-list\s*\{[^}]*overflow-y\s*:/s);
  assert.match(styles, /\.subtask-add-trigger\s*\{[^}]*opacity:\s*0;/s);
  assert.match(styles, /\.task-row:hover \.subtask-add-trigger,[\s\S]*opacity:\s*1;/);
  assert.match(html, /id="deleteGroupDialog"/);
  assert.match(html, /value="confirm"[^>]*class="danger-button"/);
  assert.doesNotMatch(html, /子代办期限|子代办提醒/);
});

function fakeList() {
  return {
    ownerDocument: {},
    addEventListener() {},
    querySelectorAll() { return []; },
  };
}

function createCaptureHarness({
  onCreate = null,
  onUpdateTitle = null,
} = {}) {
  const document = new FakeDocument();
  const list = new FakeList(document);
  const status = new FakeElement("p", document);
  const parentRow = new FakeElement("li", document);
  parentRow.className = "task-row";
  parentRow.dataset.taskId = "parent-id";
  const subtaskList = new FakeElement("ol", document);
  subtaskList.className = "subtask-list";
  const subtaskRow = new FakeElement("li", document);
  subtaskRow.className = "subtask-row";
  subtaskRow.dataset.taskId = "child-id";
  subtaskRow.dataset.parentTaskId = "parent-id";
  const title = new FakeElement("button", document);
  title.className = "subtask-title";
  title.dataset.title = "原标题";
  title.textContent = "原标题";
  subtaskRow.append(title);
  const addRow = new FakeElement("li", document);
  addRow.className = "subtask-add-row";
  const addTrigger = new FakeElement("button", document);
  addTrigger.className = "subtask-add-trigger";
  addTrigger.dataset.parentTaskId = "parent-id";
  addRow.append(addTrigger);
  subtaskList.append(subtaskRow, addRow);
  parentRow.append(subtaskList);
  list.append(parentRow);
  const createCalls = [];
  const updateCalls = [];
  const errors = [];
  const controller = new SubtaskController({
    list,
    status,
    onCreate: onCreate ?? (async (...args) => {
      createCalls.push(args);
      return {};
    }),
    onUpdateTitle: onUpdateTitle ?? (async (...args) => {
      updateCalls.push(args);
      return {};
    }),
    onReorder: async () => ({}),
    onError: (error) => errors.push(error),
    canMutate: () => true,
    isSearchActive: () => false,
  });
  const harness = {
    controller,
    list,
    status,
    parentRow,
    subtaskList,
    subtaskRow,
    title,
    addRow,
    createCalls,
    updateCalls,
    errors,
    emit: (type, event) => list.emit(type, event),
    get captureInput() {
      return parentRow.querySelector(".subtask-capture-input");
    },
  };
  return harness;
}

function eventFor(target, properties = {}) {
  return {
    target,
    key: "",
    keyCode: 0,
    isComposing: false,
    repeat: false,
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
    this.parentElement = null;
    this.disabled = false;
    this.focused = false;
    this.hidden = false;
    this.isConnected = true;
    this.textContent = "";
    this.value = "";
    this.classList = {
      add: (...tokens) => this.#updateClasses(tokens, []),
      remove: (...tokens) => this.#updateClasses([], tokens),
      contains: (token) => this.#classNames().includes(token),
      toggle: (token, force) => {
        const shouldAdd = force ?? !this.#classNames().includes(token);
        this.#updateClasses(shouldAdd ? [token] : [], shouldAdd ? [] : [token]);
        return shouldAdd;
      },
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
      child.parentElement = this;
      child.isConnected = true;
      this.children.push(child);
    });
  }

  remove() {
    if (this.parentElement) {
      this.parentElement.children = this.parentElement.children.filter((child) => child !== this);
    }
    this.parentElement = null;
    this.isConnected = false;
  }

  replaceWith(replacement) {
    if (!this.parentElement) return;
    const index = this.parentElement.children.indexOf(this);
    if (index < 0) return;
    this.parentElement.children[index] = replacement;
    replacement.parentElement = this.parentElement;
    replacement.isConnected = true;
    this.parentElement = null;
    this.isConnected = false;
  }

  closest(selector) {
    const selectors = selector.split(",").map((part) => part.trim());
    let candidate = this;
    while (candidate) {
      if (selectors.some((part) => candidate.#matchesSimple(part))) return candidate;
      candidate = candidate.parentElement;
    }
    return null;
  }

  querySelector(selector) {
    const selectors = selector.split(",").map((part) => part.trim());
    if (selector.startsWith(":scope > ")) {
      const directSelector = selector.slice(":scope > ".length);
      if (directSelector.includes(" ")) return null;
      return this.children.find((child) => child.#matchesSimple(directSelector)) ?? null;
    }
    for (const child of this.children) {
      if (selectors.some((part) => child.#matchesSimple(part))) return child;
      const found = child.querySelector(selector);
      if (found) return found;
    }
    return null;
  }

  querySelectorAll(selector) {
    if (selector.startsWith(":scope > ")) {
      const directSelector = selector.slice(":scope > ".length);
      if (directSelector.includes(">") || directSelector.includes(" ")) return [];
      return this.children.filter((child) => child.#matchesSimple(directSelector));
    }
    const matches = [];
    for (const child of this.children) {
      if (child.#matchesSimple(selector)) matches.push(child);
      matches.push(...child.querySelectorAll(selector));
    }
    return matches;
  }

  focus() {
    this.ownerDocument.activeElement = this;
    this.focused = true;
  }

  select() {}

  #matchesSimple(selector) {
    if (!selector.startsWith(".")) return false;
    return this.#classNames().includes(selector.slice(1));
  }

  #classNames() {
    return this.className.split(/\s+/).filter(Boolean);
  }

  #updateClasses(added, removed) {
    const names = new Set(this.#classNames());
    removed.forEach((token) => names.delete(token));
    added.forEach((token) => names.add(token));
    this.className = [...names].join(" ");
  }
}

class FakeList extends FakeElement {
  constructor(ownerDocument) {
    super("ol", ownerDocument);
  }

  emit(type, event) {
    this.listeners.get(type)?.(event);
  }
}
