import { LedgerSession } from "./app/ledger-session.js";
import { LedgerCommand } from "./app/ledger-contract.js";
import { diagnosticsPassed } from "./app/selectors.js";
import { createInitialState, LedgerPhase } from "./app/state.js";
import { createOutboxStore } from "./app/infrastructure/outbox-store.js";
import { TauriGateway } from "./app/infrastructure/tauri-gateway.js";
import { mountPetComponents } from "./app/pet-component.js";
import { LedgerView, ToastView } from "./app/views.js";
import { ShellController } from "./app/shell-controller.js";
import { TaskListController } from "./app/task-list-controller.js";
import { SubtaskController } from "./app/subtask-controller.js";
import { isSearchShortcut, ListSearchController } from "./app/list-search-controller.js";
import { WindowController } from "./app/window-controller.js";
import { millisecondsUntilNextLocalDay } from "./app/deadline-date.js";
import { UpdateController } from "./app/update-controller.js";
import { WeeklyCompletionController } from "./app/weekly-completion-controller.js";
import { TauriClipboardWriter } from "./app/infrastructure/clipboard-writer.js";

const root = document.body;
const statusText = document.querySelector("#statusText");
const captureForm = document.querySelector("#captureForm");
const taskTitleInput = document.querySelector("#taskTitle");
const taskList = document.querySelector("#taskList");
const taskOrderStatus = document.querySelector("#taskOrderStatus");
const moreMenu = document.querySelector("#moreMenu");
const toast = new ToastView(document.querySelector("#toast"), window);
const ledgerView = new LedgerView(document);
let taskListController = null;
let subtaskController = null;
const searchController = new ListSearchController({
  root,
  form: document.querySelector("#listSearchForm"),
  label: document.querySelector("#listSearchLabel"),
  input: document.querySelector("#listSearchInput"),
  cancelButton: document.querySelector("#listSearchCancel"),
  searchAction: document.querySelector("#searchAction"),
  captureForm,
  taskTitleInput,
  historyHeading: document.querySelector("#historyHeading"),
  historyBackButton: document.querySelector("#historyBackButton"),
  menuButton: document.querySelector("#moreMenuButton"),
  onChange: (searchState) => {
    ledgerView.setSearch(searchState);
    taskListController?.restorePendingFocus();
    subtaskController?.restorePendingFocus();
  },
});
const shellController = new ShellController({ root, menu: moreMenu, search: searchController });
const gateway = TauriGateway.fromWindow(window);

mountPetComponents(document);

if (!gateway) {
  showStaticPreview();
} else {
  startDesktopApplication(gateway);
}

function showStaticPreview() {
  const windowController = new WindowController({
    gateway: null,
    root,
    statusText,
  });
  const previewError = new Error("浏览器预览未连接本地账本");
  ledgerView.render({
    ...createInitialState(),
    phase: LedgerPhase.ERROR,
    error: previewError,
  });
  windowController.showStaticPreview();
  toast.show(errorMessage(previewError));
}

