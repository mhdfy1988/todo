import { LedgerPhase } from "./state.js";

export function activeCompletionEvents(snapshot) {
  return [...snapshot.effectiveCompletions];
}

/** 按顶层队列顺序构造只读父子组；子代办事实始终来自 snapshot.subtasks。 */
export function taskGroups(snapshot) {
  const byParent = groupSubtasks(snapshot?.subtasks ?? []);
  return (snapshot?.queue ?? []).map((task) => createTaskGroup(task, byParent.get(task.id) ?? []));
}

export function taskGroupFor(snapshot, parentTaskId) {
  const task = (snapshot?.queue ?? []).find((item) => item.id === parentTaskId) ?? null;
  if (!task) return null;
  const subtasks = (snapshot?.subtasks ?? [])
    .filter((item) => item.parentTaskId === parentTaskId && item.status !== "abandoned")
    .sort(compareSiblingPosition);
  return createTaskGroup(task, subtasks);
}

export function currentAction(snapshot) {
  const parentTask = snapshot?.currentTask ?? null;
  if (!parentTask) return null;
  const group = taskGroupFor(snapshot, parentTask.id)
    ?? createTaskGroup(parentTask, []);
  const pendingSubtask = group.subtasks.find((subtask) => subtask.status === "pending") ?? null;
  return Object.freeze({
    parentTask,
    task: pendingSubtask ?? parentTask,
    isSubtask: Boolean(pendingSubtask),
    completedCount: group.completedCount,
    totalCount: group.totalCount,
  });
}

/** 搜索父、子标题并保留父级上下文与整组真实进度。 */
export function filterTaskGroupsByTitle(groups, query) {
  const normalizedQuery = normalizeSearchQuery(query);
  if (!normalizedQuery) {
    return groups.map((group) => ({
      ...group,
      parentMatches: true,
      matchingSubtasks: group.subtasks,
      searchExpanded: false,
    }));
  }
  return groups.flatMap((group) => {
    const parentMatches = includesQuery(group.task.title, normalizedQuery);
    const matchingSubtasks = group.subtasks.filter((subtask) => (
      includesQuery(subtask.title, normalizedQuery)
    ));
    if (!parentMatches && matchingSubtasks.length === 0) return [];
    return [{
      ...group,
      parentMatches,
      matchingSubtasks: parentMatches ? group.subtasks : matchingSubtasks,
      searchExpanded: !parentMatches && matchingSubtasks.length > 0,
    }];
  });
}

/** 完成事实按顶层父代办归组；独立代办保持普通单行。 */
export function completionGroups(snapshot) {
  const completions = activeCompletionEvents(snapshot)
    .sort((left, right) => right.occurredAtMs - left.occurredAtMs);
  const activeParents = new Map((snapshot?.queue ?? []).map((task) => [task.id, task]));
  const currentSubtasks = groupSubtasks(snapshot?.subtasks ?? []);
  const groups = new Map();

  completions.forEach((event) => {
    if (event.eventType === "completed") {
      const activeParent = activeParents.get(event.taskId);
      const group = ensureCompletionGroup(
        groups,
        event.taskId,
        activeParent?.title || event.titleSnapshot,
      );
      if (!group.parentCompletion) group.parentCompletion = event;
      if (activeParent) group.title = activeParent.title;
      return;
    }
    const parentTaskId = eventParentTaskId(event);
    if (!parentTaskId) return;
    const activeParent = activeParents.get(parentTaskId);
    const title = activeParent?.title || eventParentTitle(event) || "已删除的代办";
    const group = ensureCompletionGroup(groups, parentTaskId, title);
    if (activeParent) group.title = activeParent.title;
    group.subtaskCompletions.push(event);
  });

  groups.forEach((group) => {
    const subtasks = currentSubtasks.get(group.parentTaskId) ?? [];
    group.activeParent = activeParents.has(group.parentTaskId);
    group.totalCount = subtasks.length;
    group.completedCount = group.subtaskCompletions.length;
    group.subtaskCompletions.sort(compareCompletionDetail);
    group.occurredAtMs = Math.max(
      group.parentCompletion?.occurredAtMs ?? 0,
      ...group.subtaskCompletions.map((event) => event.occurredAtMs),
    );
  });

  return [...groups.values()].sort((left, right) => right.occurredAtMs - left.occurredAtMs);
}

export function filterCompletionGroupsByTitle(groups, query) {
  const normalizedQuery = normalizeSearchQuery(query);
  if (!normalizedQuery) {
    return groups.map((group) => ({
      ...group,
      matchingSubtaskCompletions: group.subtaskCompletions,
      searchExpanded: false,
    }));
  }
  return groups.flatMap((group) => {
    const parentMatches = includesQuery(group.title, normalizedQuery);
    const matchingSubtaskCompletions = group.subtaskCompletions.filter((event) => (
      includesQuery(event.titleSnapshot, normalizedQuery)
    ));
    if (!parentMatches && matchingSubtaskCompletions.length === 0) return [];
    return [{
      ...group,
      matchingSubtaskCompletions: parentMatches
        ? group.subtaskCompletions
        : matchingSubtaskCompletions,
      searchExpanded: !parentMatches && matchingSubtaskCompletions.length > 0,
    }];
  });
}

