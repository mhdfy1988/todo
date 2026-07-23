const appWindow = document.querySelector("#appWindow");
const taskList = document.querySelector("#taskList");
const historyList = document.querySelector("#historyList");
const historyCount = document.querySelector("#historyCount");
const tasksView = document.querySelector("#tasksView");
const historyView = document.querySelector("#historyView");
const captureForm = document.querySelector("#captureForm");
const captureInput = document.querySelector("#captureInput");
const searchForm = document.querySelector("#searchForm");
const searchInput = document.querySelector("#searchInput");
const searchLabel = document.querySelector("#searchLabel");
const searchMenuAction = document.querySelector("#searchMenuAction");
const capsuleSurface = document.querySelector(".capsule-surface");
const capsuleCheckbox = document.querySelector("#capsuleCheckbox");
const capsuleTitle = document.querySelector("#capsuleTitle");
const capsuleProgress = document.querySelector("#capsuleProgress");
const capsuleDeadline = document.querySelector("#capsuleDeadline");
const toast = document.querySelector("#toast");
const deleteGroupDialog = document.querySelector("#deleteGroupDialog");
const deleteDialogTitle = document.querySelector("#deleteDialogTitle");
const deleteDialogCopy = document.querySelector("#deleteDialogCopy");
const sceneHint = document.querySelector("#sceneHint");

const sceneCopy = {
  default: "历史原型：当时点击进度展开，并把新增入口放在按需区域；v0.1.3 已改为父行固定“＋”。",
  add: "历史原型：当时成功后恢复底部入口；v0.1.3 改为父行固定“＋”，空新增取消按是否已有子项决定收起或保持展开。",
  complete: "勾选父代办会自动完成剩余子代办，再完成父项；单独完成最后一个子代办仍不会自动完成父项。",
  delete: "只有删除整个任务组才确认；删除单个未完成子代办仍是低打扰的可追溯软删除。",
  search: "命中子代办时保留父代办上下文，进度仍显示整组真实进度；搜索期间禁止重排和添加。",
  history: "子代办完成记录按父代办归组；父代办完成后，必须先撤销父项才能撤销其子项。",
  capsule: "胶囊显示队首父代办里的第一个未完成子代办；完成圆圈始终对应屏幕上显示的那件事。",
};

let idSeed = 20;
let toastTimer = null;
let state = createInitialState();

function createInitialState() {
  return {
    mode: "expanded",
    panel: "tasks",
    searching: false,
    query: "",
    editor: null,
    addingParentId: null,
    pendingDeleteId: null,
    dragged: null,
    expandedHistoryIds: new Set(),
    tasks: [
      {
        id: "weekly",
        title: "写周报",
        deadlineOn: "2026-07-25",
        expanded: true,
        children: [
          { id: "collect", title: "汇总本周完成", completedAt: "7/23 09:40" },
          { id: "issues", title: "整理本周问题", completedAt: null },
          { id: "plan", title: "写下周计划", completedAt: null },
        ],
      },
      {
        id: "feedback",
        title: "回复项目反馈",
        deadlineOn: null,
        expanded: false,
        children: [],
      },
      {
        id: "update",
        title: "测试自动更新",
        deadlineOn: "2026-07-26",
        expanded: false,
        children: [],
      },
    ],
    completedGroups: [],
    deletedGroups: [],
    completedStandalone: [
      { id: "release", title: "完成 0.1.1 发布验证", completedAt: "7/22 18:10" },
    ],
  };
}

function resetState() {
  state = createInitialState();
  window.clearTimeout(toastTimer);
  toastTimer = null;
  toast.textContent = "";
  toast.classList.remove("show");
  closeMenu();
  if (deleteGroupDialog.open) deleteGroupDialog.close("cancel");
}

function render() {
  appWindow.dataset.mode = state.mode;
  appWindow.dataset.panel = state.panel;
  appWindow.classList.toggle("is-searching", state.searching);
  document.querySelector(".expanded-surface").hidden = state.mode === "capsule";
  capsuleSurface.hidden = state.mode !== "capsule";
  searchForm.hidden = !state.searching || state.mode === "capsule";
  captureForm.hidden = state.searching || state.panel !== "tasks";
  tasksView.hidden = state.panel !== "tasks" || state.mode === "capsule";
  historyView.hidden = state.panel !== "history" || state.mode === "capsule";
  const searchCopy = state.panel === "history" ? "搜索完成记录" : "搜索待办";
  searchInput.placeholder = searchCopy;
  searchLabel.textContent = searchCopy;
  searchMenuAction.textContent = searchCopy;
  if (searchInput.value !== state.query) searchInput.value = state.query;

  renderTasks();
  renderHistory();
  renderCapsule();
  historyCount.textContent = String(countHistoryRecords());
  restoreTransientFocus();
}

