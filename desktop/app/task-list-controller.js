import { assertDeadlineOn, MAX_TASK_TITLE_LENGTH } from "./ledger-contract.js";

/** 统一待办列表交互：只提交顺序、标题与截止日期意图，不接触账本或 IPC。 */
export class TaskListController {
  /**
   * @param {{
   *   list: HTMLOListElement,
   *   status: HTMLElement,
   *   onReorder: (movedTaskId: string, expectedTaskIds: string[], orderedTaskIds: string[]) => Promise<unknown>,
   *   onUpdateDeadline: (taskId: string, deadlineOn: string|null) => Promise<unknown>,
   *   onUpdateTitle: (taskId: string, title: string) => Promise<unknown>,
   *   onError: (error: unknown) => void,
   *   canReorder?: () => boolean,
   *   isSearchActive?: () => boolean,
   *   focusSearchFallback?: () => boolean,
   * }} dependencies
   */
  constructor({
    list,
    status,
    onReorder,
    onUpdateDeadline,
    onUpdateTitle,
    onError,
    canReorder = () => true,
    isSearchActive = () => false,
    focusSearchFallback = () => false,
  }) {
    this.list = list;
    this.status = status;
    this.onReorder = onReorder;
    this.onUpdateDeadline = onUpdateDeadline;
    this.onUpdateTitle = onUpdateTitle;
    this.onError = onError;
    this.canReorder = canReorder;
    this.isSearchActive = isSearchActive;
    this.focusSearchFallback = focusSearchFallback;
    this.dragState = null;
    this.editState = null;
    this.pendingFocus = null;

    list.addEventListener("dblclick", (event) => this.#handleTitleDoubleClick(event));
    list.addEventListener("click", (event) => this.#handleTaskClick(event));
    list.addEventListener("change", (event) => this.#handleDeadlineChange(event));
    list.addEventListener("focusout", (event) => this.#handleEditFocusOut(event));
    list.addEventListener("dragstart", (event) => this.#startDrag(event));
    list.addEventListener("dragover", (event) => this.#previewDrag(event));
    list.addEventListener("drop", (event) => this.#drop(event));
    list.addEventListener("dragend", () => this.#endDrag());
    list.addEventListener("keydown", (event) => this.#handleKeydown(event));
  }

  /** 完成一项前记住最自然的后续焦点。 */
  rememberCompletionFocus(taskId) {
    this.rememberRemovalFocus(taskId);
  }

  /** 一项即将移出列表时记住最自然的后续焦点。 */
  rememberRemovalFocus(taskId) {
    const taskIds = this.#taskIds();
    const index = taskIds.indexOf(taskId);
    if (index < 0) return;
    const nextTaskId = taskIds[index + 1] ?? taskIds[index - 1];
    this.pendingFocus = nextTaskId
      ? { taskId: nextTaskId, selector: ".task-checkbox" }
      : this.isSearchActive()
        ? { taskId, selector: ".task-checkbox" }
        : null;
  }

  /** 视图以真实快照重绘后恢复键盘焦点。 */
  restorePendingFocus() {
    if (!this.pendingFocus) return;
    const { taskId, selector } = this.pendingFocus;
    const row = [...this.list.querySelectorAll(".task-row")]
      .find((item) => item.dataset.taskId === taskId);
    const control = row?.querySelector(selector);
    if (!control) {
      const shouldFocusSearch = this.isSearchActive();
      this.pendingFocus = null;
      if (shouldFocusSearch) this.focusSearchFallback();
      return;
    }
    if (control.disabled) return;
    control.focus();
    this.pendingFocus = null;
  }

  /** 从胶囊期限标签展开后，直接进入对应任务的日期编辑。 */
  beginDeadlineEdit(taskId) {
    const row = [...this.list.querySelectorAll(".task-row")]
      .find((item) => item.dataset.taskId === taskId);
    const trigger = row?.querySelector(".task-title");
    if (!trigger || trigger.disabled) return false;
    return this.#beginTaskEdit(trigger, { focusDeadline: true });
  }

  #handleTitleDoubleClick(event) {
    const trigger = event.target.closest?.(".task-title");
    if (!trigger || trigger.disabled) return;
    event.preventDefault();
    this.#beginTaskEdit(trigger);
  }

  #handleTaskClick(event) {
    const clearButton = event.target.closest?.(".task-deadline-clear");
    if (clearButton && clearButton === this.editState?.clearButton) {
      event.preventDefault();
      event.stopPropagation();
      this.editState.deadlineInput.value = "";
      this.#finishTaskEdit({ keepInvalidEditor: true, restoreFocus: true });
      return;
    }

    const deadlineButton = event.target.closest?.(".task-deadline");
    if (!deadlineButton || deadlineButton.disabled) return;
    const trigger = deadlineButton.closest(".task-row")?.querySelector(".task-title");
    if (!trigger || trigger.disabled) return;
    event.preventDefault();
    event.stopPropagation();
    this.#beginTaskEdit(trigger, { focusDeadline: true });
  }

  #handleDeadlineChange(event) {
    if (event.target !== this.editState?.deadlineInput) return;
    this.#finishTaskEdit({ keepInvalidEditor: true, restoreFocus: true });
  }

  #handleEditFocusOut(event) {
    const state = this.editState;
    if (!state || !state.stack.contains(event.target)) return;
    if (event.relatedTarget && state.stack.contains(event.relatedTarget)) return;
    this.#finishTaskEdit({ keepInvalidEditor: false, restoreFocus: false });
  }

  #handleKeydown(event) {
    const editStack = event.target.closest?.(".task-edit-stack");
    if (editStack) {
      if (event.isComposing || event.keyCode === 229) return;
      if (event.target.closest?.(".task-edit-add-subtask")) return;
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        this.#cancelTaskEdit(true);
        return;
      }
      if (event.key === "Enter" && !event.target.closest?.(".task-deadline-clear")) {
        event.preventDefault();
        event.stopPropagation();
        this.#finishTaskEdit({ keepInvalidEditor: true, restoreFocus: true });
      }
      return;
    }

    const trigger = event.target.closest?.(".task-title");
    if (
      trigger
      && !trigger.disabled
      && !event.repeat
      && (event.key === "Enter" || event.key === "F2")
    ) {
      event.preventDefault();
      event.stopPropagation();
      this.#beginTaskEdit(trigger);
      return;
    }

    this.#handleKeyboardMove(event);
  }

  #beginTaskEdit(trigger, { focusDeadline = false } = {}) {
    if (this.editState || this.list.getAttribute("aria-busy") === "true") return false;
    const taskId = trigger.dataset.taskId;
    const originalTitle = trigger.textContent ?? "";
    const originalDeadlineOn = trigger.dataset.deadlineOn || null;
    const row = trigger.closest(".task-row");
    if (!taskId || !originalTitle || !row) return false;

    const stack = this.list.ownerDocument.createElement("div");
    const input = this.list.ownerDocument.createElement("input");
    const deadlineRow = this.list.ownerDocument.createElement("div");
    const deadlineLabel = this.list.ownerDocument.createElement("label");
    const deadlineInput = this.list.ownerDocument.createElement("input");
    const clearButton = this.list.ownerDocument.createElement("button");
    const addSubtaskButton = this.list.ownerDocument.createElement("button");
    stack.className = "task-edit-stack";
    input.type = "text";
    input.className = "task-title-editor";
    input.dataset.taskId = taskId;
    input.value = originalTitle;
    input.maxLength = MAX_TASK_TITLE_LENGTH;
    input.autocomplete = "off";
    input.setAttribute("aria-label", `修改待办：${originalTitle}`);
    input.addEventListener("input", () => input.removeAttribute("aria-invalid"));
    deadlineRow.className = "task-deadline-edit-row";
    deadlineLabel.textContent = "截止日期";
    deadlineInput.type = "date";
    deadlineInput.className = "task-deadline-editor";
    deadlineInput.id = "active-task-deadline-editor";
    deadlineInput.dataset.taskId = taskId;
    deadlineInput.value = originalDeadlineOn ?? "";
    deadlineInput.setAttribute("aria-label", `截止日期：${originalTitle}`);
    deadlineLabel.htmlFor = deadlineInput.id;
    deadlineInput.addEventListener("input", () => deadlineInput.removeAttribute("aria-invalid"));
    clearButton.type = "button";
    clearButton.className = "task-deadline-clear";
    clearButton.textContent = "清除";
    clearButton.hidden = !originalDeadlineOn;
    clearButton.setAttribute("aria-label", `清除截止日期：${originalTitle}`);
    deadlineRow.append(deadlineLabel, deadlineInput, clearButton);
    addSubtaskButton.type = "button";
    addSubtaskButton.className = "task-edit-add-subtask";
    addSubtaskButton.dataset.parentTaskId = taskId;
    addSubtaskButton.textContent = "＋ 添加子代办";
    addSubtaskButton.setAttribute("aria-label", `为${originalTitle}添加子代办`);
    stack.append(input, deadlineRow, addSubtaskButton);

    this.editState = {
      taskId,
      originalTitle,
      originalDeadlineOn,
      input,
      deadlineInput,
      clearButton,
      stack,
      row,
      trigger,
    };
    row.classList.add("is-editing");
    trigger.replaceWith(stack);
    if (focusDeadline) {
      deadlineInput.focus();
    } else {
      input.focus();
      input.select();
    }
    return true;
  }

  #finishTaskEdit({ keepInvalidEditor, restoreFocus }) {
    const state = this.editState;
    if (!state) return;
    const title = state.input.value.trim();
    if (!title || [...title].length > MAX_TASK_TITLE_LENGTH) {
      if (keepInvalidEditor) {
        state.input.setAttribute("aria-invalid", "true");
        state.input.focus();
      } else {
        this.#cancelTaskEdit(restoreFocus);
      }
      return;
    }

    const deadlineOn = state.deadlineInput.value || null;
    try {
      assertDeadlineOn(deadlineOn);
    } catch (error) {
      if (keepInvalidEditor) {
        state.deadlineInput.setAttribute("aria-invalid", "true");
        state.deadlineInput.focus();
      } else {
        this.#cancelTaskEdit(restoreFocus);
      }
      return;
    }

    const titleChanged = title !== state.originalTitle;
    const deadlineChanged = deadlineOn !== state.originalDeadlineOn;
    if (!titleChanged && !deadlineChanged) {
      this.#cancelTaskEdit(restoreFocus);
      return;
    }

    // 两个字段仍走各自显式命令；仅当两者都变化时按标题、期限顺序串行确认。
    this.#leaveTaskEdit(state);
    Promise.resolve()
      .then(async () => {
        if (titleChanged) await this.onUpdateTitle(state.taskId, title);
        if (deadlineChanged) await this.onUpdateDeadline(state.taskId, deadlineOn);
      })
      .catch((error) => this.onError(error))
      .finally(() => {
        if (restoreFocus) {
          this.pendingFocus = { taskId: state.taskId, selector: ".task-title" };
        }
        this.restorePendingFocus();
      });
  }

  #cancelTaskEdit(requestFocus) {
    const state = this.editState;
    if (!state) return;
    this.#leaveTaskEdit(state);
    if (requestFocus && !state.trigger.disabled) state.trigger.focus();
  }

