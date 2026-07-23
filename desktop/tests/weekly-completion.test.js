import assert from "node:assert/strict";
import test from "node:test";

import {
  currentLocalWeekRange,
  formatWeeklyCompletionMarkdown,
  weeklyCompletionEntries,
  WeeklyCompletionController,
} from "../app/weekly-completion-controller.js";
import { TauriClipboardWriter } from "../app/infrastructure/clipboard-writer.js";

test("本地自然周从周一零点开始并在下周一零点结束", () => {
  const range = currentLocalWeekRange(new Date(2026, 6, 22, 16, 35, 20));

  assertLocalDate(range.fromMs, [2026, 6, 20, 0, 0, 0, 0]);
  assertLocalDate(range.toMs, [2026, 6, 27, 0, 0, 0, 0]);
});

test("自然周通过 setDate 正确跨越月份和年份", () => {
  const range = currentLocalWeekRange(new Date(2027, 0, 1, 8, 0));

  assertLocalDate(range.fromMs, [2026, 11, 28, 0, 0, 0, 0]);
  assertLocalDate(range.toMs, [2027, 0, 4, 0, 0, 0, 0]);
});

test("完成记录按事实顺序生成 Markdown 编号列表", () => {
  const markdown = formatWeeklyCompletionMarkdown([
    { eventType: "completed", taskId: "task-1", titleSnapshot: "完成月度巡检" },
    { eventType: "completed", taskId: "task-2", titleSnapshot: "更新部署\n说明" },
  ]);

  assert.equal(markdown, "## 本周完成\n\n1. 完成月度巡检\n2. 更新部署 说明");
});

test("父项本周完成时按父项去重并把子代办作为缩进明细", () => {
  const completions = [
    subtaskCompletion("child-1", "汇总完成", "parent", "写周报", 1),
    subtaskCompletion("child-2", "整理问题", "parent", "写周报", 2),
    { eventType: "completed", taskId: "parent", titleSnapshot: "写周报", occurredAtMs: 3 },
  ];

  assert.deepEqual(weeklyCompletionEntries(completions), [{
    title: "写周报",
    children: ["汇总完成", "整理问题"],
  }]);
  assert.equal(
    formatWeeklyCompletionMarkdown(completions),
    "## 本周完成\n\n1. 写周报\n   - 汇总完成\n   - 整理问题",
  );
});

test("父项尚未完成时以父子路径输出子代办，不平铺成重复父项", () => {
  const completions = [
    subtaskCompletion("child-1", "核对版本", "parent", "自动更新测试", 1),
    subtaskCompletion("child-2", "检查签名", "parent", "自动更新测试", 2),
  ];

  assert.equal(
    formatWeeklyCompletionMarkdown(completions),
    "## 本周完成\n\n1. 自动更新测试 / 核对版本\n2. 自动更新测试 / 检查签名",
  );
});

test("复制本周完成时查询自然周、写入 Markdown 并提示数量", async () => {
  const now = new Date(2026, 6, 22, 16, 35);
  const expectedRange = currentLocalWeekRange(now);
  const fixture = createControllerFixture({
    now,
    completions: [
      completion("完成 A", expectedRange.fromMs),
      completion("完成 B", expectedRange.toMs - 1),
    ],
  });

  const result = await fixture.controller.copyCurrentWeek();

  assert.deepEqual(fixture.factRequests, [expectedRange]);
  assert.deepEqual(fixture.writes, ["## 本周完成\n\n1. 完成 A\n2. 完成 B"]);
  assert.deepEqual(fixture.messages, ["已复制本周完成（2 项）"]);
  assert.equal(result.copied, true);
  assert.equal(result.count, 2);
});

test("父项本周完成时允许带入本周前完成的子代办", async () => {
  const now = new Date(2026, 6, 22, 16, 35);
  const range = currentLocalWeekRange(now);
  const fixture = createControllerFixture({
    now,
    completions: [
      subtaskCompletion("child-1", "前周完成的子项", "parent", "写周报", range.fromMs - 1),
      { eventType: "completed", taskId: "parent", titleSnapshot: "写周报", occurredAtMs: range.fromMs + 1 },
    ],
  });

  const result = await fixture.controller.copyCurrentWeek();

  assert.equal(result.copied, true);
  assert.equal(result.markdown, "## 本周完成\n\n1. 写周报\n   - 前周完成的子项");
});

