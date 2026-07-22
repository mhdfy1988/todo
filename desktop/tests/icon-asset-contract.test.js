import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");

function read(relativePath) {
  return readFileSync(resolve(repoRoot, relativePath), "utf8");
}

const iconSource = read("src-tauri/icons/source.svg");
const tauriConfig = JSON.parse(read("src-tauri/tauri.conf.json"));
const traySource = read("src-tauri/src/desktop/tray.rs");

test("桌面图标源保持透明且不绘制白色底板", () => {
  assert.match(iconSource, /^<svg[^>]+viewBox="0 0 512 512">/);
  assert.doesNotMatch(iconSource, /<rect\b/);
  assert.doesNotMatch(iconSource, /#f7f8f9/i);
  assert.match(iconSource, /scale\(1\.14\)/);
  assert.match(iconSource, /fill="#ef8b63"/);
});

test("托盘继续复用正式构建内嵌的默认图标", () => {
  assert.ok(tauriConfig.bundle.icon.includes("icons/icon.ico"));
  assert.match(traySource, /default_window_icon\(\)/);
  assert.match(traySource, /\.icon\(icon\)/);
});
