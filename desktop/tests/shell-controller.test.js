import test from "node:test";
import assert from "node:assert/strict";

import { ShellController } from "../app/shell-controller.js";

test("界面壳在待办与完成记录之间切换", () => {
  const root = { dataset: {} };
  const menu = { open: true };
  const search = createSearchHarness();
  const shell = new ShellController({ root, menu, search });

  assert.equal(root.dataset.panel, "tasks");
  assert.deepEqual(search.calls, [
    ["close", { restoreFocus: false }],
    ["set-panel", "tasks"],
  ]);

  search.calls.length = 0;
  shell.showHistory();
  assert.equal(root.dataset.panel, "history");
  assert.equal(menu.open, false);
  assert.deepEqual(search.calls, [
    ["close", { restoreFocus: false }],
    ["set-panel", "history"],
  ]);

  search.calls.length = 0;
  shell.showTasks();
  assert.equal(root.dataset.panel, "tasks");
  assert.deepEqual(search.calls, [
    ["close", { restoreFocus: false }],
    ["set-panel", "tasks"],
  ]);
});

test("Esc 依次关闭更多菜单、搜索、完成记录，再交还窗口控制器", () => {
  const root = { dataset: {} };
  const menu = { open: false };
  const search = createSearchHarness();
  const shell = new ShellController({ root, menu, search });

  shell.showHistory();
  search.open();
  menu.open = true;

  assert.equal(shell.closeTransientUi(), true);
  assert.equal(menu.open, false);
  assert.equal(search.isOpen(), true);
  assert.equal(root.dataset.panel, "history");

  assert.equal(shell.closeTransientUi(), true);
  assert.equal(search.isOpen(), false);
  assert.equal(root.dataset.panel, "history");

  assert.equal(shell.closeTransientUi(), true);
  assert.equal(root.dataset.panel, "tasks");

  assert.equal(shell.closeTransientUi(), false);
});

function createSearchHarness() {
  let open = false;
  const calls = [];
  return {
    calls,
    close(options) {
      calls.push(["close", options]);
      const wasOpen = open;
      open = false;
      return wasOpen;
    },
    setPanel(panel) {
      calls.push(["set-panel", panel]);
    },
    open() {
      open = true;
    },
    isOpen() {
      return open;
    },
  };
}