function renderTasks() {
  const query = normalizeQuery(state.query);
  const visible = state.tasks
    .map((parent) => projectParentForSearch(parent, query))
    .filter(Boolean);

  if (!visible.length) {
    taskList.innerHTML = `<li class="empty-row">${query ? "没有找到相关待办" : "还没有待办"}</li>`;
    return;
  }

  taskList.innerHTML = visible.map(({ parent, visibleChildren, forceExpanded, parentMatched }, index) => {
    const completedCount = parent.children.filter((child) => child.completedAt).length;
    const totalCount = parent.children.length;
    const allDone = totalCount > 0 && completedCount === totalCount;
    const expanded = forceExpanded || parent.expanded;
    const isCurrent = state.tasks[0]?.id === parent.id;
    const firstIncompleteId = parent.children.find((child) => !child.completedAt)?.id ?? null;
    const editingParent = state.editor?.kind === "parent" && state.editor.parentId === parent.id;
    const showChildRegion = expanded && (totalCount > 0 || state.addingParentId === parent.id);
    const childMarkup = expanded && totalCount > 0
      ? renderChildList(parent, visibleChildren, query, firstIncompleteId)
      : "";
    const addMarkup = showChildRegion && !state.searching
      ? renderAddChildRow(parent.id)
      : "";
    const parentTitle = highlight(parent.title, query && parentMatched ? query : "");

    return `
      <li class="task-group${isCurrent ? " is-current" : ""}" data-parent-id="${parent.id}">
        <div class="parent-row" data-row-kind="parent" data-id="${parent.id}">
          <button type="button"
            class="circle-button"
            data-action="complete-parent"
            data-parent-id="${parent.id}"
            aria-label="完成：${escapeAttribute(parent.title)}"></button>
          <div class="task-main">
            <button type="button" class="task-title" data-action="edit-parent" data-parent-id="${parent.id}"
              title="双击或按 Enter 修改" aria-label="修改代办：${escapeAttribute(parent.title)}">${parentTitle}</button>
            ${totalCount === 0 && !state.searching
              ? `<button type="button" class="quick-child-button" data-action="add-child" data-parent-id="${parent.id}">＋ 子项</button>`
              : ""}
          </div>
          <div class="task-meta">
            ${totalCount > 0
              ? `<button type="button" class="progress-toggle${allDone ? " is-ready" : ""}" data-action="toggle-parent"
                  data-parent-id="${parent.id}" aria-expanded="${expanded}" aria-controls="children-${parent.id}">
                  <span>${completedCount}/${totalCount}</span><i class="chevron" aria-hidden="true"></i>
                </button>`
              : ""}
            ${parent.deadlineOn ? `<span class="task-deadline">${formatDeadline(parent.deadlineOn)}</span>` : ""}
          </div>
          <button type="button" class="delete-button" data-action="delete-parent" data-parent-id="${parent.id}"
            aria-label="删除：${escapeAttribute(parent.title)}" title="删除">×</button>
          <button type="button" class="drag-handle" draggable="${!state.searching}" data-drag-kind="parent"
            data-parent-id="${parent.id}" aria-label="调整顺序：${escapeAttribute(parent.title)}"
            aria-keyshortcuts="Alt+ArrowUp Alt+ArrowDown">⠿</button>
        </div>
        ${editingParent ? renderParentEditor(parent) : ""}
        ${showChildRegion ? `<ol id="children-${parent.id}" class="child-list" aria-label="${escapeAttribute(parent.title)}的子代办">${childMarkup}${addMarkup}</ol>` : ""}
      </li>`;
  }).join("");
}

function renderChildList(parent, visibleChildren, query, firstIncompleteId) {
  return visibleChildren.map((child) => {
    const editing = state.editor?.kind === "child"
      && state.editor.parentId === parent.id
      && state.editor.childId === child.id;
    const completed = Boolean(child.completedAt);
    const titleMarkup = editing
      ? `<input class="inline-child-editor" data-editor="child" data-parent-id="${parent.id}" data-child-id="${child.id}"
          maxlength="500" value="${escapeAttribute(child.title)}" aria-label="修改子代办">`
      : `<button type="button" class="child-title" data-action="edit-child" data-parent-id="${parent.id}"
          data-child-id="${child.id}" title="双击或按 Enter 修改">${highlight(child.title, query)}</button>`;
    return `
      <li class="child-row${completed ? " is-completed" : ""}${child.id === firstIncompleteId ? " is-current-child" : ""}"
        data-row-kind="child" data-parent-id="${parent.id}" data-child-id="${child.id}">
        <button type="button" class="circle-button${completed ? " is-checked" : ""}" data-action="toggle-child"
          data-parent-id="${parent.id}" data-child-id="${child.id}"
          aria-label="${completed ? "撤销完成" : "完成"}：${escapeAttribute(child.title)}"></button>
        <div class="child-main">${titleMarkup}</div>
        <button type="button" class="delete-button" data-action="delete-child" data-parent-id="${parent.id}"
          data-child-id="${child.id}" aria-label="删除子代办：${escapeAttribute(child.title)}" title="删除"
          ${completed ? "disabled" : ""}>×</button>
        <button type="button" class="drag-handle" draggable="${!state.searching}" data-drag-kind="child"
          data-parent-id="${parent.id}" data-child-id="${child.id}"
          aria-label="调整子代办顺序：${escapeAttribute(child.title)}"
          aria-keyshortcuts="Alt+ArrowUp Alt+ArrowDown">⠿</button>
      </li>`;
  }).join("");
}

