import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

import { diagnosticsPassed, ledgerContentKind } from "../app/selectors.js";
import { createInitialState, LedgerPhase } from "../app/state.js";

test("index 只暴露任务主线，并保留必要的界面契约", async () => {
  const html = await readFile(new URL("../index.html", import.meta.url), "utf8");
  const styles = await readFile(new URL("../styles.css", import.meta.url), "utf8");
  const views = await readFile(new URL("../app/views.js", import.meta.url), "utf8");
  const app = await readFile(new URL("../app.js", import.meta.url), "utf8");
  const taskListController = await readFile(
    new URL("../app/task-list-controller.js", import.meta.url),
    "utf8",
  );
  const ids = [
    "desktopRoot",
    "captureForm",
    "taskTitle",
    "ledgerStatus",
    "ledgerStatusText",
    "retryButton",
    "taskList",
    "taskOrderStatus",
    "taskOrderHelp",
    "listSearchForm",
    "listSearchLabel",
    "listSearchInput",
    "listSearchCancel",
    "listSearchStatus",
    "searchAction",
    "updateAction",
    "appTitle",
    "historyLink",
    "historyOpenButton",
    "historyCount",
    "historyBackButton",
    "copyWeeklyCompletionsButton",
    "historyList",
    "capsuleTitleButton",
    "capsuleTaskTitle",
    "capsuleTaskProgress",
    "capsuleTaskDeadline",
    "capsuleTaskCheckbox",
    "moreMenu",
    "deleteGroupDialog",
    "deleteGroupDialogTitle",
    "toast",
  ];
  const actions = [
    "capsule",
    "complete-task",
    "copy-weekly-completions",
    "diagnostics",
    "edit-current-deadline",
    "expanded",
    "hide",
    "search",
    "update",
    "show-history",
    "show-tasks",
  ];

  ids.forEach((id) => assert.match(html, new RegExp(`id="${id}"`)));
  actions.forEach((action) => {
    assert.match(html, new RegExp(`data-action="${action}"`));
  });
  const windowBar = html.match(/<header class="window-bar"[\s\S]*?<\/header>/)?.[0];
  assert.ok(windowBar, "缺少顶部窗口标题栏");
  assert.match(
    windowBar,
    /id="historyBackButton"[\s\S]*?<strong id="historyTitle">已完成<\/strong>/,
    "已完成页的返回入口与标题应合并进顶部窗口标题栏",
  );
  const menu = windowBar.match(/<div class="menu-popover">[\s\S]*?<\/div>/)?.[0];
  assert.ok(menu, "缺少更多菜单");
  assert.match(
    menu,
    /id="copyWeeklyCompletionsButton"[^>]*>复制本周完成<\/button>/,
    "复制本周完成应收进更多菜单",
  );
  assert.match(
    styles,
    /body\[data-panel="history"\]\s+#copyWeeklyCompletionsButton\s*\{\s*display:\s*block;/,
    "复制本周完成只应在已完成页的更多菜单中出现",
  );
  const historyView = html.match(/<section class="panel-view history-view"[\s\S]*?<\/section>/)?.[0];
  assert.ok(historyView, "缺少已完成列表页");
  assert.doesNotMatch(historyView, /panel-heading|copyWeeklyCompletionsButton/);
  const historyLink = html.match(/<div id="historyLink"[\s\S]*?<\/div>/)?.[0];
  assert.ok(historyLink, "缺少待办页底部的已完成入口");
  assert.doesNotMatch(
    historyLink.match(/^<div[^>]*>/)?.[0] ?? "",
    /data-action=/,
    "已完成整行不能作为点击入口",
  );
  assert.match(
    historyLink,
    /id="historyOpenButton"[\s\S]*?data-action="show-history"[\s\S]*?<span>已完成<\/span>[\s\S]*?id="historyCount"/,
    "已完成、数量和箭头应组成右侧紧凑入口",
  );
  assert.match(
    styles,
    /\.history-link\s*\{[^}]*justify-content:\s*flex-end;/,
    "已完成标签应与数量和箭头组成靠右的一组",
  );
  const capsule = html.match(/<section class="surface capsule"[\s\S]*?<\/section>/)?.[0];
  assert.ok(capsule, "缺少当前任务胶囊");
  assert.ok(
    capsule.indexOf("capsuleTaskCheckbox") < capsule.indexOf("task-copy"),
    "完成圆圈应位于任务标题之前",
  );
  assert.match(views, /dataset\.action = "delete-task"/);
  assert.match(views, /undo\.dataset\.action = "undo-completion"/);
  assert.match(views, /undo\.dataset\.eventId = event\.id/);
  assert.match(views, /undo\.textContent = "↶"/);
  assert.match(views, /undo\.title = "撤销完成"/);
  assert.match(views, /undo\.setAttribute\("aria-label", `撤销完成：\$\{event\.titleSnapshot\}`\)/);
  assert.match(views, /copy\.append\(title, time\);\s*item\.append\(copy, undo\);/s);
  assert.doesNotMatch(views, /undo\.textContent = "撤销"/);
  assert.match(styles, /\.history-copy b\s*\{[^}]*flex:\s*1;[^}]*text-overflow:\s*ellipsis;/s);
  assert.match(styles, /\.history-copy time\s*\{[^}]*margin-left:\s*auto;[^}]*white-space:\s*nowrap;/s);
  assert.match(styles, /\.history-window-title\s*\{[^}]*pointer-events:\s*auto;/s);
  assert.doesNotMatch(styles, /\.panel-heading|\.heading-text-action/);
  assert.match(styles, /\.history-list button\s*\{[^}]*width:\s*28px;[^}]*height:\s*28px;[^}]*border:\s*0;/s);
  assert.match(styles, /\.capture-form button\s*\{[^}]*color:\s*var\(--soft\);[^}]*font-weight:\s*600;/s);
  assert.match(styles, /\.capture-form button:not\(:disabled\):hover\s*\{[^}]*background:\s*var\(--wash\);[^}]*color:\s*var\(--ink\);/s);
  assert.doesNotMatch(styles, /\.capture-form button\s*\{[^}]*background:\s*var\(--dark\)/s);
  assert.match(
    app,
    /case "copy-weekly-completions":\s*shellController\.closeMenu\(\);\s*await weeklyCompletionController\.copyCurrentWeek\(\);\s*return;/s,
  );
  assert.doesNotMatch(app, /button\.checked = false|incompleteCount|revealFirstPending\(taskId/);
  assert.match(app, /const operation = await session\.completeTask\(taskId\);/);
  assert.match(app, /const ledgerBoundReadAction = action === "copy-weekly-completions";/);
  assert.match(
    app,
    /button\.disabled = ledgerBoundReadAction \? !session\.canMutate\(\) : false;/,
  );
  assert.match(styles, /\.panel-body\s*\{[^}]*overflow:\s*hidden;/s);
  assert.doesNotMatch(styles, /\.panel-body\s*\{[^}]*overflow-y:\s*auto;/s);
  assert.match(styles, /\.task-list\s*\{[^}]*min-height:\s*0;[^}]*flex:\s*1;[^}]*overflow-y:\s*auto;[^}]*scrollbar-gutter:\s*stable;/s);
  assert.match(styles, /\.history-list\s*\{[^}]*min-height:\s*0;[^}]*flex:\s*1;[^}]*overflow-y:\s*auto;[^}]*scrollbar-gutter:\s*stable;/s);
  assert.match(styles, /\.task-list::\-webkit-scrollbar,\s*\.history-list::\-webkit-scrollbar\s*\{\s*width:\s*7px;\s*\}/s);
  assert.match(styles, /\.task-list::\-webkit-scrollbar-button,\s*\.history-list::\-webkit-scrollbar-button\s*\{[^}]*display:\s*none;[^}]*width:\s*0;[^}]*height:\s*0;/s);
  assert.match(views, /const title = this\.document\.createElement\("button"\)/);
  assert.match(views, /title\.dataset\.taskId = task\.id/);
  assert.doesNotMatch(views, /title\.dataset\.action/);
  assert.match(taskListController, /input\.maxLength = MAX_TASK_TITLE_LENGTH/);
  assert.match(taskListController, /deadlineInput\.type = "date"/);
  assert.match(taskListController, /deadlineLabel\.htmlFor = deadlineInput\.id/);
  assert.match(taskListController, /className = "task-deadline-edit-row"/);
  assert.match(styles, /\.task-row\.is-editing \.task-deadline\s*\{\s*display:\s*none;\s*\}/);
  assert.match(taskListController, /event\.key === "F2"/);
  assert.doesNotMatch(html, /双击修改|修改按钮/);
  assert.doesNotMatch(html, /截止日期|无期限|\+期限/);
  assert.match(views, /if \(deadline\) \{/);
  assert.match(views, /className = `task-deadline is-\$\{deadline\.state\}`/);
  assert.match(views, /refreshDeadlineLabels\(todayOn = localDateOnly\(\)\)/);
  assert.match(html, /<script type="module" src="\.\/app\.js"><\/script>/);
  assert.match(html, /id="capsuleTaskCheckbox"[^>]+type="checkbox"/);
  assert.match(html, /id="capsuleTaskCheckbox"[^>]+hidden disabled/);
  assert.match(html, /class="capsule-hide-button"[^>]+data-action="hide"[^>]+aria-label="隐藏到托盘"/);
  assert.match(html, /class="capsule-hide-button"[^>]*>[\s\S]*?<span class="capsule-hide-icon" aria-hidden="true"><\/span>[\s\S]*?<\/button>/);
  assert.match(styles, /\.task-copy\s*\{[^}]*display:\s*flex;[^}]*align-items:\s*center;[^}]*gap:\s*8px;/s);
  assert.match(styles, /\.capsule-title-button\s*\{[^}]*flex:\s*1;[^}]*width:\s*auto;/s);
  assert.match(styles, /\.capsule-deadline\s*\{[^}]*min-height:\s*24px;[^}]*display:\s*flex;[^}]*flex:\s*0 0 auto;[^}]*margin:\s*0;[^}]*text-align:\s*right;/s);
  assert.match(styles, /\.capsule-hide-button\s*\{[^}]*position:\s*absolute;[^}]*top:\s*6px;[^}]*right:\s*7px;/s);
  assert.match(styles, /\.capsule-hide-icon\s*\{[^}]*border-right:\s*1px solid currentColor;[^}]*border-bottom:\s*1px solid currentColor;[^}]*transform:\s*rotate\(45deg\);/s);
  assert.equal(html.match(/data-action="hide"/g)?.length, 2);
  assert.match(views, /current\?\.title \?\? "暂无待办"/);
  assert.match(views, /current \? `当前待办：\$\{actionTitle\}，展开任务面板` : "暂无待办，展开任务面板"/);
  assert.match(views, /capsuleTaskCheckbox\.hidden = !action/);
  assert.match(html, /id="capsuleTaskDeadline"[^>]+data-action="edit-current-deadline"/);
  assert.match(views, /#renderCapsuleDeadline\(current\?\.deadlineOn \?\? null\)/);
  assert.match(views, /this\.capsuleTaskDeadline\.hidden = !deadline/);
  assert.match(views, /deadline \? `修改\$\{deadline\.title\}` : "当前任务没有截止日期"/);
  assert.match(taskListController, /beginDeadlineEdit\(taskId\)/);
  assert.equal(html.match(/data-pet/g)?.length, 2);
  assert.equal(html.match(/class="ear left"/g)?.length, 1);
  [
    "桌面实验",
    "桌面悬浮窗实验",
    "pendingCount",
    "completedCount",
    "coinBalance",
    "currentTaskMeta",
    "按记录顺序",
    "保留历史，可撤销",
    "experiment-bar",
    "pet-panel",
    "pet-capsule",
    "现在做",
    "接下来",
    "completeTaskButton",
    "currentCard",
    "queueSection",
    "pass-through",
    "开启鼠标穿透",
  ].forEach((copy) => assert.doesNotMatch(html, new RegExp(copy)));

  // 保留浅色桌面上的必要轮廓，但不能让外阴影进入透明圆角。
  for (const selector of ["expanded", "capsule"]) {
    const surfaceRule = styles.match(new RegExp(`\\.${selector}\\s*\\{([^}]*)\\}`, "s"));
    assert.ok(surfaceRule, `缺少 .${selector} 样式规则`);
    assert.match(surfaceRule[1], /\bborder\s*:\s*1px\s+solid\s+rgba\(/);
    assert.doesNotMatch(surfaceRule[1], /\bbox-shadow\s*:/);
  }
});

