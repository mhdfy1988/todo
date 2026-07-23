import { MAX_TASK_TITLE_LENGTH, normalizeTaskTitle } from "./ledger-contract.js";
import { moveTaskByOffset } from "./task-list-controller.js";

/**
 * 子代办交互控制器：只维护展开、单次添加、编辑、同组排序与焦点等临时界面状态。
 * 所有业务变化均通过组合根注入的意图回调提交给 LedgerSession。
 */
export class SubtaskController {
  constructor({
    list,
    status,
    deleteDialog = null,
    deleteDialogTitle = null,
    onCreate,
    onUpdateTitle,
    onReorder,
    onError,
    canMutate = () => true,
    isSearchActive = () => false,
    focusSearchFallback = () => false,
  }) {
    this.list = list;
    this.status = status;
    this.deleteDialog = deleteDialog;
    this.deleteDialogTitle = deleteDialogTitle;
    this.onCreate = onCreate;
    this.onUpdateTitle = onUpdateTitle;
    this.onReorder = onReorder;
    this.onError = onError;
    this.canMutate = canMutate;
    this.isSearchActive = isSearchActive;
    this.focusSearchFallback = focusSearchFallback;
    this.expandedParentIds = new Set();
    this.addingParentId = null;
    this.addDraft = "";
    this.captureSubmitting = false;
    this.editState = null;
    this.dragState = null;
    this.pendingFocus = null;

    list.addEventListener("click", (event) => this.#handleClick(event));
    list.addEventListener("dblclick", (event) => this.#handleDoubleClick(event));
    list.addEventListener("keydown", (event) => this.#handleKeydown(event));
    list.addEventListener("focusout", (event) => this.#handleFocusOut(event));
    list.addEventListener("dragstart", (event) => this.#startDrag(event));
    list.addEventListener("dragover", (event) => this.#previewDrag(event));
    list.addEventListener("drop", (event) => this.#drop(event));
    list.addEventListener("dragend", () => this.#endDrag());
  }

  sync() {
    if (this.editState && !this.editState.input.isConnected) this.editState = null;
    this.list.querySelectorAll(".task-row").forEach((row) => {
      const parentTaskId = row.dataset.taskId;
      const subtaskList = row.querySelector(":scope > .subtask-list");
      if (!parentTaskId || !subtaskList) return;
      const searchExpanded = subtaskList.dataset.searchExpanded === "true";
      const expanded = searchExpanded
        || this.expandedParentIds.has(parentTaskId)
        || this.addingParentId === parentTaskId;
      subtaskList.hidden = !expanded;
      row.classList.toggle("has-expanded-subtasks", expanded);
      const progress = row.querySelector(":scope > .task-main .subtask-progress-button");
      if (progress) progress.setAttribute("aria-expanded", String(expanded));
      const addRow = subtaskList.querySelector(":scope > .subtask-add-row");
      if (addRow) addRow.hidden = this.addingParentId === parentTaskId || this.isSearchActive();
    });
    this.#syncCaptureEditor();
  }

  toggle(parentTaskId) {
    const row = this.#parentRow(parentTaskId);
    if (!row) return false;
    if (this.expandedParentIds.has(parentTaskId)) {
      this.expandedParentIds.delete(parentTaskId);
      if (this.addingParentId === parentTaskId) this.#stopAdding(false);
    } else {
      this.expandedParentIds.add(parentTaskId);
    }
    this.sync();
    return true;
  }

  expand(parentTaskId) {
    if (!parentTaskId) return false;
    this.expandedParentIds.add(parentTaskId);
    this.sync();
    return Boolean(this.#parentRow(parentTaskId));
  }

  revealFromCapsule(parentTaskId, taskId) {
    if (!parentTaskId || !taskId) return false;
    if (parentTaskId !== taskId) this.expand(parentTaskId);
    const row = parentTaskId === taskId
      ? this.#parentRow(parentTaskId)
      : this.#subtaskRow(parentTaskId, taskId);
    const checkbox = row?.querySelector(parentTaskId === taskId
      ? ":scope > .task-checkbox"
      : ".subtask-checkbox");
    if (!checkbox || checkbox.disabled) return false;
    checkbox.focus();
    checkbox.scrollIntoView?.({ block: "nearest" });
    return true;
  }

  startAdding(parentTaskId) {
    if (!this.canMutate() || this.isSearchActive()) return false;
    this.addingParentId = parentTaskId;
    this.addDraft = "";
    this.expandedParentIds.add(parentTaskId);
    this.sync();
    this.#captureInput()?.focus();
    return true;
  }

  rememberSubtaskFocus(parentTaskId, taskId, { remains = true } = {}) {
    const row = this.#subtaskRow(parentTaskId, taskId);
    if (!row) return;
    if (remains) {
      this.pendingFocus = { parentTaskId, taskId, selector: ".subtask-checkbox" };
      return;
    }
    const rows = [...row.parentElement.querySelectorAll(":scope > .subtask-row")];
    const index = rows.indexOf(row);
    const sibling = rows[index + 1] ?? rows[index - 1];
    this.pendingFocus = sibling
      ? { parentTaskId, taskId: sibling.dataset.taskId, selector: ".subtask-checkbox" }
      : { parentTaskId, taskId: parentTaskId, selector: ":scope > .task-checkbox" };
  }

  restorePendingFocus() {
    this.sync();
    if (!this.pendingFocus) return;
    const { parentTaskId, taskId, selector } = this.pendingFocus;
    const row = taskId === parentTaskId
      ? this.#parentRow(parentTaskId)
      : this.#subtaskRow(parentTaskId, taskId);
    const control = row?.querySelector(selector);
    if (!control) {
      const searchActive = this.isSearchActive();
      this.pendingFocus = null;
      if (searchActive) this.focusSearchFallback();
      return;
    }
    if (control.disabled) return;
    control.focus();
    this.pendingFocus = null;
  }

  confirmGroupDeletion(title) {
    if (!this.deleteDialog || typeof this.deleteDialog.showModal !== "function") {
      return Promise.resolve(false);
    }
    if (this.deleteDialogTitle) this.deleteDialogTitle.textContent = `删除“${title}”？`;
    this.deleteDialog.returnValue = "cancel";
    this.deleteDialog.showModal();
    return new Promise((resolve) => {
      this.deleteDialog.addEventListener("close", () => {
        resolve(this.deleteDialog.returnValue === "confirm");
      }, { once: true });
    });
  }

  #handleClick(event) {
    const progress = event.target.closest?.(".subtask-progress-button");
    if (progress) {
      event.preventDefault();
      event.stopPropagation();
      this.toggle(progress.dataset.parentTaskId);
      return;
    }
    const add = event.target.closest?.(".subtask-add-trigger, .subtask-add-button, .task-edit-add-subtask");
    if (add && !add.disabled) {
      event.preventDefault();
      event.stopPropagation();
      this.startAdding(add.dataset.parentTaskId);
    }
  }

  #handleDoubleClick(event) {
    const trigger = event.target.closest?.(".subtask-title");
    if (!trigger || trigger.disabled) return;
    event.preventDefault();
    this.#beginTitleEdit(trigger);
  }

  #handleKeydown(event) {
    if (event.isComposing || event.keyCode === 229) return;
    const capture = event.target.closest?.(".subtask-capture-input");
    if (capture) {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        this.#stopAdding(true);
      } else if (event.key === "Enter") {
        event.preventDefault();
        event.stopPropagation();
        void this.#submitCapture(capture, { restoreFocus: true });
      }
      return;
    }
    const editor = event.target.closest?.(".subtask-title-editor");
    if (editor) {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        this.#cancelTitleEdit(true);
      } else if (event.key === "Enter") {
        event.preventDefault();
        event.stopPropagation();
        this.#finishTitleEdit({ keepInvalidEditor: true, restoreFocus: true });
      }
      return;
    }
    const progress = event.target.closest?.(".subtask-progress-button");
    if (progress && (event.key === "ArrowRight" || event.key === "ArrowLeft")) {
      event.preventDefault();
      const shouldExpand = event.key === "ArrowRight";
      if (shouldExpand) this.expandedParentIds.add(progress.dataset.parentTaskId);
      else this.expandedParentIds.delete(progress.dataset.parentTaskId);
      this.sync();
      return;
    }
    const title = event.target.closest?.(".subtask-title");
    if (title && !title.disabled && !event.repeat && (event.key === "Enter" || event.key === "F2")) {
      event.preventDefault();
      event.stopPropagation();
      this.#beginTitleEdit(title);
      return;
    }
    this.#handleKeyboardMove(event);
  }

  #handleFocusOut(event) {
    if (event.target === this.#captureInput()) {
      this.addDraft = event.target.value;
      if (!this.addDraft.trim()) {
        this.#stopAdding(false);
        return;
      }
      if (!event.target.disabled) {
        void this.#submitCapture(event.target, { restoreFocus: false });
      }
      return;
    }
    if (event.target !== this.editState?.input) return;
    this.#finishTitleEdit({ keepInvalidEditor: false, restoreFocus: false });
  }

  #syncCaptureEditor() {
    this.list.querySelectorAll(".subtask-capture-row").forEach((row) => {
      if (row.closest(".task-row")?.dataset.taskId !== this.addingParentId) row.remove();
    });
    if (!this.addingParentId || this.isSearchActive()) return;
    const row = this.#parentRow(this.addingParentId);
    const subtaskList = row?.querySelector(":scope > .subtask-list");
    if (!subtaskList) return;
    let captureRow = subtaskList.querySelector(":scope > .subtask-capture-row");
    if (!captureRow) {
      captureRow = this.list.ownerDocument.createElement("li");
      const input = this.list.ownerDocument.createElement("input");
      captureRow.className = "subtask-capture-row";
      input.type = "text";
      input.className = "subtask-capture-input";
      input.maxLength = MAX_TASK_TITLE_LENGTH;
      input.autocomplete = "off";
      input.placeholder = "子代办名称";
      input.setAttribute("aria-label", "子代办名称");
      input.value = this.addDraft;
      input.addEventListener("input", () => {
        this.addDraft = input.value;
        input.removeAttribute("aria-invalid");
      });
      captureRow.append(input);
      subtaskList.append(captureRow);
    }
    const input = captureRow.querySelector(".subtask-capture-input");
    input.disabled = !this.canMutate();
    const addRow = subtaskList.querySelector(":scope > .subtask-add-row");
    if (addRow) addRow.hidden = true;
  }

  async #submitCapture(input, { restoreFocus = false } = {}) {
    if (!this.canMutate() || this.captureSubmitting) return;
    let title;
    try {
      title = normalizeTaskTitle(input.value);
    } catch {
      input.setAttribute("aria-invalid", "true");
      input.focus();
      return;
    }
    this.captureSubmitting = true;
    input.disabled = true;
    this.addDraft = input.value;
    try {
      const operation = await this.onCreate(this.addingParentId, title);
      if (!operation) {
        input.disabled = !this.canMutate();
        return;
      }
      this.addDraft = "";
      this.#announce("已添加子代办");
      this.#stopAdding(restoreFocus);
    } catch (error) {
      this.sync();
      const current = this.#captureInput();
      if (current) {
        current.value = this.addDraft;
        current.setAttribute("aria-invalid", "true");
        if (!current.disabled) current.focus();
      }
      this.onError(error);
    } finally {
      this.captureSubmitting = false;
    }
  }

  #stopAdding(restoreFocus) {
    const parentTaskId = this.addingParentId;
    this.addingParentId = null;
    this.addDraft = "";
    this.sync();
    if (!restoreFocus || !parentTaskId) return;
    const row = this.#parentRow(parentTaskId);
    const target = row?.querySelector(".subtask-progress-button, .subtask-add-trigger, .task-title");
    if (target && !target.disabled) target.focus();
  }