function renderAddChildRow(parentId) {
  const adding = state.addingParentId === parentId;
  return `
    <li class="add-child-row">
      ${adding
        ? `<form class="child-add-form" data-parent-id="${parentId}">
            <input data-add-child-input="${parentId}" maxlength="500" placeholder="输入子代办" aria-label="添加子代办">
          </form>`
        : `<button type="button" class="add-child-trigger" data-action="add-child" data-parent-id="${parentId}">＋ 添加子代办</button>`}
    </li>`;
}

function renderParentEditor(parent) {
  return `
    <form class="editor-panel" data-parent-editor="${parent.id}">
      <div class="editor-grid">
        <label class="editor-field">
          <span>标题</span>
          <input name="title" maxlength="500" value="${escapeAttribute(parent.title)}" aria-label="代办标题">
        </label>
        <label class="editor-field">
          <span>期限（可选）</span>
          <input name="deadlineOn" type="date" value="${parent.deadlineOn ?? ""}" aria-label="代办期限">
        </label>
      </div>
      <div class="editor-actions">
        <button type="button" class="editor-add-child" data-action="add-child" data-parent-id="${parent.id}">＋ 添加子代办</button>
        <button type="button" data-action="cancel-editor">取消</button>
        <button type="submit">保存</button>
      </div>
    </form>`;
}

function renderHistory() {
  const query = normalizeQuery(state.query);
  const activeGroups = state.tasks
    .filter((parent) => parent.children.some((child) => child.completedAt))
    .map((parent) => ({ parent, status: "active" }));
  const completedGroups = state.completedGroups.map((parent) => ({ parent, status: "completed" }));
  const deletedGroups = state.deletedGroups.map((parent) => ({ parent, status: "deleted" }));
  const groups = [...activeGroups, ...completedGroups, ...deletedGroups]
    .map((group) => projectHistoryGroup(group, query))
    .filter(Boolean);
  const standalone = state.completedStandalone.filter((entry) => !query || includesQuery(entry.title, query));

  if (!groups.length && !standalone.length) {
    historyList.innerHTML = `<li class="empty-row">${query ? "没有找到相关完成记录" : "还没有完成记录"}</li>`;
    return;
  }

  historyList.innerHTML = [
    ...groups.map(({ parent, status, visibleChildren, parentMatched, forceExpanded }) => {
      const completedCount = parent.children.filter((child) => child.completedAt).length;
      const parentCompleted = status === "completed";
      const expanded = forceExpanded || state.expandedHistoryIds.has(parent.id);
      const stateText = parentCompleted
        ? `${completedCount}/${parent.children.length} · ${parent.completedAt}`
        : status === "deleted"
          ? `${completedCount}/${parent.children.length} · 已删除`
          : `进行中 · ${completedCount}/${parent.children.length}`;
      return `
        <li class="history-group">
          <div class="history-parent">
            <b>${highlight(parent.title, parentMatched ? query : "")}</b>
            <span class="history-state">${stateText}</span>
            <button type="button" class="history-toggle" data-action="toggle-history-group" data-parent-id="${parent.id}"
              aria-expanded="${expanded}" aria-controls="history-children-${parent.id}"
              aria-label="${expanded ? "收起" : "展开"}${escapeAttribute(parent.title)}的子代办完成记录">
              <i class="chevron" aria-hidden="true"></i>
            </button>
            ${parentCompleted
              ? `<button type="button" class="history-undo" data-action="undo-parent" data-parent-id="${parent.id}"
                  aria-label="撤销完成：${escapeAttribute(parent.title)}" title="撤销完成">↶</button>`
              : `<span class="history-action-spacer"></span>`}
          </div>
          ${expanded
            ? `<ol id="history-children-${parent.id}" class="history-children" aria-label="${escapeAttribute(parent.title)}的已完成子代办">
                ${visibleChildren.map((child) => `
                  <li class="history-child">
                    <span>${highlight(child.title, query)}</span>
                    <time>${child.completedAt}</time>
                    <button type="button" class="history-undo" data-action="undo-child" data-parent-id="${parent.id}"
                      data-child-id="${child.id}" aria-label="撤销完成：${escapeAttribute(parent.title)} / ${escapeAttribute(child.title)}"
                      title="${parentCompleted ? "请先撤销父代办" : status === "deleted" ? "父代办已删除" : "撤销完成"}"
                      ${status === "active" ? "" : "disabled"}>↶</button>
                  </li>`).join("")}
              </ol>`
            : ""}
        </li>`;
    }),
    ...standalone.map((entry) => `
      <li class="history-standalone">
        <b>${highlight(entry.title, query)}</b>
        <time>${entry.completedAt}</time>
        <button type="button" class="history-undo" data-action="undo-standalone" data-entry-id="${entry.id}"
          aria-label="撤销完成：${escapeAttribute(entry.title)}" title="撤销完成">↶</button>
      </li>`),
  ].join("");
}

