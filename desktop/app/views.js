import { LedgerPhase } from "./state.js";
import {
  completionGroups,
  currentAction,
  filterCompletionGroupsByTitle,
  filterTaskGroupsByTitle,
  formatTime,
  ledgerContentKind,
  normalizeSearchQuery,
  taskGroups,
  titleMatchRanges,
} from "./selectors.js";
import { deadlinePresentation, localDateOnly } from "./deadline-date.js";

/** 账本投影视图：只把显式状态转换为 DOM，不发起任何命令。 */
export class LedgerView {
  /** @param {Document} document */
  constructor(document) {
    this.document = document;
    this.root = document.body;
    this.captureForm = required(document, "#captureForm");
    this.taskTitleInput = required(document, "#taskTitle");
    this.captureButton = required(this.captureForm, "button[type='submit']");
    this.ledgerStatus = required(document, "#ledgerStatus");
    this.ledgerStatusText = required(document, "#ledgerStatusText");
    this.retryButton = required(document, "#retryButton");
    this.capsuleTitleButton = required(document, "#capsuleTitleButton");
    this.capsuleTaskTitle = required(document, "#capsuleTaskTitle");
    this.capsuleTaskProgress = required(document, "#capsuleTaskProgress");
    this.capsuleTaskDeadline = required(document, "#capsuleTaskDeadline");
    this.capsuleTaskCheckbox = required(document, "#capsuleTaskCheckbox");
    this.taskList = required(document, "#taskList");
    this.taskOrderStatus = required(document, "#taskOrderStatus");
    this.historyLink = required(document, "#historyLink");
    this.historyCount = required(document, "#historyCount");
    this.copyWeeklyCompletionsButton = required(document, "#copyWeeklyCompletionsButton");
    this.historyList = required(document, "#historyList");
    this.listSearchStatus = required(document, "#listSearchStatus");
    this.searchState = Object.freeze({ panel: null, query: "" });
    this.expandedHistoryGroupIds = new Set();
    this.lastState = null;
  }

  /** @param {Object} state */
  render(state) {
    this.lastState = state;
    this.root.dataset.ledgerState = state.phase;
    const contentKind = ledgerContentKind(state);
    if (contentKind === "snapshot") {
      this.#renderSnapshot(state.snapshot);
    } else if (contentKind === "error") {
      this.#renderError();
    } else if (contentKind === "recovery") {
      this.#renderRecovery();
    } else {
      this.#renderLoading();
    }
    this.#renderControls(state);
  }

  /** 搜索是纯视图状态；账本刷新后会继续使用当前查询重绘。 */
  setSearch(searchState) {
    this.searchState = Object.freeze({
      panel: searchState?.panel ?? null,
      query: String(searchState?.query ?? ""),
    });
    if (this.lastState) this.render(this.lastState);
  }

  resetHistoryExpansion() {
    this.expandedHistoryGroupIds.clear();
    if (this.lastState) this.render(this.lastState);
  }

  toggleHistoryGroup(parentTaskId) {
    if (this.expandedHistoryGroupIds.has(parentTaskId)) {
      this.expandedHistoryGroupIds.delete(parentTaskId);
    } else {
      this.expandedHistoryGroupIds.add(parentTaskId);
    }
    if (this.lastState) this.render(this.lastState);
  }