test("本周前子完成只有对应父项本周完成时才合法", async () => {
  const now = new Date(2026, 6, 22, 16, 35);
  const range = currentLocalWeekRange(now);
  const fixture = createControllerFixture({
    now,
    completions: [
      subtaskCompletion("child-1", "过早子项", "parent-a", "父项 A", range.fromMs - 1),
      { eventType: "completed", taskId: "parent-b", titleSnapshot: "父项 B", occurredAtMs: range.fromMs + 1 },
    ],
  });

  await assert.rejects(
    fixture.controller.copyCurrentWeek(),
    /本地账本返回了无效的本周完成记录/,
  );
  assert.deepEqual(fixture.writes, []);
});

test("本周没有完成记录时不覆盖剪贴板", async () => {
  const fixture = createControllerFixture({
    now: new Date(2026, 6, 22, 16, 35),
    completions: [],
  });

  const result = await fixture.controller.copyCurrentWeek();

  assert.deepEqual(fixture.writes, []);
  assert.deepEqual(fixture.messages, ["本周还没有完成记录"]);
  assert.equal(result.copied, false);
  assert.equal(result.count, 0);
});

test("剪贴板写入失败时不显示成功提示", async () => {
  const now = new Date(2026, 6, 22, 16, 35);
  const range = currentLocalWeekRange(now);
  const messages = [];
  const controller = new WeeklyCompletionController({
    gateway: {
      async weeklyFacts(fromMs, toMs) {
        return {
          fromMs,
          toMs,
          effectiveCompletions: [completion("完成 A", range.fromMs)],
          ongoingTasks: [],
        };
      },
    },
    clipboard: {
      async writeText() { throw new Error("剪贴板暂不可用"); },
    },
    toast: { show(message) { messages.push(message); } },
    now: () => new Date(now),
  });

  await assert.rejects(controller.copyCurrentWeek(), /剪贴板暂不可用/);
  assert.deepEqual(messages, []);
});

test("周报事实结构或返回区间异常时拒绝写入剪贴板", async () => {
  const now = new Date(2026, 6, 22, 16, 35);
  const fixture = createControllerFixture({
    now,
    factsFactory: ({ fromMs, toMs }) => ({
      fromMs: fromMs + 1,
      toMs,
      effectiveCompletions: [completion("错误区间", fromMs + 1)],
      ongoingTasks: [],
    }),
  });

  await assert.rejects(
    fixture.controller.copyCurrentWeek(),
    /本地账本返回了无效的本周完成记录/,
  );
  assert.deepEqual(fixture.writes, []);
  assert.deepEqual(fixture.messages, []);
});

test("Tauri 剪贴板适配器只转发纯文本写入", async () => {
  const writes = [];
  const writer = TauriClipboardWriter.fromWindow({
    __TAURI__: {
      clipboardManager: {
        marker: "tauri-clipboard",
        async writeText(text) { writes.push([this.marker, text]); },
      },
    },
  });

  await writer.writeText("本周完成");

  assert.deepEqual(writes, [["tauri-clipboard", "本周完成"]]);
  await assert.rejects(writer.writeText(null), /剪贴板内容必须是文本/);
});

test("Tauri 剪贴板能力不存在时显式报错且不静默回退", async () => {
  const writer = TauriClipboardWriter.fromWindow({ __TAURI__: {} });

  await assert.rejects(
    writer.writeText("本周完成"),
    /系统剪贴板写入能力不可用/,
  );
});

function createControllerFixture({ now, completions = [], factsFactory = null }) {
  const factRequests = [];
  const writes = [];
  const messages = [];
  const gateway = {
    async weeklyFacts(fromMs, toMs) {
      const range = { fromMs, toMs };
      factRequests.push(range);
      return factsFactory?.(range) ?? {
        ...range,
        effectiveCompletions: completions,
        ongoingTasks: [],
      };
    },
  };
  const controller = new WeeklyCompletionController({
    gateway,
    clipboard: { async writeText(text) { writes.push(text); } },
    toast: { show(message) { messages.push(message); } },
    now: () => new Date(now),
  });
  return { controller, factRequests, messages, writes };
}

function completion(titleSnapshot, occurredAtMs) {
  return {
    eventType: "completed",
    taskId: `task-${titleSnapshot}`,
    titleSnapshot,
    occurredAtMs,
  };
}

function subtaskCompletion(taskId, titleSnapshot, parentTaskId, parentTitle, occurredAtMs) {
  return {
    eventType: "subtask_completed",
    taskId,
    titleSnapshot,
    occurredAtMs,
    metadata: { parentTaskId, parentTitle },
  };
}

function assertLocalDate(timestamp, expected) {
  const value = new Date(timestamp);
  assert.deepEqual(
    [
      value.getFullYear(),
      value.getMonth(),
      value.getDate(),
      value.getHours(),
      value.getMinutes(),
      value.getSeconds(),
      value.getMilliseconds(),
    ],
    expected,
  );
}