function renderCapsule() {
  const parent = state.tasks[0];
  if (!parent) {
    capsuleTitle.textContent = "暂无待办";
    capsuleProgress.hidden = true;
    capsuleDeadline.hidden = true;
    capsuleCheckbox.hidden = true;
    return;
  }

  const nextChild = parent.children.find((child) => !child.completedAt) ?? null;
  const completedCount = parent.children.filter((child) => child.completedAt).length;
  capsuleCheckbox.hidden = false;
  capsuleCheckbox.dataset.parentId = parent.id;
  capsuleCheckbox.dataset.childId = nextChild?.id ?? "";
  capsuleCheckbox.setAttribute("aria-label", nextChild
    ? `完成子代办：${nextChild.title}`
    : `完成代办：${parent.title}`);
  capsuleTitle.innerHTML = nextChild
    ? `<span class="capsule-parent">${escapeHtml(parent.title)} / </span>${escapeHtml(nextChild.title)}`
    : escapeHtml(parent.title);
  capsuleProgress.hidden = parent.children.length === 0;
  capsuleProgress.textContent = `${completedCount}/${parent.children.length}`;
  capsuleDeadline.hidden = !parent.deadlineOn;
  capsuleDeadline.textContent = parent.deadlineOn ? formatDeadline(parent.deadlineOn) : "";
}

function projectParentForSearch(parent, query) {
  if (!query) return { parent, visibleChildren: parent.children, forceExpanded: false, parentMatched: false };
  const parentMatched = includesQuery(parent.title, query);
  const matchingChildren = parent.children.filter((child) => includesQuery(child.title, query));
  if (!parentMatched && !matchingChildren.length) return null;
  return {
    parent,
    visibleChildren: parentMatched ? parent.children : matchingChildren,
    forceExpanded: !parentMatched && matchingChildren.length > 0,
    parentMatched,
  };
}

function projectHistoryGroup({ parent, status }, query) {
  const completedChildren = parent.children.filter((child) => child.completedAt);
  if (!query) {
    return { parent, status, visibleChildren: completedChildren, parentMatched: false, forceExpanded: false };
  }
  const parentMatched = includesQuery(parent.title, query);
  const matchingChildren = completedChildren.filter((child) => includesQuery(child.title, query));
  if (!parentMatched && !matchingChildren.length) return null;
  return {
    parent,
    status,
    visibleChildren: parentMatched ? completedChildren : matchingChildren,
    parentMatched,
    forceExpanded: !parentMatched && matchingChildren.length > 0,
  };
}

function countHistoryRecords() {
  const activeChildRecords = state.tasks.reduce(
    (count, parent) => count + parent.children.filter((child) => child.completedAt).length,
    0,
  );
  const completedGroupRecords = state.completedGroups.reduce(
    (count, parent) => count + 1 + parent.children.filter((child) => child.completedAt).length,
    0,
  );
  const deletedGroupRecords = state.deletedGroups.reduce(
    (count, parent) => count + parent.children.filter((child) => child.completedAt).length,
    0,
  );
  return activeChildRecords + completedGroupRecords + deletedGroupRecords + state.completedStandalone.length;
}

function startParentEdit(parentId) {
  if (state.searching) return;
  state.editor = { kind: "parent", parentId };
  state.addingParentId = null;
  const parent = findParent(parentId);
  if (parent) parent.expanded = true;
  render();
}