/** @param {TauriGateway} activeGateway */
function startDesktopApplication(activeGateway) {
  const session = new LedgerSession({
    gateway: activeGateway,
    outboxStoreFactory: (profile) => profile === "normal"
      ? createOutboxStore(profile, window.localStorage)
      : createOutboxStore(profile),
  });
  const windowController = new WindowController({
    gateway: activeGateway,
    root,
    statusText,
  });
  const updateController = new UpdateController({
    gateway: activeGateway,
    actionButton: document.querySelector("#updateAction"),
    root: document.querySelector("#desktopRoot"),
    toast,
    timerHost: window,
  });
  const weeklyCompletionController = new WeeklyCompletionController({
    gateway: activeGateway,
    clipboard: TauriClipboardWriter.fromWindow(window),
    toast,
  });
  window.addEventListener("beforeunload", () => updateController.stop(), { once: true });
  taskListController = new TaskListController({
    list: taskList,
    status: taskOrderStatus,
    onReorder: (movedTaskId, expectedTaskIds, orderedTaskIds) => session.reorderTasks(
      movedTaskId,
      expectedTaskIds,
      orderedTaskIds,
    ),
    onUpdateDeadline: (taskId, deadlineOn) => session.updateTaskDeadline(taskId, deadlineOn),
    onUpdateTitle: (taskId, title) => session.updateTaskTitle(taskId, title),
    onError: (error) => handleMutationFailure(session, error),
    canReorder: () => !searchController.isActive("tasks"),
    isSearchActive: () => searchController.isActive("tasks"),
    focusSearchFallback: () => searchController.focus({ select: false }),
  });
  subtaskController = new SubtaskController({
    list: taskList,
    status: taskOrderStatus,
    deleteDialog: document.querySelector("#deleteGroupDialog"),
    deleteDialogTitle: document.querySelector("#deleteGroupDialogTitle"),
    onCreate: (parentTaskId, title) => session.createSubtask(parentTaskId, title),
    onUpdateTitle: (taskId, title) => session.updateTaskTitle(taskId, title),
    onReorder: (parentTaskId, movedTaskId, expectedTaskIds, orderedTaskIds) => (
      session.reorderSubtasks(parentTaskId, movedTaskId, expectedTaskIds, orderedTaskIds)
    ),
    onError: (error) => handleMutationFailure(session, error),
    canMutate: () => session.canMutate(),
    isSearchActive: () => searchController.isActive("tasks"),
    focusSearchFallback: () => searchController.focus({ select: false }),
  });
  scheduleDeadlinePresentationRefresh(ledgerView);

  session.subscribe((state) => {
    ledgerView.render(state);
    taskListController.restorePendingFocus();
    subtaskController.restorePendingFocus();
  });

  captureForm.addEventListener("submit", async (event) => {
    event.preventDefault();
    const title = taskTitleInput.value.trim();
    if (!title || !session.canMutate()) return;
    try {
      const operation = await session.captureTask(title);
      clearCapturedTitle(operation);
      if (operation) toast.show("已记下");
    } catch (error) {
      handleMutationFailure(session, error);
    }
  });

  document.addEventListener("click", async (event) => {
    if (!event.target.closest("#moreMenu")) shellController.closeMenu();
    const button = event.target.closest("[data-action]");
    if (!button || button.disabled) return;
    const action = button.dataset.action;
    const ledgerAction = action === "complete-task"
      || action === "delete-task"
      || action === "undo-completion";
    const ledgerBoundReadAction = action === "copy-weekly-completions";
    if (!ledgerAction) button.disabled = true;
    try {
      await handleAction({
        action,
        button,
        session,
        shellController,
        searchController,
        taskListController,
        subtaskController,
        updateController,
        weeklyCompletionController,
        windowController,
      });
    } catch (error) {
      if (ledgerAction) {
        handleMutationFailure(session, error);
      } else {
        console.error(error);
        toast.show(errorMessage(error));
      }
    } finally {
      if (!ledgerAction && button.isConnected) {
        button.disabled = ledgerBoundReadAction ? !session.canMutate() : false;
      }
    }
  });

  window.addEventListener("keydown", async (event) => {
    if (event.isComposing || event.keyCode === 229 || searchController.isComposing()) return;
    if (document.querySelector("#deleteGroupDialog")?.open) return;
    if (isSearchShortcut(event)) {
      event.preventDefault();
      try {
        if (windowController.mode !== "expanded") {
          await windowController.setMode("expanded");
          shellController.showTasks();
        }
        shellController.closeMenu();
        searchController.open(root.dataset.panel === "history" ? "history" : "tasks");
      } catch (error) {
        console.error(error);
        toast.show(errorMessage(error));
      }
      return;
    }
    if (event.key !== "Escape" || windowController.mode !== "expanded") return;
    event.preventDefault();
    if (shellController.closeTransientUi()) return;
    try {
      await windowController.setMode("capsule");
    } catch (error) {
      console.error(error);
    }
  });

  start(session, updateController, windowController);
}

/** 期限只是展示派生值；午夜刷新 DOM，不写账本也不重绘任务列表。 */
function scheduleDeadlinePresentationRefresh(view) {
  let timer = 0;
  const schedule = () => {
    timer = window.setTimeout(() => {
      view.refreshDeadlineLabels();
      schedule();
    }, millisecondsUntilNextLocalDay() + 250);
  };
  schedule();
  window.addEventListener("beforeunload", () => window.clearTimeout(timer), { once: true });
}

async function start(session, updateController, windowController) {
  try {
    await windowController.subscribeToStatusChanges();
    const result = await session.start();
    if (!result) return;
    windowController.applyStatus(result.status);
    clearCapturedTitle(result.operation);
    if (result.recoveryError) {
      toast.show(errorMessage(result.recoveryError));
    } else if (result.recovered) {
      toast.show("上次未确认的操作已恢复");
    }
    void updateController.start(session.state.profile, session);
  } catch (error) {
    console.error(error);
    toast.show(errorMessage(error));
  }
}

