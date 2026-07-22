import assert from "node:assert/strict";
import test from "node:test";

import { UpdateController } from "../app/update-controller.js";

test("normal 就绪后自动检查并把可用版本收进单一菜单入口", async () => {
  const fixture = createFixture({ availableVersion: "0.1.1" });

  await fixture.controller.start("normal", fixture.session);

  assert.equal(fixture.gateway.checks, 1);
  assert.equal(fixture.button.hidden, false);
  assert.equal(fixture.button.textContent, "安装更新 v0.1.1");
  assert.deepEqual(fixture.messages, ["发现新版本 v0.1.1"]);
  assert.equal(fixture.timer.intervals.length, 1);
});

test("smoke 隐藏更新入口且绝不访问网络", async () => {
  const fixture = createFixture({ availableVersion: "0.1.1" });

  await fixture.controller.start("smoke", fixture.session);

  assert.equal(fixture.gateway.checks, 0);
  assert.equal(fixture.button.hidden, true);
  assert.equal(fixture.button.disabled, true);
  assert.equal(fixture.timer.intervals.length, 0);
});

test("手动检查在没有新版本时给出简短反馈", async () => {
  const fixture = createFixture({ availableVersion: null });
  await fixture.controller.start("normal", fixture.session);
  fixture.messages.length = 0;

  await fixture.controller.handleAction();

  assert.equal(fixture.gateway.checks, 2);
  assert.deepEqual(fixture.messages, ["已是最新版本"]);
  assert.equal(fixture.button.textContent, "检查更新");
});

test("安装期间锁住桌面交互并提交刚检查到的版本", async () => {
  const fixture = createFixture({ availableVersion: "0.1.1" });
  await fixture.controller.start("normal", fixture.session);

  await fixture.controller.handleAction();

  assert.deepEqual(fixture.gateway.installs, ["0.1.1"]);
  assert.equal(fixture.root.inert, false);
  assert.equal(fixture.root.attributes.has("aria-busy"), false);
  assert.match(fixture.messages.at(-1), /完成后会自动重启/);
});

test("账本仍在操作或恢复时拒绝安装", async () => {
  const fixture = createFixture({ availableVersion: "0.1.1", canMutate: false });
  await fixture.controller.start("normal", fixture.session);

  await assert.rejects(
    fixture.controller.handleAction(),
    /请先等当前待办操作完成/,
  );
  assert.deepEqual(fixture.gateway.installs, []);
});

function createFixture({ availableVersion, canMutate = true }) {
  const button = { disabled: false, hidden: false, textContent: "" };
  const root = {
    inert: false,
    attributes: new Map(),
    setAttribute(name, value) { this.attributes.set(name, value); },
    removeAttribute(name) { this.attributes.delete(name); },
  };
  const messages = [];
  const timer = {
    intervals: [],
    cleared: [],
    setInterval(callback, delay) {
      this.intervals.push({ callback, delay });
      return this.intervals.length;
    },
    clearInterval(id) { this.cleared.push(id); },
  };
  const gateway = {
    checks: 0,
    installs: [],
    async checkForUpdate() {
      this.checks += 1;
      return { currentVersion: "0.1.0", availableVersion };
    },
    async installUpdate(version) { this.installs.push(version); },
  };
  const session = { canMutate: () => canMutate };
  const controller = new UpdateController({
    gateway,
    actionButton: button,
    root,
    toast: { show: (message) => messages.push(message) },
    timerHost: timer,
  });
  return { button, controller, gateway, messages, root, session, timer };
}