test("列表搜索保持独立输入、纯过滤与可访问快捷键契约", async () => {
  const html = await readFile(new URL("../index.html", import.meta.url), "utf8");
  const styles = await readFile(new URL("../styles.css", import.meta.url), "utf8");
  const views = await readFile(new URL("../app/views.js", import.meta.url), "utf8");
  const app = await readFile(new URL("../app.js", import.meta.url), "utf8");

  const desktopRoot = html.match(/<main\s+id="desktopRoot"[^>]*>/)?.[0];
  assert.ok(desktopRoot, "缺少桌面主容器");
  assert.doesNotMatch(desktopRoot, /aria-live=/, "主容器不能把每次列表重绘都作为通用播报");

  const searchAction = html.match(/<button\s+id="searchAction"[^>]*>/)?.[0];
  assert.ok(searchAction, "缺少更多菜单中的搜索入口");
  assert.match(searchAction, /data-action="search"/);
  assert.match(searchAction, /aria-keyshortcuts="Control\+F"/);

  const searchForm = html.match(/<form\s+id="listSearchForm"[\s\S]*?<\/form>/)?.[0];
  assert.ok(searchForm, "缺少待办与已完成共享的搜索表单");
  assert.match(searchForm, /role="search"/);
  assert.match(searchForm, /id="listSearchLabel"[^>]+for="listSearchInput"/);
  assert.match(searchForm, /id="listSearchInput"[^>]+type="search"/);
  assert.match(searchForm, /id="listSearchInput"[^>]+aria-controls="taskList"/);
  assert.match(searchForm, /id="listSearchInput"[^>]+aria-describedby="listSearchStatus"/);
  assert.match(searchForm, /id="listSearchCancel"[^>]+type="button"/);
  assert.doesNotMatch(searchForm, /id="taskTitle"/, "搜索不能复用新增待办输入框");
  assert.equal(html.match(/id="listSearchForm"/g)?.length, 1, "两页应共享一份独立搜索表单");

  const captureForm = html.match(/<form\s+id="captureForm"[\s\S]*?<\/form>/)?.[0];
  assert.ok(captureForm, "缺少新增待办表单");
  assert.match(captureForm, /id="taskTitle"/);
  assert.doesNotMatch(captureForm, /id="listSearchInput"/);

  const searchStatus = searchForm.match(/<p\s+id="listSearchStatus"[^>]*>/)?.[0];
  assert.ok(searchStatus, "缺少搜索结果专用状态播报区");
  assert.match(searchStatus, /role="status"/);
  assert.match(searchStatus, /aria-live="polite"/);
  assert.match(searchStatus, /aria-atomic="true"/);

  assert.match(views, /filterTaskGroupsByTitle\(groups, query\)/);
  assert.match(views, /filterCompletionGroupsByTitle\(groups, query\)/);
  assert.match(views, /document\.createTextNode\(/);
  assert.match(views, /document\.createElement\("mark"\)/);
  assert.match(views, /mark\.className = "search-match"/);
  assert.match(views, /mark\.textContent = text\.slice\(start, end\)/);
  assert.doesNotMatch(views, /innerHTML\s*=/, "搜索高亮必须使用文本节点，不能拼接 HTML");
  assert.match(views, /this\.#renderTasks\(snapshot\.queue, current\?\.id \?\? null\)/);
  assert.match(views, /if \(task\.id === currentTaskId\) item\.setAttribute\("aria-current", "step"\)/);
  assert.doesNotMatch(views, /if \(index === 0\) item\.setAttribute\("aria-current", "step"\)/);
  assert.match(views, /handle\.hidden = searching;\s*handle\.disabled = searching;\s*handle\.draggable = !searching;/s);
  assert.match(views, /const reorderUnavailable = unavailable \|\| this\.searchState\.panel === "tasks"/);
  assert.match(views, /this\.copyWeeklyCompletionsButton = required\(document, "#copyWeeklyCompletionsButton"\);/);
  assert.match(views, /this\.copyWeeklyCompletionsButton\.disabled = unavailable;/);
  assert.match(views, /handle\.disabled = reorderUnavailable;\s*handle\.draggable = !reorderUnavailable;/s);

  assert.match(
    app,
    /if \(event\.isComposing \|\| event\.keyCode === 229 \|\| searchController\.isComposing\(\)\) return;/,
  );
  assert.match(
    app,
    /if \(isSearchShortcut\(event\)\) \{\s*event\.preventDefault\(\);/s,
    "Ctrl+F 必须阻止 WebView 原生页面查找条",
  );
  assert.match(
    app,
    /case "search":\s*shellController\.closeMenu\(\);\s*searchController\.open\(/s,
    "从更多菜单进入搜索前必须关闭菜单",
  );
  assert.doesNotMatch(
    styles,
    /body\[data-search-panel="tasks"\]\s+\.(?:task-row|subtask-row)\s*\{[^}]*grid-template-columns:/s,
    "搜索态应复用统一的三列布局，不能复制一套网格列定义",
  );
});

test("五种账本阶段统一选择加载、快照、错误或恢复内容", () => {
  const initial = createInitialState();
  assert.equal(ledgerContentKind(initial), "loading");
  assert.equal(
    ledgerContentKind({ ...initial, phase: LedgerPhase.ERROR }),
    "error",
  );
  assert.equal(
    ledgerContentKind({ ...initial, phase: LedgerPhase.RECOVERY }),
    "recovery",
  );
  for (const phase of [LedgerPhase.READY, LedgerPhase.BUSY, LedgerPhase.RECOVERY]) {
    assert.equal(
      ledgerContentKind({ ...initial, phase, snapshotReady: true }),
      "snapshot",
    );
  }
});

test("窗口与账本诊断使用完整且统一的通过口径", () => {
  const status = {
    inWorkArea: true,
    alwaysOnTop: true,
    trayReady: true,
  };
  const integrity = {
    sqliteQuickCheck: true,
    foreignKeys: true,
    rewardPrefixBalances: true,
    eventRewardLinks: true,
    receiptLinks: true,
    taskRewardNetValues: true,
    taskProjectionMatchesLedger: true,
    taskHierarchyValid: true,
    failures: [],
  };

  assert.equal(diagnosticsPassed(status, integrity), true);
  assert.equal(diagnosticsPassed({ ...status, trayReady: false }, integrity), false);
  assert.equal(
    diagnosticsPassed(status, { ...integrity, taskRewardNetValues: false }),
    false,
  );
  assert.equal(
    diagnosticsPassed(status, { ...integrity, taskHierarchyValid: false }),
    false,
  );
});