  #leaveTaskEdit(state) {
    this.editState = null;
    state.stack.replaceWith(state.trigger);
    state.row.classList.remove("is-editing");
  }

  #startDrag(event) {
    const handle = event.target.closest?.(".drag-handle");
    if (!handle) return;
    if (!this.canReorder() || handle.disabled || handle.draggable === false) {
      event.preventDefault();
      return;
    }
    const row = handle.closest(".task-row");
    const movedTaskId = row?.dataset.taskId;
    if (!row || !movedTaskId) {
      event.preventDefault();
      return;
    }
    this.dragState = {
      movedTaskId,
      expectedTaskIds: this.#taskIds(),
      row,
      dropped: false,
    };
    row.classList.add("is-dragging");
    if (event.dataTransfer) {
      event.dataTransfer.effectAllowed = "move";
      event.dataTransfer.setData("text/plain", movedTaskId);
    }
  }

  #previewDrag(event) {
    const state = this.dragState;
    if (!state) return;
    if (!this.canReorder()) {
      this.#clearDraggingState();
      return;
    }
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
    const targetRow = event.target.closest?.(".task-row");
    if (!targetRow) {
      if (event.target === this.list) this.list.append(state.row);
      return;
    }
    if (targetRow === state.row) return;
    const bounds = targetRow.getBoundingClientRect();
    const before = event.clientY < bounds.top + bounds.height / 2;
    this.list.insertBefore(
      state.row,
      before ? targetRow : targetRow.nextElementSibling,
    );
  }

  #drop(event) {
    const state = this.dragState;
    if (!state) return;
    event.preventDefault();
    if (!this.canReorder()) {
      this.#clearDraggingState();
      return;
    }
    state.dropped = true;
    const orderedTaskIds = this.#taskIds();
    this.#clearDraggingState();
    if (sameOrder(state.expectedTaskIds, orderedTaskIds)) return;
    this.pendingFocus = {
      taskId: state.movedTaskId,
      selector: ".drag-handle",
    };
    this.#submitReorder(
      state.movedTaskId,
      state.expectedTaskIds,
      orderedTaskIds,
    );
  }

  #endDrag() {
    const state = this.dragState;
    if (!state) return;
    if (!state.dropped) this.#restoreOrder(state.expectedTaskIds);
    this.#clearDraggingState();
  }

  #handleKeyboardMove(event) {
    const handle = event.target.closest?.(".drag-handle");
    if (!this.canReorder() || !handle || handle.disabled || !event.altKey) return;
    const offset = event.key === "ArrowUp" ? -1 : event.key === "ArrowDown" ? 1 : 0;
    if (offset === 0) return;
    event.preventDefault();
    const movedTaskId = handle.dataset.taskId;
    const expectedTaskIds = this.#taskIds();
    const orderedTaskIds = moveTaskByOffset(expectedTaskIds, movedTaskId, offset);
    if (sameOrder(expectedTaskIds, orderedTaskIds)) {
      this.#announce(offset < 0 ? "已经是第一项" : "已经是最后一项");
      return;
    }
    this.pendingFocus = { taskId: movedTaskId, selector: ".drag-handle" };
    this.#announce(offset < 0 ? "已上移一项" : "已下移一项");
    this.#submitReorder(movedTaskId, expectedTaskIds, orderedTaskIds);
  }

  #submitReorder(movedTaskId, expectedTaskIds, orderedTaskIds) {
    if (!this.canReorder()) return;
    Promise.resolve(
      this.onReorder(movedTaskId, expectedTaskIds, orderedTaskIds),
    ).catch((error) => {
      this.#restoreOrder(expectedTaskIds);
      this.onError(error);
    });
  }

  #taskIds() {
    return [...this.list.querySelectorAll(".task-row")]
      .map((item) => item.dataset.taskId)
      .filter(Boolean);
  }

  #restoreOrder(taskIds) {
    const rows = new Map(
      [...this.list.querySelectorAll(".task-row")]
        .map((row) => [row.dataset.taskId, row]),
    );
    taskIds.forEach((taskId) => {
      const row = rows.get(taskId);
      if (row) this.list.append(row);
    });
  }

  #clearDraggingState() {
    this.dragState?.row.classList.remove("is-dragging");
    this.dragState = null;
  }

  #announce(message) {
    this.status.textContent = "";
    queueMicrotask(() => {
      this.status.textContent = message;
    });
  }
}

export function moveTaskByOffset(taskIds, movedTaskId, offset) {
  const ordered = [...taskIds];
  const currentIndex = ordered.indexOf(movedTaskId);
  if (currentIndex < 0) return ordered;
  const nextIndex = Math.max(0, Math.min(ordered.length - 1, currentIndex + offset));
  if (nextIndex === currentIndex) return ordered;
  ordered.splice(currentIndex, 1);
  ordered.splice(nextIndex, 0, movedTaskId);
  return ordered;
}

function sameOrder(left, right) {
  return left.length === right.length
    && left.every((taskId, index) => taskId === right[index]);
}