  /** 跨过本地午夜时只刷新日期文案，不重绘列表或打断正在编辑的任务。 */
  refreshDeadlineLabels(todayOn = localDateOnly()) {
    this.taskList.querySelectorAll(".task-deadline").forEach((button) => {
      const deadline = deadlinePresentation(button.dataset.deadlineOn || null, todayOn);
      if (!deadline) return;
      button.className = `task-deadline is-${deadline.state}`;
      button.textContent = deadline.label;
      button.title = deadline.title;
      button.setAttribute("aria-label", `修改${deadline.title}`);
    });
    this.#renderCapsuleDeadline(
      this.capsuleTaskDeadline.dataset.deadlineOn || null,
      todayOn,
    );
  }

  /** @param {Object} snapshot */
  #renderSnapshot(snapshot) {
    const current = snapshot.currentTask;
    const action = currentAction(snapshot);
    if (!this.searchState.panel) this.listSearchStatus.textContent = "";
    this.ledgerStatus.hidden = true;
    this.retryButton.hidden = true;
    const actionTitle = action?.isSubtask
      ? `${action.parentTask.title} / ${action.task.title}`
      : current?.title ?? "暂无待办";
    this.capsuleTaskTitle.textContent = actionTitle;
    this.capsuleTaskTitle.title = actionTitle === "暂无待办" ? "" : actionTitle;
    this.capsuleTaskProgress.hidden = !action?.totalCount;
    this.capsuleTaskProgress.textContent = action?.totalCount
      ? `${action.completedCount}/${action.totalCount}`
      : "";
    this.capsuleTitleButton.setAttribute(
      "aria-label",
      current ? `当前待办：${actionTitle}，展开任务面板` : "暂无待办，展开任务面板",
    );
    this.capsuleTitleButton.dataset.parentTaskId = current?.id ?? "";
    this.capsuleTitleButton.dataset.taskId = action?.task.id ?? "";
    this.capsuleTaskDeadline.dataset.taskId = current?.id ?? "";
    this.#renderCapsuleDeadline(current?.deadlineOn ?? null);
    this.capsuleTaskCheckbox.dataset.taskId = action?.task.id ?? "";
    this.capsuleTaskCheckbox.dataset.parentTaskId = current?.id ?? "";
    this.capsuleTaskCheckbox.dataset.isSubtask = String(Boolean(action?.isSubtask));
    this.capsuleTaskCheckbox.hidden = !action;
    this.capsuleTaskCheckbox.setAttribute(
      "aria-label",
      action ? `完成：${actionTitle}` : "当前没有待办",
    );
    this.#renderTasks(snapshot.queue, current?.id ?? null);
    this.#renderHistory(snapshot);
  }

  #renderLoading() {
    this.#showStatus("正在读取本地账本…");
    this.retryButton.hidden = true;
    this.capsuleTaskTitle.textContent = "本地账本加载中";
    this.capsuleTaskProgress.hidden = true;
    this.capsuleTitleButton.setAttribute("aria-label", "正在读取本地账本");
    this.capsuleTaskDeadline.dataset.taskId = "";
    this.#renderCapsuleDeadline(null);
    this.capsuleTaskCheckbox.dataset.taskId = "";
    this.capsuleTaskCheckbox.hidden = true;
    this.#hideCollections();
  }

  #renderRecovery() {
    this.#showStatus("正在恢复上次操作…");
    this.retryButton.hidden = true;
    this.capsuleTaskTitle.textContent = "正在确认上次操作";
    this.capsuleTaskProgress.hidden = true;
    this.capsuleTitleButton.setAttribute("aria-label", "正在恢复上次操作");
    this.capsuleTaskDeadline.dataset.taskId = "";
    this.#renderCapsuleDeadline(null);
    this.capsuleTaskCheckbox.dataset.taskId = "";
    this.capsuleTaskCheckbox.hidden = true;
    this.#hideCollections();
  }

  #renderError() {
    this.#showStatus("本地账本暂不可用");
    this.retryButton.hidden = false;
    this.capsuleTaskTitle.textContent = "本地账本暂不可用";
    this.capsuleTaskProgress.hidden = true;
    this.capsuleTitleButton.setAttribute("aria-label", "本地账本暂不可用，展开任务面板");
    this.capsuleTaskDeadline.dataset.taskId = "";
    this.#renderCapsuleDeadline(null);
    this.capsuleTaskCheckbox.dataset.taskId = "";
    this.capsuleTaskCheckbox.hidden = true;
    this.#hideCollections();
  }

  /** @param {Array<Object>} tasks @param {string|null} currentTaskId */
  #renderTasks(tasks, currentTaskId) {
    const searching = this.searchState.panel === "tasks";
    const query = searching ? this.searchState.query : "";
    const normalizedQuery = normalizeSearchQuery(query);
    const groups = taskGroups({
      queue: tasks,
      subtasks: this.lastState?.snapshot?.subtasks ?? [],
    });
    const visibleGroups = searching ? filterTaskGroupsByTitle(groups, query) : groups;
    this.taskList.replaceChildren();
    if (tasks.length === 0) {
      this.#renderListStatus(this.taskList, "还没有待办");
      if (searching) this.#announceSearchResult("还没有待办");
      return;
    }
    if (normalizedQuery && visibleGroups.length === 0) {
      this.#renderListStatus(this.taskList, "没有找到相关待办");
      this.#announceSearchResult("没有找到相关待办");
      return;
    }
    if (searching) {
      this.#announceSearchResult(
        normalizedQuery ? `找到 ${visibleGroups.length} 组待办` : `共 ${visibleGroups.length} 项待办`,
      );
    }
    visibleGroups.forEach((group) => {
      const task = group.task;
      const item = this.document.createElement("li");
      const checkbox = this.document.createElement("input");
      const main = this.document.createElement("div");
      const title = this.document.createElement("button");
      const progress = this.document.createElement("button");
      const addFirst = this.document.createElement("button");
      const remove = this.document.createElement("button");
      const handle = this.document.createElement("button");
      const handleMark = this.document.createElement("span");
      const subtaskList = this.document.createElement("ol");
      item.className = "task-row";
      item.dataset.taskId = task.id;
      item.dataset.subtaskCount = String(group.totalCount);
      if (task.id === currentTaskId) item.setAttribute("aria-current", "step");
      checkbox.type = "checkbox";
      checkbox.className = "task-checkbox";
      checkbox.dataset.action = "complete-task";
      checkbox.dataset.taskId = task.id;
      checkbox.setAttribute("aria-label", `完成：${task.title}`);
      main.className = "task-main";
      title.type = "button";
      title.className = "task-title";
      title.dataset.taskId = task.id;
      title.dataset.deadlineOn = task.deadlineOn ?? "";
      title.dataset.title = task.title;
      appendHighlightedText(this.document, title, task.title, query);
      title.title = task.title;
      title.setAttribute("aria-label", `修改待办：${task.title}`);
      if (group.totalCount > 0) {
        main.append(title);
        progress.type = "button";
        progress.className = "subtask-progress-button";
        progress.dataset.parentTaskId = task.id;
        progress.textContent = `${group.completedCount}/${group.totalCount}`;
        progress.setAttribute("aria-expanded", "false");
        progress.setAttribute("aria-controls", `subtasks-${task.id}`);
        progress.setAttribute("aria-label", `${task.title}的子代办，已完成${group.completedCount}项，共${group.totalCount}项`);
        main.append(progress);
      } else if (!searching) {
        const titleSlot = this.document.createElement("div");
        titleSlot.className = "task-title-slot";
        addFirst.type = "button";
        addFirst.className = "subtask-add-trigger";
        addFirst.dataset.parentTaskId = task.id;
        addFirst.textContent = "＋ 子项";
        addFirst.setAttribute("aria-label", `为${task.title}添加子代办`);
        titleSlot.append(title, addFirst);
        main.append(titleSlot);
      } else {
        main.append(title);
      }
      const deadline = deadlinePresentation(task.deadlineOn ?? null);
      if (deadline) {
        const deadlineButton = this.document.createElement("button");
        deadlineButton.type = "button";
        deadlineButton.className = `task-deadline is-${deadline.state}`;
        deadlineButton.dataset.taskId = task.id;
        deadlineButton.dataset.deadlineOn = task.deadlineOn;
        deadlineButton.textContent = deadline.label;
        deadlineButton.title = deadline.title;
        deadlineButton.setAttribute("aria-label", `修改${deadline.title}`);
        main.append(deadlineButton);
      }
      remove.type = "button";
      remove.className = "delete-task-button";
      remove.dataset.action = "delete-task";
      remove.dataset.taskId = task.id;
      remove.dataset.taskTitle = task.title;
      remove.dataset.hasSubtasks = String(group.totalCount > 0);
      remove.textContent = "×";
      remove.title = "删除";
      remove.setAttribute("aria-label", `删除：${task.title}`);
      handle.type = "button";
      handle.className = "drag-handle";
      handle.dataset.taskId = task.id;
      handle.hidden = searching;
      handle.disabled = searching;
      handle.draggable = !searching;
      handle.setAttribute("aria-label", `调整顺序：${task.title}`);
      handle.setAttribute("aria-describedby", "taskOrderHelp");
      handle.setAttribute("aria-keyshortcuts", "Alt+ArrowUp Alt+ArrowDown");
      handleMark.setAttribute("aria-hidden", "true");
      handleMark.textContent = "⠿";
      handle.append(handleMark);
      subtaskList.id = `subtasks-${task.id}`;
      subtaskList.className = "subtask-list";
      subtaskList.hidden = true;
      subtaskList.dataset.parentTaskId = task.id;
      subtaskList.dataset.searchExpanded = String(Boolean(group.searchExpanded));
      subtaskList.setAttribute("aria-label", `${task.title}的子代办`);
      const visibleSubtasks = group.matchingSubtasks ?? group.subtasks;
      visibleSubtasks.forEach((subtask) => {
        subtaskList.append(this.#createSubtaskRow(subtask, task, query, searching));
      });
      if (!searching) {
        const addRow = this.document.createElement("li");
        const addButton = this.document.createElement("button");
        addRow.className = "subtask-add-row";
        addButton.type = "button";
        addButton.className = "subtask-add-button";
        addButton.dataset.parentTaskId = task.id;
        addButton.textContent = "＋ 添加子代办";
        addButton.setAttribute("aria-label", `为${task.title}添加子代办`);
        addRow.append(addButton);
        subtaskList.append(addRow);
      }
      item.append(checkbox, main, remove, handle, subtaskList);
      this.taskList.append(item);
    });
  }

  #createSubtaskRow(subtask, parentTask, query, searching) {
    const item = this.document.createElement("li");
    const checkbox = this.document.createElement("input");
    const title = this.document.createElement("button");
    const remove = this.document.createElement("button");
    const handle = this.document.createElement("button");
    const handleMark = this.document.createElement("span");
    const completed = subtask.status === "completed";
    item.className = `subtask-row${completed ? " is-completed" : ""}`;
    item.dataset.taskId = subtask.id;
    item.dataset.parentTaskId = parentTask.id;
    item.dataset.status = subtask.status;
    checkbox.type = "checkbox";
    checkbox.className = "task-checkbox subtask-checkbox";
    checkbox.checked = completed;
    checkbox.dataset.taskId = subtask.id;
    checkbox.dataset.parentTaskId = parentTask.id;
    checkbox.dataset.isSubtask = "true";
    if (completed) {
      checkbox.dataset.action = "undo-completion";
      checkbox.dataset.eventId = subtask.activeCompletionEventId ?? "";
      checkbox.setAttribute("aria-label", `撤销完成：${subtask.title}`);
    } else {
      checkbox.dataset.action = "complete-task";
      checkbox.setAttribute("aria-label", `完成：${subtask.title}`);
    }
    title.type = "button";
    title.className = "subtask-title";
    title.dataset.taskId = subtask.id;
    title.dataset.parentTaskId = parentTask.id;
    title.dataset.title = subtask.title;
    appendHighlightedText(this.document, title, subtask.title, query);
    title.title = subtask.title;
    title.setAttribute("aria-label", completed
      ? `已完成子代办：${subtask.title}`
      : `修改子代办：${subtask.title}`);
    title.disabled = completed;
    remove.type = "button";
    remove.className = "delete-subtask-button";
    remove.dataset.action = "delete-task";
    remove.dataset.taskId = subtask.id;
    remove.dataset.parentTaskId = parentTask.id;
    remove.dataset.isSubtask = "true";
    remove.textContent = "×";
    remove.title = completed ? "已完成子代办需先撤销" : "删除";
    remove.setAttribute("aria-label", `删除子代办：${subtask.title}`);
    remove.hidden = completed;
    handle.type = "button";
    handle.className = "subtask-handle";
    handle.dataset.taskId = subtask.id;
    handle.dataset.parentTaskId = parentTask.id;
    handle.hidden = searching;
    handle.disabled = searching;
    handle.draggable = !searching;
    handle.setAttribute("aria-label", `调整子代办顺序：${subtask.title}`);
    handle.setAttribute("aria-describedby", "taskOrderHelp");
    handle.setAttribute("aria-keyshortcuts", "Alt+ArrowUp Alt+ArrowDown");
    handleMark.setAttribute("aria-hidden", "true");
    handleMark.textContent = "⠿";
    handle.append(handleMark);
    item.append(checkbox, title, remove, handle);
    return item;
  }

  /** @param {Object} snapshot */
  #renderHistory(snapshot) {
    const groups = completionGroups(snapshot);
    const searching = this.searchState.panel === "history";
    const query = searching ? this.searchState.query : "";
    const normalizedQuery = normalizeSearchQuery(query);
    const visibleGroups = searching ? filterCompletionGroupsByTitle(groups, query) : groups;
    this.historyCount.textContent = String(groups.length);
    this.historyLink.hidden = groups.length === 0;
    this.historyList.replaceChildren();
    if (groups.length === 0) {
      this.#renderListStatus(this.historyList, "还没有完成记录");
      if (searching) this.#announceSearchResult("还没有完成记录");
      return;
    }
    if (normalizedQuery && visibleGroups.length === 0) {
      this.#renderListStatus(this.historyList, "没有找到相关完成记录");
      this.#announceSearchResult("没有找到相关完成记录");
      return;
    }
    if (searching) {
      this.#announceSearchResult(
        normalizedQuery
          ? `找到 ${visibleGroups.length} 组完成记录`
          : `共 ${visibleGroups.length} 组完成记录`,
      );
    }
    visibleGroups.forEach((group) => {
      if (group.totalCount > 0) {
        this.historyList.append(this.#createHistoryGroup(group, query));
        return;
      }
      const event = group.parentCompletion;
      if (!event) return;
      const item = this.document.createElement("li");
      const copy = this.document.createElement("span");
      const title = this.document.createElement("b");
      const time = this.document.createElement("time");
      const undo = this.document.createElement("button");
      copy.className = "history-copy";
      appendHighlightedText(this.document, title, event.titleSnapshot, query);
      title.title = event.titleSnapshot;
      time.textContent = formatTime(event.occurredAtMs);
      undo.type = "button";
      undo.dataset.action = "undo-completion";
      undo.dataset.eventId = event.id;
      undo.textContent = "↶";
      undo.title = "撤销完成";
      undo.setAttribute("aria-label", `撤销完成：${event.titleSnapshot}`);
      copy.append(title, time);
      item.append(copy, undo);
      this.historyList.append(item);
    });
  }

  #createHistoryGroup(group, query) {
    const item = this.document.createElement("li");
    const summary = this.document.createElement("div");
    const disclosure = this.document.createElement("button");
    const copy = this.document.createElement("span");
    const title = this.document.createElement("b");
    const meta = this.document.createElement("span");
    const undo = this.document.createElement("button");
    const children = this.document.createElement("ol");
    const searchExpanded = Boolean(group.searchExpanded);
    const expanded = searchExpanded || this.expandedHistoryGroupIds.has(group.parentTaskId);
    item.className = "history-group";
    item.dataset.parentTaskId = group.parentTaskId;
    summary.className = "history-group-summary";
    disclosure.type = "button";
    disclosure.className = "history-disclosure";
    disclosure.dataset.action = "toggle-history-group";
    disclosure.dataset.parentTaskId = group.parentTaskId;
    disclosure.setAttribute("aria-expanded", String(expanded));
    disclosure.setAttribute("aria-controls", `history-subtasks-${group.parentTaskId}`);
    disclosure.setAttribute("aria-label", `${expanded ? "收起" : "展开"}${group.title}的子代办完成记录`);
    copy.className = "history-copy history-group-copy";
    appendHighlightedText(this.document, title, group.title, query);
    title.title = group.title;
    meta.className = "history-group-meta";
    const progress = `${group.completedCount}/${group.totalCount}`;
    if (group.parentCompletion) {
      meta.textContent = `${progress} · ${formatTime(group.parentCompletion.occurredAtMs)}`;
    } else {
      meta.textContent = `${group.activeParent ? "进行中" : "已移除"} · ${progress}`;
    }
    copy.append(title, meta);
    summary.append(disclosure, copy);
    if (group.parentCompletion) {
      undo.type = "button";
      undo.dataset.action = "undo-completion";
      undo.dataset.eventId = group.parentCompletion.id;
      undo.textContent = "↶";
      undo.title = "撤销完成";
      undo.setAttribute("aria-label", `撤销完成：${group.title}`);
      summary.append(undo);
    }
    children.id = `history-subtasks-${group.parentTaskId}`;
    children.className = "history-subtask-list";
    children.hidden = !expanded;
    children.setAttribute("aria-label", `${group.title}的子代办完成记录`);
    const childEvents = group.matchingSubtaskCompletions ?? group.subtaskCompletions;
    childEvents.forEach((event) => {
      const child = this.document.createElement("li");
      const childCopy = this.document.createElement("span");
      const childTitle = this.document.createElement("b");
      const time = this.document.createElement("time");
      const childUndo = this.document.createElement("button");
      child.className = "history-subtask-row";
      childCopy.className = "history-copy";
      appendHighlightedText(this.document, childTitle, event.titleSnapshot, query);
      childTitle.title = event.titleSnapshot;
      time.textContent = formatTime(event.occurredAtMs);
      childUndo.type = "button";
      childUndo.dataset.action = "undo-completion";
      childUndo.dataset.eventId = event.id;
      childUndo.dataset.taskId = event.taskId;
      childUndo.dataset.parentTaskId = group.parentTaskId;
      childUndo.dataset.isSubtask = "true";
      childUndo.dataset.blocked = String(Boolean(group.parentCompletion) || !group.activeParent);
      childUndo.textContent = "↶";
      childUndo.title = group.parentCompletion
        ? "请先撤销父代办"
        : group.activeParent ? "撤销完成" : "任务组已移除";
      childUndo.setAttribute("aria-label", `${childUndo.title}：${event.titleSnapshot}`);
      childUndo.disabled = childUndo.dataset.blocked === "true";
      childCopy.append(childTitle, time);
      child.append(childCopy, childUndo);
      children.append(child);
    });
    item.append(summary, children);
    return item;
  }

  #renderListStatus(list, message) {
    const item = this.document.createElement("li");
    item.className = "empty-row";
    item.textContent = message;
    list.replaceChildren(item);
  }

  #renderCapsuleDeadline(deadlineOn, todayOn = localDateOnly()) {
    const deadline = deadlinePresentation(deadlineOn, todayOn);
    this.capsuleTaskDeadline.hidden = !deadline;
    this.capsuleTaskDeadline.textContent = deadline?.label ?? "";
    this.capsuleTaskDeadline.title = deadline?.title ?? "";
    this.capsuleTaskDeadline.setAttribute(
      "aria-label",
      deadline ? `修改${deadline.title}` : "当前任务没有截止日期",
    );
    this.capsuleTaskDeadline.dataset.deadlineOn = deadlineOn ?? "";
    this.capsuleTaskDeadline.dataset.state = deadline?.state ?? "";
  }

  #hideCollections() {
    this.historyLink.hidden = true;
    this.taskList.replaceChildren();
    this.historyList.replaceChildren();
    this.listSearchStatus.textContent = "";
  }

  #announceSearchResult(message) {
    if (this.listSearchStatus.textContent === message) return;
    this.listSearchStatus.textContent = message;
  }

  #showStatus(message) {
    this.ledgerStatus.hidden = false;
    this.ledgerStatusText.textContent = message;
  }

  /** @param {Object} state */
  #renderControls(state) {
    const unavailable = state.phase !== LedgerPhase.READY || Boolean(state.pendingOperation);
    const reorderUnavailable = unavailable || this.searchState.panel === "tasks";
    this.taskTitleInput.disabled = unavailable;
    this.captureButton.disabled = unavailable;
    this.copyWeeklyCompletionsButton.disabled = unavailable;
    this.capsuleTaskCheckbox.checked = false;
    this.capsuleTaskCheckbox.disabled = unavailable || !state.snapshot.currentTask;
    this.capsuleTaskDeadline.disabled = unavailable || this.capsuleTaskDeadline.hidden;
    this.taskList.setAttribute("aria-busy", String(unavailable));
    this.taskList.querySelectorAll(".task-checkbox").forEach((checkbox) => {
      if (!checkbox.classList.contains("subtask-checkbox")) checkbox.checked = false;
      checkbox.disabled = unavailable;
    });
    this.taskList.querySelectorAll(".drag-handle").forEach((handle) => {
      handle.disabled = reorderUnavailable;
      handle.draggable = !reorderUnavailable;
    });
    this.taskList.querySelectorAll(".delete-task-button").forEach((button) => {
      button.disabled = unavailable;
    });
    this.taskList.querySelectorAll(".delete-subtask-button").forEach((button) => {
      button.disabled = unavailable || button.hidden;
    });
    this.taskList.querySelectorAll(".task-title").forEach((button) => {
      button.disabled = unavailable;
    });
    this.taskList.querySelectorAll(".task-deadline").forEach((button) => {
      button.disabled = unavailable;
    });
    this.taskList.querySelectorAll(".subtask-title").forEach((button) => {
      button.disabled = unavailable || button.closest(".subtask-row")?.dataset.status === "completed";
    });
    this.taskList.querySelectorAll(".subtask-add-trigger, .subtask-add-button").forEach((button) => {
      button.disabled = unavailable || this.searchState.panel === "tasks";
    });
    this.taskList.querySelectorAll(".subtask-handle").forEach((handle) => {
      handle.disabled = reorderUnavailable;
      handle.draggable = !reorderUnavailable;
    });
    this.retryButton.disabled = state.phase !== LedgerPhase.ERROR;
    this.historyList
      .querySelectorAll("[data-action='undo-completion']")
      .forEach((button) => {
        button.disabled = unavailable || button.dataset.blocked === "true";
      });
  }
}

export class ToastView {
  /** @param {HTMLElement} element @param {Window} targetWindow */
  constructor(element, targetWindow) {
    this.element = element;
    this.targetWindow = targetWindow;
    this.timer = 0;
  }

  /** @param {string} message */
  show(message) {
    this.element.textContent = message;
    this.element.classList.add("show");
    this.targetWindow.clearTimeout(this.timer);
    this.timer = this.targetWindow.setTimeout(
      () => this.element.classList.remove("show"),
      2600,
    );
  }
}

function required(parent, selector) {
  const element = parent.querySelector(selector);
  if (!element) throw new Error(`界面缺少必要元素：${selector}`);
  return element;
}

function appendHighlightedText(document, element, title, query) {
  const text = String(title ?? "");
  const ranges = titleMatchRanges(text, query);
  if (ranges.length === 0) {
    element.textContent = text;
    return;
  }
  let offset = 0;
  ranges.forEach(([start, end]) => {
    if (start > offset) element.append(document.createTextNode(text.slice(offset, start)));
    const mark = document.createElement("mark");
    mark.className = "search-match";
    mark.textContent = text.slice(start, end);
    element.append(mark);
    offset = end;
  });
  if (offset < text.length) element.append(document.createTextNode(text.slice(offset)));
}