function startChildEdit(parentId, childId) {
  if (state.searching) return;
  const child = findChild(parentId, childId);
  if (!child || child.completedAt) {
    showToast("已完成的子代办需要先撤销完成才能修改");
    return;
  }
  state.editor = { kind: "child", parentId, childId };
  state.addingParentId = null;
  render();
}

function startAddingChild(parentId) {
  if (state.searching) return;
  const parent = findParent(parentId);
  if (!parent) return;
  parent.expanded = true;
  state.addingParentId = parentId;
  render();
}

function submitChildAdd(input, { exitOnBlank = false } = {}) {
  const parentId = input.dataset.addChildInput;
  if (!parentId || state.addingParentId !== parentId) return false;
  const title = input.value.trim();
  if (!title) {
    if (exitOnBlank) {
      state.addingParentId = null;
      render();
    }
    return false;
  }
  const parent = findParent(parentId);
  if (!parent) return false;
  parent.children.push({ id: `child-${++idSeed}`, title, completedAt: null });
  state.addingParentId = null;
  showToast(`已添加子代办“${title}”`);
  render();
  return true;
}

function completeParent(parentId) {
  const parent = findParent(parentId);
  if (!parent) return;
  const remaining = parent.children.filter((child) => !child.completedAt);
  const completedAt = nowLabel();
  remaining.forEach((child) => { child.completedAt = completedAt; });
  const index = state.tasks.findIndex((task) => task.id === parentId);
  const [completed] = state.tasks.splice(index, 1);
  completed.completedAt = completedAt;
  if (completed.children.length) state.completedGroups.unshift(completed);
  else state.completedStandalone.unshift({ id: completed.id, title: completed.title, completedAt: completed.completedAt });
  state.editor = null;
  state.addingParentId = null;
  showToast(remaining.length
    ? `已完成“${completed.title}”及 ${remaining.length} 个剩余子代办`
    : `已完成“${completed.title}”`);
  render();
}

function toggleChild(parentId, childId) {
  const parent = findParent(parentId);
  const child = findChild(parentId, childId);
  if (!parent || !child) return;
  child.completedAt = child.completedAt ? null : nowLabel();
  const allDone = parent.children.length > 0 && parent.children.every((item) => item.completedAt);
  showToast(child.completedAt
    ? (allDone ? "子代办已全部完成，请勾选父代办确认" : `已完成“${child.title}”`)
    : `已撤销“${child.title}”`);
  render();
}

function requestDeleteParent(parentId) {
  const parent = findParent(parentId);
  if (!parent) return;
  if (!parent.children.length) {
    state.tasks = state.tasks.filter((task) => task.id !== parentId);
    showToast(`已删除“${parent.title}”`);
    render();
    return;
  }
  const remaining = parent.children.filter((child) => !child.completedAt).length;
  state.pendingDeleteId = parentId;
  deleteDialogTitle.textContent = `删除“${parent.title}”？`;
  deleteDialogCopy.textContent = `父代办和 ${remaining} 个未完成子代办都会从列表移除，${parent.children.length - remaining} 条已有完成记录仍然保留。`;
  if (!deleteGroupDialog.open) deleteGroupDialog.showModal();
}

function deleteChild(parentId, childId) {
  const parent = findParent(parentId);
  const child = findChild(parentId, childId);
  if (!parent || !child) return;
  if (child.completedAt) {
    showToast("请先撤销该子代办的完成状态");
    return;
  }
  parent.children = parent.children.filter((item) => item.id !== childId);
  state.editor = null;
  showToast(`已删除子代办“${child.title}”`);
  render();
}

function undoParent(parentId) {
  const index = state.completedGroups.findIndex((parent) => parent.id === parentId);
  if (index < 0) return;
  const [parent] = state.completedGroups.splice(index, 1);
  delete parent.completedAt;
  parent.expanded = false;
  state.tasks.push(parent);
  showToast(`已撤销“${parent.title}”，任务组回到队尾`);
  render();
}

function undoChild(parentId, childId) {
  const child = findChild(parentId, childId);
  if (!child) return;
  child.completedAt = null;
  showToast(`已撤销“${child.title}”`);
  render();
}

function undoStandalone(entryId) {
  const index = state.completedStandalone.findIndex((entry) => entry.id === entryId);
  if (index < 0) return;
  const [entry] = state.completedStandalone.splice(index, 1);
  state.tasks.push({ id: entry.id, title: entry.title, deadlineOn: null, expanded: false, children: [] });
  showToast(`已撤销“${entry.title}”，代办回到队尾`);
  render();
}

