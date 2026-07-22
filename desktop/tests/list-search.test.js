import test from "node:test";
import assert from "node:assert/strict";

import {
  filterCompletionEventsByTitle,
  filterTasksByTitle,
  normalizeSearchQuery,
  titleMatchRanges,
} from "../app/selectors.js";
import {
  isSearchShortcut,
  ListSearchController,
} from "../app/list-search-controller.js";

test("空查询返回保持原顺序的新数组", () => {
  const tasks = [
    { id: "task-1", title: "第一项" },
    { id: "task-2", title: "第二项" },
  ];

  for (const query of ["", "   ", null, undefined]) {
    const result = filterTasksByTitle(tasks, query);
    assert.deepEqual(result, tasks);
    assert.notEqual(result, tasks);
  }
  assert.equal(normalizeSearchQuery("  API 待办  "), "api 待办");
});

test("待办标题按中文和英文大小写做包含匹配且不改变队列顺序", () => {
  const tasks = [
    { id: "task-1", title: "整理 API 文档" },
    { id: "task-2", title: "买菜" },
    { id: "task-3", title: "api 回归" },
    { id: "task-4", title: "整理周报" },
  ];

  assert.deepEqual(
    filterTasksByTitle(tasks, " ApI ").map((task) => task.id),
    ["task-1", "task-3"],
  );
  assert.deepEqual(
    filterTasksByTitle(tasks, "整理").map((task) => task.id),
    ["task-1", "task-4"],
  );
  assert.deepEqual(tasks.map((task) => task.id), ["task-1", "task-2", "task-3", "task-4"]);
});

test("完成记录只使用 titleSnapshot 过滤并保持事件顺序", () => {
  const events = [
    { id: "event-1", titleSnapshot: "完成 API 文档", title: "不应读取这个字段" },
    { id: "event-2", titleSnapshot: "整理周报", title: "API" },
    { id: "event-3", titleSnapshot: "api 回归", title: "无关" },
  ];

  assert.deepEqual(
    filterCompletionEventsByTitle(events, "API").map((event) => event.id),
    ["event-1", "event-3"],
  );
  assert.deepEqual(
    filterCompletionEventsByTitle(events, "周报").map((event) => event.id),
    ["event-2"],
  );
});

test("高亮范围生成互不重叠的多段安全切片", () => {
  assert.deepEqual(titleMatchRanges("待办和待办", "待办"), [[0, 2], [3, 5]]);
  assert.deepEqual(titleMatchRanges("API api Api", "aPi"), [[0, 3], [4, 7], [8, 11]]);

  const title = "aaaaa";
  const ranges = titleMatchRanges(title, "aa");
  assert.deepEqual(ranges, [[0, 2], [2, 4]]);
  ranges.forEach(([start, end], index) => {
    assert.ok(start >= 0);
    assert.ok(end <= title.length);
    assert.ok(start < end);
    if (index > 0) assert.ok(ranges[index - 1][1] <= start);
  });
  assert.deepEqual(titleMatchRanges("待办", ""), []);
  assert.deepEqual(titleMatchRanges("短", "更长的查询"), []);
});

test("搜索控制器打开待办搜索、接收输入并由取消按钮关闭", () => {
  const harness = createSearchHarness();

  assert.deepEqual(harness.controller.state, { panel: null, query: "" });
  assert.equal(harness.searchAction.textContent, "搜索待办");
  assert.equal(harness.searchAction.getAttribute("aria-label"), "搜索待办");

  assert.equal(harness.controller.open("tasks"), true);
  assert.deepEqual(harness.controller.state, { panel: "tasks", query: "" });
  assert.equal(harness.form.hidden, false);
  assert.equal(harness.captureForm.hidden, true);
  assert.equal(harness.historyHeading.hidden, false);
  assert.equal(harness.root.dataset.searchPanel, "tasks");
  assert.equal(harness.label.textContent, "搜索待办");
  assert.equal(harness.input.placeholder, "搜索待办");
  assert.equal(harness.input.getAttribute("aria-label"), "搜索待办");
  assert.equal(harness.input.getAttribute("aria-controls"), "taskList");
  assert.equal(harness.input.focusCount, 1);
  assert.equal(harness.input.selectCount, 1);

  harness.input.value = "  API  ";
  harness.input.emit("input");
  assert.deepEqual(harness.controller.state, { panel: "tasks", query: "API" });
  assert.deepEqual(harness.changes.at(-1), { panel: "tasks", query: "API" });

  harness.cancelButton.emit("click");
  assert.deepEqual(harness.controller.state, { panel: null, query: "" });
  assert.equal(harness.form.hidden, true);
  assert.equal(harness.captureForm.hidden, false);
  assert.equal(harness.historyHeading.hidden, false);
  assert.equal("searchPanel" in harness.root.dataset, false);
  assert.equal(harness.taskTitleInput.focusCount, 1);
  assert.equal(harness.controller.close(), false);
});

test("待办输入不可聚焦时取消搜索回到更多菜单按钮", () => {
  const harness = createSearchHarness();
  harness.taskTitleInput.disabled = true;

  harness.controller.open("tasks");
  harness.cancelButton.emit("click");

  assert.equal(harness.taskTitleInput.focusCount, 0);
  assert.equal(harness.menuButton.focusCount, 1);
});

