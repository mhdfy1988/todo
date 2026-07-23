import assert from "node:assert/strict";
import test from "node:test";

import {
  activeCompletionEvents,
  completionGroups,
  currentAction,
  filterCompletionGroupsByTitle,
  filterTaskGroupsByTitle,
  taskGroups,
} from "../app/selectors.js";

test("父子分组按 siblingPosition 排序并派生真实进度与当前动作", () => {
  const parent = task("parent", "写周报");
  const snapshot = snapshotOf({
    queue: [parent],
    subtasks: [
      subtask("child-3", parent.id, "写计划", 3, "pending"),
      subtask("child-1", parent.id, "汇总完成", 1, "completed"),
      subtask("child-2", parent.id, "整理问题", 2, "pending"),
    ],
  });

  const [group] = taskGroups(snapshot);
  assert.deepEqual(group.subtasks.map((item) => item.id), ["child-1", "child-2", "child-3"]);
  assert.equal(group.completedCount, 1);
  assert.equal(group.totalCount, 3);
  assert.equal(group.firstPendingSubtask.id, "child-2");
  assert.deepEqual(currentAction(snapshot), {
    parentTask: parent,
    task: group.firstPendingSubtask,
    isSubtask: true,
    completedCount: 1,
    totalCount: 3,
  });
});

test("子代办全部完成后当前动作回到父代办且不会自动完成父项", () => {
  const parent = task("parent", "写周报");
  const snapshot = snapshotOf({
    queue: [parent],
    subtasks: [
      subtask("child-1", parent.id, "汇总完成", 1, "completed"),
      subtask("child-2", parent.id, "整理问题", 2, "completed"),
    ],
  });

  assert.deepEqual(currentAction(snapshot), {
    parentTask: parent,
    task: parent,
    isSubtask: false,
    completedCount: 2,
    totalCount: 2,
  });
});

test("待办搜索命中子标题时保留父上下文与整组真实进度", () => {
  const parent = task("parent", "写周报");
  const [group] = taskGroups(snapshotOf({
    queue: [parent],
    subtasks: [
      subtask("child-1", parent.id, "汇总完成", 1, "completed"),
      subtask("child-2", parent.id, "整理问题", 2, "pending"),
      subtask("child-3", parent.id, "写计划", 3, "pending"),
    ],
  }));

  const [result] = filterTaskGroupsByTitle([group], "问题");
  assert.equal(result.task.id, parent.id);
  assert.equal(result.parentMatches, false);
  assert.equal(result.searchExpanded, true);
  assert.deepEqual(result.matchingSubtasks.map((item) => item.id), ["child-2"]);
  assert.deepEqual([result.completedCount, result.totalCount], [1, 3]);

  const [parentResult] = filterTaskGroupsByTitle([group], "周报");
  assert.equal(parentResult.searchExpanded, false);
  assert.deepEqual(parentResult.matchingSubtasks.map((item) => item.id), [
    "child-1", "child-2", "child-3",
  ]);
});

test("已删除父组的历史总数只使用完整子项投影，不从截断审计事件猜测", () => {
  const subtasks = [
    subtask("child-1", "parent", "汇总完成", 1, "completed"),
    subtask("child-2", "parent", "整理问题", 2, "pending"),
    subtask("child-3", "parent", "写计划", 3, "pending"),
  ];
  const effectiveCompletions = [
    completed("done-1", "child-1", "汇总完成", "parent", "写周报", 10),
  ];

  const [group] = completionGroups(snapshotOf({
    subtasks,
    effectiveCompletions,
    events: [created("misleading-create", "child-1", "parent", "旧标题")],
  }));
  assert.equal(group.activeParent, false);
  assert.equal(group.parentCompletion, null);
  assert.deepEqual([group.completedCount, group.totalCount], [1, 3]);
});

test("父项完成历史使用有效完成事实和完整子项投影保持 3/3", () => {
  const effectiveCompletions = [
    completed("done-1", "child-1", "汇总完成", "parent", "写周报", 10),
    completed("done-2", "child-2", "整理问题", "parent", "写周报", 11),
    completed("done-3", "child-3", "写计划", "parent", "写周报", 12),
    { id: "parent-done", taskId: "parent", titleSnapshot: "写周报", eventType: "completed", occurredAtMs: 13, metadata: {} },
  ];
  const [group] = completionGroups(snapshotOf({
    completed: [{ ...task("parent", "写周报"), status: "completed" }],
    subtasks: [
      subtask("child-1", "parent", "汇总完成", 1, "completed"),
      subtask("child-2", "parent", "整理问题", 2, "completed"),
      subtask("child-3", "parent", "写计划", 3, "completed"),
    ],
    effectiveCompletions,
  }));
  assert.deepEqual([group.completedCount, group.totalCount], [3, 3]);
  assert.equal(group.parentCompletion.id, "parent-done");
});

test("同毫秒子完成明细按事件 sequence 升序且不改变原有时间顺序", () => {
  const [group] = completionGroups(snapshotOf({
    subtasks: [
      subtask("child-1", "parent", "第一项", 1, "completed"),
      subtask("child-2", "parent", "第二项", 2, "completed"),
      subtask("child-3", "parent", "第三项", 3, "completed"),
      subtask("child-older", "parent", "更早完成", 4, "completed"),
    ],
    effectiveCompletions: [
      completed("done-3", "child-3", "第三项", "parent", "写周报", 100, 13),
      completed("done-2", "child-2", "第二项", "parent", "写周报", 100, 12),
      completed("done-1", "child-1", "第一项", "parent", "写周报", 100, 11),
      completed("done-older", "child-older", "更早完成", "parent", "写周报", 90, 10),
    ],
  }));

  assert.deepEqual(group.subtaskCompletions.map((event) => event.id), [
    "done-older", "done-1", "done-2", "done-3",
  ]);
});