export function upcomingTasks(snapshot) {
  return snapshot.queue.slice(1, 4);
}

export function normalizeSearchQuery(query) {
  return String(query ?? "").trim().toLocaleLowerCase("zh-CN");
}

export function filterTasksByTitle(tasks, query) {
  return filterByTitle(tasks, query, (task) => task.title);
}

export function filterCompletionEventsByTitle(events, query) {
  const normalizedQuery = normalizeSearchQuery(query);
  if (!normalizedQuery) return [...events];
  return events.filter((event) => includesQuery(event.titleSnapshot, normalizedQuery)
    || includesQuery(eventParentTitle(event), normalizedQuery));
}

export function titleMatchRanges(title, query) {
  const normalizedTitle = String(title ?? "").toLocaleLowerCase("zh-CN");
  const normalizedQuery = normalizeSearchQuery(query);
  if (!normalizedQuery) return [];
  const ranges = [];
  let offset = 0;
  while (offset < normalizedTitle.length) {
    const index = normalizedTitle.indexOf(normalizedQuery, offset);
    if (index < 0) break;
    ranges.push([index, index + normalizedQuery.length]);
    offset = index + normalizedQuery.length;
  }
  return ranges;
}

function filterByTitle(items, query, titleOf) {
  const normalizedQuery = normalizeSearchQuery(query);
  if (!normalizedQuery) return [...items];
  return items.filter((item) => String(titleOf(item) ?? "")
    .toLocaleLowerCase("zh-CN")
    .includes(normalizedQuery));
}

function groupSubtasks(subtasks) {
  const groups = new Map();
  subtasks
    .filter((subtask) => subtask?.parentTaskId && subtask.status !== "abandoned")
    .sort(compareSiblingPosition)
    .forEach((subtask) => {
      const existing = groups.get(subtask.parentTaskId) ?? [];
      existing.push(subtask);
      groups.set(subtask.parentTaskId, existing);
    });
  return groups;
}

function createTaskGroup(task, subtasks) {
  const completedCount = subtasks.filter((subtask) => subtask.status === "completed").length;
  return Object.freeze({
    task,
    subtasks: Object.freeze([...subtasks]),
    completedCount,
    totalCount: subtasks.length,
    firstPendingSubtask: subtasks.find((subtask) => subtask.status === "pending") ?? null,
  });
}

function compareSiblingPosition(left, right) {
  return (left.siblingPosition ?? Number.MAX_SAFE_INTEGER)
    - (right.siblingPosition ?? Number.MAX_SAFE_INTEGER);
}

function compareCompletionDetail(left, right) {
  const timeDifference = left.occurredAtMs - right.occurredAtMs;
  if (timeDifference !== 0) return timeDifference;
  return (left.sequence ?? Number.MAX_SAFE_INTEGER)
    - (right.sequence ?? Number.MAX_SAFE_INTEGER);
}

function includesQuery(value, normalizedQuery) {
  return String(value ?? "").toLocaleLowerCase("zh-CN").includes(normalizedQuery);
}

function eventParentTaskId(event) {
  return typeof event?.metadata?.parentTaskId === "string"
    ? event.metadata.parentTaskId
    : null;
}

function eventParentTitle(event) {
  return typeof event?.metadata?.parentTitle === "string"
    ? event.metadata.parentTitle
    : "";
}

function ensureCompletionGroup(groups, parentTaskId, title) {
  let group = groups.get(parentTaskId);
  if (group) return group;
  group = {
    parentTaskId,
    title,
    parentCompletion: null,
    subtaskCompletions: [],
    completedCount: 0,
    totalCount: 0,
    occurredAtMs: 0,
    activeParent: false,
  };
  groups.set(parentTaskId, group);
  return group;
}

export function ledgerContentKind(state) {
  if (state.phase === LedgerPhase.ERROR) return "error";
  if (state.snapshotReady) return "snapshot";
  if (state.phase === LedgerPhase.RECOVERY) return "recovery";
  return "loading";
}

export function diagnosticsPassed(status, integrity) {
  return Boolean(
    status.inWorkArea
      && status.alwaysOnTop
      && status.trayReady
      && integrity.sqliteQuickCheck
      && integrity.foreignKeys
      && integrity.rewardPrefixBalances
      && integrity.eventRewardLinks
      && integrity.receiptLinks
      && integrity.taskRewardNetValues
      && integrity.taskProjectionMatchesLedger
      && integrity.taskHierarchyValid
      && Array.isArray(integrity.failures)
      && integrity.failures.length === 0,
  );
}

export function formatTime(timestamp) {
  const value = new Date(timestamp);
  if (Number.isNaN(value.getTime())) return "";
  return new Intl.DateTimeFormat("zh-CN", {
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(value);
}
