import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");

function read(relativePath) {
  return readFileSync(resolve(repoRoot, relativePath), "utf8");
}

const html = read("desktop/index.html");
const tauriConfig = JSON.parse(read("src-tauri/tauri.conf.json"));
const candidateConfig = JSON.parse(read("src-tauri/tauri.candidate.conf.json"));
const releaseWorkflow = read(".github/workflows/release.yml");
const traySource = read("src-tauri/src/desktop/tray.rs");
const appSource = read("src-tauri/src/app.rs");
const schemaSource = read("src-tauri/src/ledger/sqlite/schema.rs");
const outboxSource = read("desktop/app/infrastructure/outbox-store.js");
const packageJson = JSON.parse(read("package.json"));
const cargoToml = read("src-tauri/Cargo.toml");

test("面向用户的产品名称统一为待办", () => {
  assert.match(html, /<title>待办<\/title>/);
  assert.match(html, /<strong id="appTitle">待办<\/strong>/);
  assert.equal(tauriConfig.productName, "待办");
  assert.equal(candidateConfig.productName, "待办测试版");
  assert.equal(tauriConfig.app.windows.find(({ label }) => label === "main")?.title, "待办");
  assert.match(traySource, /"打开待办"/);
  assert.match(traySource, /"退出待办"/);
  assert.match(traySource, /\.tooltip\("待办"\)/);
  assert.match(releaseWorkflow, /releaseName:\s*待办 v__VERSION__/);
  assert.doesNotMatch([html, traySource, appSource, schemaSource].join("\n"), /做伴/);
});

test("改名不改变数据与恢复协议身份", () => {
  assert.equal(tauriConfig.identifier, "com.luoji.zuoban.spike");
  assert.equal(packageJson.name, "zuoban-desktop-spike");
  assert.match(cargoToml, /^name = "zuoban-desktop-spike"$/m);
  assert.match(appSource, /join\("zuoban-ledger\.sqlite3"\)/);
  assert.match(traySource, /with_id\("zuoban-main-tray"\)/);
  assert.match(outboxSource, /zuoban\.ledger\.pending-operation\.v1/);
});