function reorder(kind, parentId, itemId, delta) {
  if (state.searching) return;
  const list = kind === "parent" ? state.tasks : findParent(parentId)?.children;
  if (!list) return;
  const index = list.findIndex((item) => item.id === itemId);
  const target = index + delta;
  if (index < 0 || target < 0 || target >= list.length) return;
  const [moved] = list.splice(index, 1);
  list.splice(target, 0, moved);
  showToast(kind === "parent" ? "已调整代办顺序" : "已调整子代办顺序");
  render();
  queueMicrotask(() => {
    const selector = kind === "parent"
      ? `[data-drag-kind="parent"][data-parent-id="${itemId}"]`
      : `[data-drag-kind="child"][data-child-id="${itemId}"]`;
    taskList.querySelector(selector)?.focus();
  });
}

function showTasks() {
  state.panel = "tasks";
  state.searching = false;
  state.query = "";
  render();
}

function showHistory() {
  state.panel = "history";
  state.searching = false;
  state.query = "";
  state.expandedHistoryIds.clear();
  closeMenu();
  render();
}

function startSearch() {
  state.searching = true;
  state.query = "";
  state.editor = null;
  state.addingParentId = null;
  closeMenu();
  render();
  queueMicrotask(() => searchInput.focus());
}

function cancelSearch() {
  state.searching = false;
  state.query = "";
  render();
}

function showToast(message) {
  window.clearTimeout(toastTimer);
  toast.textContent = message;
  toast.classList.add("show");
  toastTimer = window.setTimeout(() => toast.classList.remove("show"), 2600);
}

function setScene(scene) {
  resetState();
  document.querySelectorAll(".scene-button").forEach((button) => {
    button.classList.toggle("is-active", button.dataset.scene === scene);
  });
  sceneHint.textContent = sceneCopy[scene];

  if (scene === "add") {
    const parent = findParent("feedback");
    parent.expanded = true;
    state.editor = { kind: "parent", parentId: "feedback" };
    state.addingParentId = "feedback";
  } else if (scene === "complete") {
    const parent = findParent("weekly");
    parent.children[1].completedAt = "7/23 11:20";
    parent.expanded = true;
  } else if (scene === "search") {
    state.searching = true;
    state.query = "问题";
  } else if (scene === "history") {
    state.panel = "history";
  } else if (scene === "capsule") {
    state.mode = "capsule";
  }

  render();

  if (scene === "delete") {
    window.setTimeout(() => requestDeleteParent("weekly"), 80);
  } else if (scene === "search") {
    queueMicrotask(() => searchInput.focus());
  }
}

function restoreTransientFocus() {
  queueMicrotask(() => {
    if (state.addingParentId) {
      taskList.querySelector(`[data-add-child-input="${state.addingParentId}"]`)?.focus();
      return;
    }
    if (state.editor?.kind === "parent") {
      taskList.querySelector(`[data-parent-editor="${state.editor.parentId}"] input[name="title"]`)?.focus();
      return;
    }
    if (state.editor?.kind === "child") {
      taskList.querySelector(`[data-editor="child"][data-child-id="${state.editor.childId}"]`)?.focus();
    }
  });
}

function findParent(parentId) {
  return state.tasks.find((parent) => parent.id === parentId) ?? null;
}

function findChild(parentId, childId) {
  return findParent(parentId)?.children.find((child) => child.id === childId) ?? null;
}

function normalizeQuery(value) {
  return value.trim().toLocaleLowerCase("zh-CN");
}

function includesQuery(value, query) {
  return value.toLocaleLowerCase("zh-CN").includes(query);
}

function highlight(value, query) {
  if (!query) return escapeHtml(value);
  const source = value.toLocaleLowerCase("zh-CN");
  const index = source.indexOf(query);
  if (index < 0) return escapeHtml(value);
  const end = index + query.length;
  return `${escapeHtml(value.slice(0, index))}<mark class="search-match">${escapeHtml(value.slice(index, end))}</mark>${escapeHtml(value.slice(end))}`;
}

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

function escapeAttribute(value) {
  return escapeHtml(value);
}

function formatDeadline(value) {
  const [, month, day] = value.split("-");
  return `${Number(month)}/${Number(day)}`;
}

function nowLabel() {
  const now = new Date();
  return `${now.getMonth() + 1}/${now.getDate()} ${String(now.getHours()).padStart(2, "0")}:${String(now.getMinutes()).padStart(2, "0")}`;
}

function closeMenu() {
  document.querySelector("#moreMenu")?.removeAttribute("open");
}

captureForm.addEventListener("submit", (event) => {
  event.preventDefault();
  const title = captureInput.value.trim();
  if (!title) return;
  state.tasks.push({ id: `task-${++idSeed}`, title, deadlineOn: null, expanded: false, children: [] });
  captureInput.value = "";
  showToast(`已记下“${title}”`);
  render();
});

searchInput.addEventListener("input", () => {
  state.query = searchInput.value;
  renderTasks();
  renderHistory();
});

