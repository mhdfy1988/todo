import test from "node:test";
import assert from "node:assert/strict";

import { WindowController } from "../app/window-controller.js";

test("原生窗口状态事件统一更新控制器和页面模式", async () => {
  let listener = null;
  const gateway = {
    subscribeWindowStatus: async (nextListener) => {
      listener = nextListener;
      return () => {};
    },
  };
  const root = { dataset: { mode: "capsule" } };
  const statusText = { textContent: "" };
  const controller = new WindowController({ gateway, root, statusText });

  await controller.subscribeToStatusChanges();
  listener({
    mode: "expanded",
    focused: true,
    inWorkArea: true,
  });

  assert.equal(controller.mode, "expanded");
  assert.equal(root.dataset.mode, "expanded");
  assert.equal(statusText.textContent, "置顶 · 当前有焦点 · 位置安全");
});
