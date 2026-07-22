import { LedgerPhase } from "./state.js";
import {
  activeCompletionEvents,
  filterCompletionEventsByTitle,
  filterTasksByTitle,
  formatTime,
  ledgerContentKind,
  normalizeSearchQuery,
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
    if (!this.searchState.panel) this.listSearchStatus.textContent = "";
    this.ledgerStatus.hidden = true;
    this.retryButton.hidden = true;
    this.capsuleTaskTitle.textContent = current?.title ?? "暂无待办";
    this.capsuleTaskTitle.title = current?.title ?? "";
    this.capsuleTitleButton.setAttribute(
      "aria-label",
      current ? `当前待办：${current.title}，展开任务面板` : "暂无待办，展开任务面板",
    );
    this.capsuleTaskDeadline.dataset.taskId = current?.id ?? "";
    this.#renderCapsuleDeadline(current?.deadlineOn ?? null);
    this.capsuleTaskCheckbox.dataset.taskId = current?.id ?? "";
    this.capsuleTaskCheckbox.hidden = !current;
    this.capsuleTaskCheckbox.setAttribute(
      "aria-label",
      current ? `完成：${current.title}` : "当前没有待办",
    );
    this.#renderTasks(snapshot.queue, current?.id ?? null);
    this.#renderHistory(snapshot);
  }

  #renderLoading() {
    this.#showStatus("正在读取本地账本…");
    this.retryButton.hidden = true;
    this.capsuleTaskTitle.textContent = "本地账本加载中";
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
    const visibleTasks = searching ? filterTasksByTitle(tasks, query) : tasks;
    this.taskList.replaceChildren();
    if (tasks.length === 0) {
      this.#renderListStatus(this.taskList, "还没有待办");
      if (searching) this.#announceSearchResult("还没有待办");
      return;
    }
    if (normalizedQuery && visibleTasks.length === 0) {
      this.#renderListStatus(this.taskList, "没有找到相关待办");
      this.#announceSearchResult("没有找到相关待办");
      return;
    }
    if (searching) {
      this.#announceSearchResult(
        normalizedQuery ? `找到 ${visibleTasks.length} 项待办` : `共 ${visibleTasks.length} 项待办`,
      );
    }
    visibleTasks.forEach((task) => {
      const item = this.document.createElement("li");
      const checkbox = this.document.createElement("input");
      const main = this.document.createElement("div");
      const title = this.document.createElement("button");
      const remove = this.document.createElement("button");
      const handle = this.document.createElement("button");
      const handleMark = this.document.createElement("span");
      item.className = "task-row";
      item.dataset.taskId = task.id;
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
      appendHighlightedText(this.document, title, task.title, query);
      title.title = task.title;
      title.setAttribute("aria-label", `修改待办：${task.title}`);
      main.append(title);
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
      item.append(checkbox, main, remove, handle);
      this.taskList.append(item);
    });
  }

  /** @param {Object} snapshot */
  #renderHistory(snapshot) {
    const completions = activeCompletionEvents(snapshot);
    const searching = this.searchState.panel === "history";
    const query = searching ? this.searchState.query : "";
    const normalizedQuery = normalizeSearchQuery(query);
    const visibleCompletions = searching
      ? filterCompletionEventsByTitle(completions, query)
      : completions;
    this.historyCount.textContent = String(completions.length);
    this.historyLink.hidden = completions.length === 0;
    this.historyList.replaceChildren();
    if (completions.length === 0) {
      this.#renderListStatus(this.historyList, "还没有完成记录");
      if (searching) this.#announceSearchResult("还没有完成记录");
      return;
    }
    if (normalizedQuery && visibleCompletions.length === 0) {
      this.#renderListStatus(this.historyList, "没有找到相关完成记录");
      this.#announceSearchResult("没有找到相关完成记录");
      return;
    }
    if (searching) {
      this.#announceSearchResult(
        normalizedQuery
          ? `找到 ${visibleCompletions.length} 项完成记录`
          : `共 ${visibleCompletions.length} 项完成记录`,
      );
    }
    visibleCompletions.forEach((event) => {
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
      checkbox.checked = false;
      checkbox.disabled = unavailable;
    });
    this.taskList.querySelectorAll(".drag-handle").forEach((handle) => {
      handle.disabled = reorderUnavailable;
      handle.draggable = !reorderUnavailable;
    });
    this.taskList.querySelectorAll(".delete-task-button").forEach((button) => {
      button.disabled = unavailable;
    });
    this.taskList.querySelectorAll(".task-title").forEach((button) => {
      button.disabled = unavailable;
    });
    this.taskList.querySelectorAll(".task-deadline").forEach((button) => {
      button.disabled = unavailable;
    });
    this.retryButton.disabled = state.phase !== LedgerPhase.ERROR;
    this.historyList
      .querySelectorAll("[data-action='undo-completion']")
      .forEach((button) => {
        button.disabled = unavailable;
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