async function runDiagnostics(session, windowController) {
  const result = await session.runDiagnostics();
  if (!result) return;
  windowController.applyStatus(result.status);
  clearCapturedTitle(result.operation);
  if (result.recoveryError) {
    toast.show(errorMessage(result.recoveryError));
    return;
  }
  toast.show(
    diagnosticsPassed(result.status, result.integrity)
      ? "窗口与本地账本检查通过"
      : "检查发现异常",
  );
}

async function handleAction({
  action,
  button,
  session,
  shellController,
  searchController,
  taskListController,
  subtaskController,
  updateController,
  weeklyCompletionController,
  windowController,
}) {
  switch (action) {
    case "expanded":
      await windowController.setMode(action);
      shellController.showTasks();
      if (!subtaskController.revealFromCapsule(
        button.dataset.parentTaskId,
        button.dataset.taskId,
      )) taskTitleInput.focus();
      return;
    case "edit-current-deadline":
      await windowController.setMode("expanded");
      shellController.showTasks();
      if (!taskListController.beginDeadlineEdit(button.dataset.taskId)) {
        taskTitleInput.focus();
      }
      return;
    case "capsule":
    case "pet":
    case "edge":
      await windowController.setMode(action);
      shellController.showTasks();
      return;
    case "show-history":
      ledgerView.resetHistoryExpansion();
      subtaskController.sync();
      shellController.showHistory();
      return;
    case "show-tasks":
      shellController.showTasks();
      return;
    case "toggle-history-group":
      ledgerView.toggleHistoryGroup(button.dataset.parentTaskId);
      subtaskController.sync();
      return;
    case "search":
      shellController.closeMenu();
      searchController.open(root.dataset.panel === "history" ? "history" : "tasks");
      return;
    case "hide":
      shellController.showTasks();
      await windowController.hideToTray();
      return;
    case "diagnostics":
      shellController.closeMenu();
      await runDiagnostics(session, windowController);
      return;
    case "update":
      await updateController.handleAction();
      return;
    case "copy-weekly-completions":
      await weeklyCompletionController.copyCurrentWeek();
      return;
    case "complete-task": {
      const taskId = button.dataset.taskId;
      if (!taskId) return;
      const isSubtask = button.dataset.isSubtask === "true";
      if (!isSubtask) {
        taskListController.rememberCompletionFocus(taskId);
      } else {
        if (button.id !== "capsuleTaskCheckbox") {
          subtaskController.rememberSubtaskFocus(button.dataset.parentTaskId, taskId);
        }
      }
      const operation = await session.completeTask(taskId);
      if (operation) toast.show("已完成");
      return;
    }
    case "delete-task": {
      const taskId = button.dataset.taskId;
      if (!taskId) return;
      const isSubtask = button.dataset.isSubtask === "true";
      if (!isSubtask && button.dataset.hasSubtasks === "true") {
        const confirmed = await subtaskController.confirmGroupDeletion(button.dataset.taskTitle || "该代办");
        if (!confirmed) return;
      }
      if (isSubtask) {
        subtaskController.rememberSubtaskFocus(button.dataset.parentTaskId, taskId, { remains: false });
      } else {
        taskListController.rememberRemovalFocus(taskId);
      }
      const operation = await session.deleteTask(taskId);
      if (operation) toast.show("已从待办删除");
      return;
    }
    case "undo-completion": {
      const isSubtask = button.dataset.isSubtask === "true";
      if (isSubtask && button.dataset.parentTaskId && button.closest("#taskList")) {
        subtaskController.rememberSubtaskFocus(
          button.dataset.parentTaskId,
          button.dataset.taskId || "",
        );
      }
      const operation = await session.undoCompletion(button.dataset.eventId);
      if (operation) toast.show(isSubtask ? "已撤销子代办完成" : "已撤销，任务回到队尾");
      searchController.focus({ select: false });
      return;
    }
    default:
      throw new Error(`未知操作：${action}`);
  }
}

function clearCapturedTitle(operation) {
  if (
    operation?.command === LedgerCommand.CAPTURE
      && taskTitleInput.value.trim() === operation.payload.title
  ) {
    taskTitleInput.value = "";
  }
}

function handleMutationFailure(session, error) {
  console.error(error);
  const pendingOperation = session.state.pendingOperation;
  if (pendingOperation?.committed) {
    toast.show("操作已经保存，但列表刷新失败；点击“检查”继续恢复");
  } else if (pendingOperation) {
    toast.show("操作结果尚未确认；点击“检查”继续恢复");
  } else {
    toast.show(errorMessage(error));
  }
}

function errorMessage(error) {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  if (error && typeof error.message === "string") {
    return error.code ? `${error.message}（${error.code}）` : error.message;
  }
  return "操作失败";
}