searchForm.addEventListener("submit", (event) => event.preventDefault());

taskList.addEventListener("submit", (event) => {
  const addForm = event.target.closest(".child-add-form");
  if (addForm) {
    event.preventDefault();
    const input = addForm.querySelector("input");
    submitChildAdd(input);
    return;
  }

  const editor = event.target.closest(".editor-panel");
  if (editor) {
    event.preventDefault();
    const parent = findParent(editor.dataset.parentEditor);
    if (!parent) return;
    const title = editor.elements.title.value.trim();
    if (!title) {
      editor.elements.title.focus();
      return;
    }
    parent.title = title;
    parent.deadlineOn = editor.elements.deadlineOn.value || null;
    state.editor = null;
    showToast("已保存代办修改");
    render();
  }
});

taskList.addEventListener("click", (event) => {
  const target = event.target.closest("[data-action]");
  if (!target) return;
  const { action, parentId, childId } = target.dataset;
  if (action === "toggle-parent") {
    const parent = findParent(parentId);
    if (parent) parent.expanded = !parent.expanded;
    render();
  } else if (action === "add-child") {
    startAddingChild(parentId);
  } else if (action === "complete-parent") {
    completeParent(parentId);
  } else if (action === "toggle-child") {
    toggleChild(parentId, childId);
  } else if (action === "delete-parent") {
    requestDeleteParent(parentId);
  } else if (action === "delete-child") {
    deleteChild(parentId, childId);
  } else if (action === "cancel-editor") {
    state.editor = null;
    render();
  }
});

taskList.addEventListener("dblclick", (event) => {
  const parentTitle = event.target.closest(".task-title");
  const childTitle = event.target.closest(".child-title");
  if (parentTitle) startParentEdit(parentTitle.dataset.parentId);
  else if (childTitle) startChildEdit(childTitle.dataset.parentId, childTitle.dataset.childId);
});

taskList.addEventListener("keydown", (event) => {
  const addInput = event.target.closest("[data-add-child-input]");
  if (addInput && event.key === "Enter" && !event.isComposing) {
    event.preventDefault();
    addInput.closest("form")?.requestSubmit();
    return;
  }
  if (addInput && event.key === "Escape") {
    event.preventDefault();
    state.addingParentId = null;
    render();
    return;
  }

  const parentTitle = event.target.closest(".task-title");
  const childTitle = event.target.closest(".child-title");
  if ((event.key === "Enter" || event.key === "F2") && parentTitle) {
    event.preventDefault();
    startParentEdit(parentTitle.dataset.parentId);
    return;
  }
  if ((event.key === "Enter" || event.key === "F2") && childTitle) {
    event.preventDefault();
    startChildEdit(childTitle.dataset.parentId, childTitle.dataset.childId);
    return;
  }

  const childEditor = event.target.closest(".inline-child-editor");
  if (childEditor && event.key === "Enter") {
    event.preventDefault();
    const child = findChild(childEditor.dataset.parentId, childEditor.dataset.childId);
    const title = childEditor.value.trim();
    if (child && title) child.title = title;
    state.editor = null;
    showToast("已保存子代办修改");
    render();
    return;
  }
  if (childEditor && event.key === "Escape") {
    event.preventDefault();
    state.editor = null;
    render();
    return;
  }

  const handle = event.target.closest(".drag-handle");
  if (handle && event.altKey && (event.key === "ArrowUp" || event.key === "ArrowDown")) {
    event.preventDefault();
    reorder(
      handle.dataset.dragKind,
      handle.dataset.parentId,
      handle.dataset.dragKind === "parent" ? handle.dataset.parentId : handle.dataset.childId,
      event.key === "ArrowUp" ? -1 : 1,
    );
  }

  const progress = event.target.closest(".progress-toggle");
  if (progress && (event.key === "ArrowLeft" || event.key === "ArrowRight")) {
    event.preventDefault();
    const parent = findParent(progress.dataset.parentId);
    if (parent) parent.expanded = event.key === "ArrowRight";
    render();
  }
});

taskList.addEventListener("focusout", (event) => {
  const addInput = event.target.closest("[data-add-child-input]");
  if (addInput) {
    submitChildAdd(addInput, { exitOnBlank: true });
    return;
  }
  const childEditor = event.target.closest(".inline-child-editor");
  if (!childEditor || !state.editor) return;
  const child = findChild(childEditor.dataset.parentId, childEditor.dataset.childId);
  const title = childEditor.value.trim();
  if (child && title) child.title = title;
  state.editor = null;
  render();
});