  #beginTitleEdit(trigger) {
    if (this.editState || !this.canMutate()) return false;
    const row = trigger.closest(".subtask-row");
    const taskId = row?.dataset.taskId;
    const parentTaskId = row?.dataset.parentTaskId;
    const originalTitle = trigger.dataset.title || trigger.textContent || "";
    if (!row || !taskId || !parentTaskId || !originalTitle) return false;
    const input = this.list.ownerDocument.createElement("input");
    input.type = "text";
    input.className = "subtask-title-editor";
    input.value = originalTitle;
    input.maxLength = MAX_TASK_TITLE_LENGTH;
    input.autocomplete = "off";
    input.setAttribute("aria-label", `修改子代办：${originalTitle}`);
    input.addEventListener("input", () => input.removeAttribute("aria-invalid"));
    this.editState = { row, taskId, parentTaskId, originalTitle, trigger, input };
    row.classList.add("is-editing");
    trigger.replaceWith(input);
    input.focus();
    input.select();
    return true;
  }

  #finishTitleEdit({ keepInvalidEditor, restoreFocus }) {
    const state = this.editState;
    if (!state) return;
    let title;
    try {
      title = normalizeTaskTitle(state.input.value);
    } catch {
      if (keepInvalidEditor) {
        state.input.setAttribute("aria-invalid", "true");
        state.input.focus();
      } else {
        this.#cancelTitleEdit(restoreFocus);
      }
      return;
    }
    if (title === state.originalTitle) {
      this.#cancelTitleEdit(restoreFocus);
      return;
    }
    this.#leaveTitleEdit(state);
    if (restoreFocus) {
      this.pendingFocus = {
        parentTaskId: state.parentTaskId,
        taskId: state.taskId,
        selector: ".subtask-title",
      };
    }
    Promise.resolve(this.onUpdateTitle(state.taskId, title))
      .catch((error) => this.onError(error))
      .finally(() => {
        if (restoreFocus) this.restorePendingFocus();
      });
  }

  #cancelTitleEdit(restoreFocus) {
    const state = this.editState;
    if (!state) return;
    this.#leaveTitleEdit(state);
    if (restoreFocus && !state.trigger.disabled) state.trigger.focus();
  }

  #leaveTitleEdit(state) {
    this.editState = null;
    state.input.replaceWith(state.trigger);
    state.row.classList.remove("is-editing");
  }

  #startDrag(event) {
    const handle = event.target.closest?.(".subtask-handle");
    if (!handle || handle.disabled || !this.canMutate() || this.isSearchActive()) {
      if (handle) event.preventDefault();
      return;
    }
    const row = handle.closest(".subtask-row");
    const list = row?.parentElement;
    const parentTaskId = row?.dataset.parentTaskId;
    const movedTaskId = row?.dataset.taskId;
    if (!row || !list || !parentTaskId || !movedTaskId) {
      event.preventDefault();
      return;
    }
    this.dragState = {
      parentTaskId,
      movedTaskId,
      expectedTaskIds: subtaskIds(list),
      row,
      list,
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
    if (!state || this.isSearchActive() || !this.canMutate()) return;
    const targetRow = event.target.closest?.(".subtask-row");
    if (!targetRow || targetRow.parentElement !== state.list) return;
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
    if (targetRow === state.row) return;
    const bounds = targetRow.getBoundingClientRect();
    const before = event.clientY < bounds.top + bounds.height / 2;
    state.list.insertBefore(state.row, before ? targetRow : targetRow.nextElementSibling);
  }

  #drop(event) {
    const state = this.dragState;
    if (!state || this.isSearchActive() || !this.canMutate()) return;
    const targetList = event.target.closest?.(".subtask-list");
    if (targetList !== state.list) return;
    event.preventDefault();
    state.dropped = true;
    const orderedTaskIds = subtaskIds(state.list);
    this.#clearDrag();
    if (sameOrder(state.expectedTaskIds, orderedTaskIds)) return;
    this.pendingFocus = {
      parentTaskId: state.parentTaskId,
      taskId: state.movedTaskId,
      selector: ".subtask-handle",
    };
    this.#submitReorder(state, orderedTaskIds);
  }

  #endDrag() {
    const state = this.dragState;
    if (!state) return;
    if (!state.dropped) restoreSubtaskOrder(state.list, state.expectedTaskIds);
    this.#clearDrag();
  }

  #handleKeyboardMove(event) {
    const handle = event.target.closest?.(".subtask-handle");
    if (!handle || handle.disabled || !event.altKey || this.isSearchActive() || !this.canMutate()) return;
    const offset = event.key === "ArrowUp" ? -1 : event.key === "ArrowDown" ? 1 : 0;
    if (offset === 0) return;
    event.preventDefault();
    const row = handle.closest(".subtask-row");
    const list = row?.parentElement;
    const parentTaskId = row?.dataset.parentTaskId;
    const movedTaskId = row?.dataset.taskId;
    if (!list || !parentTaskId || !movedTaskId) return;
    const expectedTaskIds = subtaskIds(list);
    const orderedTaskIds = moveTaskByOffset(expectedTaskIds, movedTaskId, offset);
    if (sameOrder(expectedTaskIds, orderedTaskIds)) {
      this.#announce(offset < 0 ? "已经是第一项" : "已经是最后一项");
      return;
    }
    this.pendingFocus = { parentTaskId, taskId: movedTaskId, selector: ".subtask-handle" };
    this.#announce(offset < 0 ? "已上移一个子代办" : "已下移一个子代办");
    this.#submitReorder({ parentTaskId, movedTaskId, expectedTaskIds, list }, orderedTaskIds);
  }

  #submitReorder(state, orderedTaskIds) {
    Promise.resolve(this.onReorder(
      state.parentTaskId,
      state.movedTaskId,
      state.expectedTaskIds,
      orderedTaskIds,
    )).catch((error) => {
      restoreSubtaskOrder(state.list, state.expectedTaskIds);
      this.onError(error);
    });
  }

  #clearDrag() {
    this.dragState?.row.classList.remove("is-dragging");
    this.dragState = null;
  }

  #captureInput() {
    return this.#parentRow(this.addingParentId)?.querySelector(".subtask-capture-input") ?? null;
  }

  #parentRow(parentTaskId) {
    return [...this.list.querySelectorAll(":scope > .task-row")]
      .find((row) => row.dataset.taskId === parentTaskId) ?? null;
  }

  #subtaskRow(parentTaskId, taskId) {
    const parent = this.#parentRow(parentTaskId);
    return [...(parent?.querySelectorAll(":scope > .subtask-list > .subtask-row") ?? [])]
      .find((row) => row.dataset.taskId === taskId) ?? null;
  }

  #announce(message) {
    this.status.textContent = "";
    queueMicrotask(() => {
      this.status.textContent = message;
    });
  }
}

export function subtaskIds(list) {
  return [...list.querySelectorAll(":scope > .subtask-row")]
    .map((row) => row.dataset.taskId)
    .filter(Boolean);
}

function restoreSubtaskOrder(list, taskIds) {
  const rows = new Map(
    [...list.querySelectorAll(":scope > .subtask-row")]
      .map((row) => [row.dataset.taskId, row]),
  );
  const footer = list.querySelector(":scope > .subtask-add-row, :scope > .subtask-capture-row");
  taskIds.forEach((taskId) => {
    const row = rows.get(taskId);
    if (row) list.insertBefore(row, footer ?? null);
  });
}

function sameOrder(left, right) {
  return left.length === right.length && left.every((taskId, index) => taskId === right[index]);
}