test("完成记录搜索使用对应文案、列表关联和返回焦点", () => {
  const harness = createSearchHarness();

  harness.controller.setPanel("history");
  assert.equal(harness.searchAction.textContent, "搜索完成记录");
  assert.equal(harness.searchAction.getAttribute("aria-label"), "搜索完成记录");

  harness.controller.open();
  assert.deepEqual(harness.controller.state, { panel: "history", query: "" });
  assert.equal(harness.captureForm.hidden, false);
  assert.equal(harness.historyHeading.hidden, true);
  assert.equal(harness.input.placeholder, "搜索完成记录");
  assert.equal(harness.input.getAttribute("aria-controls"), "historyList");

  harness.controller.close();
  assert.equal(harness.historyBackButton.focusCount, 1);
  assert.equal(harness.taskTitleInput.focusCount, 0);
  assert.throws(() => harness.controller.setPanel("unknown"), /未知搜索面板/);
});

test("切换搜索面板会清空旧查询且不把焦点还给旧面板", () => {
  const harness = createSearchHarness();
  harness.controller.open("tasks");
  harness.input.value = "旧查询";
  harness.input.emit("input");

  assert.equal(harness.controller.open("history"), true);
  assert.deepEqual(harness.controller.state, { panel: "history", query: "" });
  assert.equal(harness.input.value, "");
  assert.equal(harness.taskTitleInput.focusCount, 0);
  assert.deepEqual(harness.changes.slice(-2), [
    { panel: null, query: "" },
    { panel: "history", query: "" },
  ]);
});

test("输入法组合期间不发布半成品，compositionend 后只发布完整查询", () => {
  const harness = createSearchHarness();
  harness.controller.open("tasks");
  const changeCountAfterOpen = harness.changes.length;

  harness.input.emit("compositionstart");
  assert.equal(harness.controller.isComposing(), true);
  harness.input.value = "代";
  harness.input.emit("input");
  harness.input.value = "代办";
  harness.input.emit("input");

  assert.deepEqual(harness.controller.state, { panel: "tasks", query: "" });
  assert.equal(harness.changes.length, changeCountAfterOpen);

  harness.input.emit("compositionend");
  assert.equal(harness.controller.isComposing(), false);
  assert.deepEqual(harness.controller.state, { panel: "tasks", query: "代办" });
  assert.equal(harness.changes.length, changeCountAfterOpen + 1);
  assert.deepEqual(harness.changes.at(-1), { panel: "tasks", query: "代办" });

  // WebView 会在 compositionend 后补发同值 input，不应造成第二次重绘。
  harness.input.emit("input");
  assert.equal(harness.changes.length, changeCountAfterOpen + 1);
});

test("搜索表单阻止提交，未打开时输入不会发布状态", () => {
  const harness = createSearchHarness();
  const submit = harness.form.emit("submit");
  assert.equal(submit.defaultPrevented, true);

  harness.input.value = "不会生效";
  harness.input.emit("input");
  assert.deepEqual(harness.controller.state, { panel: null, query: "" });
  assert.deepEqual(harness.changes, []);
});

test("Ctrl 或 Cmd 加 F 才识别为搜索快捷键，Alt 组合不拦截", () => {
  assert.equal(isSearchShortcut({ key: "f", ctrlKey: true }), true);
  assert.equal(isSearchShortcut({ key: "F", ctrlKey: true }), true);
  assert.equal(isSearchShortcut({ key: "f", metaKey: true }), true);
  assert.equal(isSearchShortcut({ key: "F", ctrlKey: true, shiftKey: true }), true);
  assert.equal(isSearchShortcut({ key: "f", ctrlKey: true, altKey: true }), false);
  assert.equal(isSearchShortcut({ key: "f" }), false);
  assert.equal(isSearchShortcut({ key: "g", ctrlKey: true }), false);
});

function createSearchHarness() {
  const root = createElement();
  const form = createElement({ hidden: true });
  const label = createElement();
  const input = createElement();
  const cancelButton = createElement();
  const searchAction = createElement();
  const captureForm = createElement();
  const taskTitleInput = createElement();
  const historyHeading = createElement();
  const historyBackButton = createElement();
  const menuButton = createElement();
  const changes = [];
  const controller = new ListSearchController({
    root,
    form,
    label,
    input,
    cancelButton,
    searchAction,
    captureForm,
    taskTitleInput,
    historyHeading,
    historyBackButton,
    menuButton,
    onChange: (state) => changes.push({ ...state }),
  });
  return {
    controller,
    root,
    form,
    label,
    input,
    cancelButton,
    searchAction,
    captureForm,
    taskTitleInput,
    historyHeading,
    historyBackButton,
    menuButton,
    changes,
  };
}

function createElement({ hidden = false, disabled = false } = {}) {
  const listeners = new Map();
  const attributes = new Map();
  return {
    hidden,
    disabled,
    dataset: {},
    textContent: "",
    placeholder: "",
    value: "",
    focusCount: 0,
    selectCount: 0,
    addEventListener(type, listener) {
      const current = listeners.get(type) ?? [];
      current.push(listener);
      listeners.set(type, current);
    },
    emit(type, patch = {}) {
      const event = {
        defaultPrevented: false,
        preventDefault() {
          this.defaultPrevented = true;
        },
        ...patch,
      };
      (listeners.get(type) ?? []).forEach((listener) => listener(event));
      return event;
    },
    setAttribute(name, value) {
      attributes.set(name, String(value));
    },
    getAttribute(name) {
      return attributes.get(name) ?? null;
    },
    focus() {
      this.focusCount += 1;
    },
    select() {
      this.selectCount += 1;
    },
  };
}