test("同毫秒旧完成事实的 sequence 为 null 或缺失时排在有序号事实之后", () => {
  const missingSequence = completed(
    "legacy-missing",
    "child-missing",
    "缺少序号",
    "parent",
    "写周报",
    100,
  );
  delete missingSequence.sequence;

  const [group] = completionGroups(snapshotOf({
    subtasks: [
      subtask("child-1", "parent", "第一项", 1, "completed"),
      subtask("child-2", "parent", "第二项", 2, "completed"),
      subtask("child-null", "parent", "空序号", 3, "completed"),
      subtask("child-missing", "parent", "缺少序号", 4, "completed"),
    ],
    effectiveCompletions: [
      missingSequence,
      completed("legacy-null", "child-null", "空序号", "parent", "写周报", 100),
      completed("done-2", "child-2", "第二项", "parent", "写周报", 100, 12),
      completed("done-1", "child-1", "第一项", "parent", "写周报", 100, 11),
    ],
  }));

  const ids = group.subtaskCompletions.map((event) => event.id);
  assert.deepEqual(ids.slice(0, 2), ["done-1", "done-2"]);
  assert.deepEqual(new Set(ids.slice(2)), new Set(["legacy-null", "legacy-missing"]));
});

test("有效完成事实只使用后端专用投影，不回退到可能截断的 events", () => {
  const effectiveCompletions = [
    completed("done-1", "child-1", "汇总完成", "parent", "写周报", 10),
    completed("done-3", "child-3", "写计划", "parent", "写周报", 12),
  ];
  const snapshot = snapshotOf({
    effectiveCompletions,
    events: [
      completed("stale-done", "child-2", "旧完成", "parent", "旧标题", 1),
      { id: "undo-2", taskId: "child-2", titleSnapshot: "旧完成", eventType: "subtask_completion_undone", occurredAtMs: 2, reversesEventId: "stale-done", metadata: { parentTaskId: "parent", parentTitle: "旧标题" } },
    ],
  });

  assert.deepEqual(activeCompletionEvents(snapshot).map((event) => event.id), [
    "done-1", "done-3",
  ]);
  assert.notEqual(activeCompletionEvents(snapshot), effectiveCompletions);
});

test("完成记录搜索可由父标题命中全部明细，也可由子标题临时展开", () => {
  const groups = completionGroups(snapshotOf({
    subtasks: [
      subtask("child-1", "parent", "汇总完成", 1, "completed"),
      subtask("child-2", "parent", "整理问题", 2, "completed"),
    ],
    effectiveCompletions: [
      completed("done-1", "child-1", "汇总完成", "parent", "写周报", 10),
      completed("done-2", "child-2", "整理问题", "parent", "写周报", 11),
    ],
  }));

  const [byParent] = filterCompletionGroupsByTitle(groups, "周报");
  assert.equal(byParent.searchExpanded, false);
  assert.equal(byParent.matchingSubtaskCompletions.length, 2);

  const [byChild] = filterCompletionGroupsByTitle(groups, "问题");
  assert.equal(byChild.searchExpanded, true);
  assert.deepEqual(byChild.matchingSubtaskCompletions.map((event) => event.id), ["done-2"]);
});

test("进行中父组始终优先当前标题，不被旧子完成快照覆盖", () => {
  const parent = task("parent", "写周报（当前标题）");
  const [group] = completionGroups(snapshotOf({
    queue: [parent],
    subtasks: [subtask("child-1", parent.id, "汇总完成", 1, "completed")],
    effectiveCompletions: [
      completed("done-1", "child-1", "汇总完成", parent.id, "写周报（旧标题）", 10),
    ],
  }));

  assert.equal(group.activeParent, true);
  assert.equal(group.title, "写周报（当前标题）");
});

test("非活动父组按时间倒序保留最新父标题快照", () => {
  const [group] = completionGroups(snapshotOf({
    subtasks: [
      subtask("child-1", "parent", "早期子项", 1, "completed"),
      subtask("child-2", "parent", "近期子项", 2, "completed"),
    ],
    effectiveCompletions: [
      completed("older", "child-1", "早期子项", "parent", "旧父标题", 10),
      completed("newer", "child-2", "近期子项", "parent", "新父标题", 20),
    ],
  }));

  assert.equal(group.activeParent, false);
  assert.equal(group.title, "新父标题");
});

function snapshotOf({
  queue = [],
  completed: completedTasks = [],
  subtasks = [],
  effectiveCompletions = [],
  events = [],
} = {}) {
  return {
    currentTask: queue[0] ?? null,
    queue,
    completed: completedTasks,
    subtasks,
    effectiveCompletions,
    events,
    rewards: [],
    balance: 0,
  };
}

function task(id, title) {
  return { id, title, status: "pending", deadlineOn: null };
}

function subtask(id, parentTaskId, title, siblingPosition, status) {
  return { id, parentTaskId, title, siblingPosition, status, activeCompletionEventId: status === "completed" ? `done-${id}` : null };
}

function created(id, taskId, parentTaskId, parentTitle) {
  return {
    id,
    taskId,
    titleSnapshot: taskId,
    eventType: "subtask_created",
    occurredAtMs: 1,
    metadata: { parentTaskId, parentTitle },
  };
}

function completed(
  id,
  taskId,
  titleSnapshot,
  parentTaskId,
  parentTitle,
  occurredAtMs,
  sequence = null,
) {
  return {
    sequence,
    id,
    taskId,
    titleSnapshot,
    eventType: "subtask_completed",
    occurredAtMs,
    metadata: { parentTaskId, parentTitle },
  };
}