taskList.addEventListener("dragstart", (event) => {
  const handle = event.target.closest(".drag-handle");
  if (!handle || state.searching) {
    event.preventDefault();
    return;
  }
  state.dragged = {
    kind: handle.dataset.dragKind,
    parentId: handle.dataset.parentId,
    id: handle.dataset.dragKind === "parent" ? handle.dataset.parentId : handle.dataset.childId,
  };
  event.dataTransfer.effectAllowed = "move";
});

taskList.addEventListener("dragover", (event) => {
  if (state.dragged) event.preventDefault();
});

taskList.addEventListener("drop", (event) => {
  event.preventDefault();
  const dragged = state.dragged;
  if (!dragged) return;
  const targetParent = event.target.closest("[data-row-kind='parent']");
  const targetChild = event.target.closest("[data-row-kind='child']");
  if (dragged.kind === "parent" && targetParent) {
    moveBefore(state.tasks, dragged.id, targetParent.dataset.id);
  } else if (dragged.kind === "child" && targetChild && targetChild.dataset.parentId === dragged.parentId) {
    moveBefore(findParent(dragged.parentId).children, dragged.id, targetChild.dataset.childId);
  }
  state.dragged = null;
  render();
});

taskList.addEventListener("dragend", () => { state.dragged = null; });

historyList.addEventListener("click", (event) => {
  const target = event.target.closest("[data-action]");
  if (!target || target.disabled) return;
  if (target.dataset.action === "toggle-history-group") {
    const parentId = target.dataset.parentId;
    if (state.expandedHistoryIds.has(parentId)) state.expandedHistoryIds.delete(parentId);
    else state.expandedHistoryIds.add(parentId);
    renderHistory();
    queueMicrotask(() => {
      historyList.querySelector(`[data-action="toggle-history-group"][data-parent-id="${parentId}"]`)?.focus();
    });
  } else if (target.dataset.action === "undo-parent") undoParent(target.dataset.parentId);
  else if (target.dataset.action === "undo-child") undoChild(target.dataset.parentId, target.dataset.childId);
  else if (target.dataset.action === "undo-standalone") undoStandalone(target.dataset.entryId);
});

document.addEventListener("click", (event) => {
  const action = event.target.closest("[data-action]")?.dataset.action;
  if (action === "show-history") showHistory();
  else if (action === "show-tasks") showTasks();
  else if (action === "start-search") startSearch();
  else if (action === "cancel-search") cancelSearch();
  else if (action === "expand-window") {
    state.mode = "expanded";
    state.panel = "tasks";
    const parent = state.tasks[0];
    if (parent?.children.length) parent.expanded = true;
    render();
  }
});

document.querySelector("#capsuleAction").addEventListener("click", () => {
  state.mode = "capsule";
  render();
});

capsuleCheckbox.addEventListener("click", () => {
  const parentId = capsuleCheckbox.dataset.parentId;
  const childId = capsuleCheckbox.dataset.childId;
  if (childId) toggleChild(parentId, childId);
  else completeParent(parentId);
});

capsuleTitle.addEventListener("click", () => {
  state.mode = "expanded";
  const parent = state.tasks[0];
  if (parent?.children.length) parent.expanded = true;
  render();
});

deleteGroupDialog.addEventListener("close", () => {
  if (deleteGroupDialog.returnValue !== "confirm") {
    state.pendingDeleteId = null;
    return;
  }
  const parent = findParent(state.pendingDeleteId);
  if (!parent) return;
  if (parent.children.some((child) => child.completedAt)) state.deletedGroups.unshift(parent);
  state.tasks = state.tasks.filter((task) => task.id !== parent.id);
  state.pendingDeleteId = null;
  showToast(`已删除任务组“${parent.title}”`);
  render();
});

document.querySelectorAll(".scene-button").forEach((button) => {
  button.addEventListener("click", () => setScene(button.dataset.scene));
});

window.addEventListener("keydown", (event) => {
  if (event.ctrlKey && event.key.toLocaleLowerCase() === "f") {
    event.preventDefault();
    startSearch();
    return;
  }
  if (event.key === "Escape") {
    if (state.addingParentId) {
      state.addingParentId = null;
      render();
    } else if (state.editor) {
      state.editor = null;
      render();
    } else if (state.searching) {
      cancelSearch();
    } else if (state.panel === "history") {
      showTasks();
    }
  }
});

function moveBefore(list, movedId, targetId) {
  if (movedId === targetId) return;
  const from = list.findIndex((item) => item.id === movedId);
  const target = list.findIndex((item) => item.id === targetId);
  if (from < 0 || target < 0) return;
  const [moved] = list.splice(from, 1);
  const adjustedTarget = list.findIndex((item) => item.id === targetId);
  list.splice(adjustedTarget, 0, moved);
}

render();
