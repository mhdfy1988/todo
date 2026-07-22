import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");

function read(relativePath) {
  return readFileSync(resolve(repoRoot, relativePath), "utf8");
}

const packageJson = JSON.parse(read("package.json"));
const tauriConfig = JSON.parse(read("src-tauri/tauri.conf.json"));
const cargoToml = read("src-tauri/Cargo.toml");
const workflow = read(".github/workflows/release.yml");
const capability = JSON.parse(read("src-tauri/capabilities/default.json"));

test("三个发布版本和 Windows NSIS 目标保持一致", () => {
  const cargoVersion = cargoToml.match(/^version\s*=\s*"([^"]+)"$/m)?.[1];
  assert.equal(packageJson.version, tauriConfig.version);
  assert.equal(packageJson.version, cargoVersion);
  assert.equal(tauriConfig.bundle.active, true);
  assert.deepEqual(tauriConfig.bundle.targets, ["nsis"]);
  assert.equal(tauriConfig.bundle.createUpdaterArtifacts, true);
  assert.equal(tauriConfig.bundle.windows.nsis.installMode, "currentUser");
  assert.equal(packageJson.scripts["desktop:build"], "tauri build --no-bundle");
  assert.equal(packageJson.scripts["desktop:bundle:windows"], "tauri build --bundles nsis");
});

test("更新源、公钥和 Rust 插件进入首个安装版", () => {
  const updater = tauriConfig.plugins.updater;
  assert.deepEqual(updater.endpoints, [
    "https://github.com/mhdfy1988/todo/releases/latest/download/latest.json",
  ]);
  assert.match(updater.pubkey, /^[A-Za-z0-9+/=\r\n]+$/);
  assert.ok(updater.pubkey.length > 100);
  assert.equal(updater.windows.installMode, "passive");
  assert.match(cargoToml, /^tauri-plugin-updater = "2\.10\.1"$/m);
});

test("发布工作流先过门禁，再上传安装器、签名和 latest.json", () => {
  assert.match(workflow, /^\s*permissions:\s*\n\s*contents: write/m);
  assert.match(workflow, /runs-on: windows-latest/);
  assert.match(workflow, /npm\.cmd run release:version:check/);
  assert.match(workflow, /npm\.cmd run desktop:check/);
  assert.match(workflow, /手动发布只能从 main 分支执行/);
  assert.equal(workflow.match(/shell: pwsh/g)?.length, 3);
  assert.doesNotMatch(workflow, /shell: powershell/);
  assert.match(workflow, /tauri-apps\/tauri-action@v1/);
  assert.match(workflow, /TAURI_SIGNING_PRIVATE_KEY: \$\{\{ secrets\.TAURI_SIGNING_PRIVATE_KEY \}\}/);
  assert.match(workflow, /TAURI_SIGNING_PRIVATE_KEY_PASSWORD: \$\{\{ secrets\.TAURI_SIGNING_PRIVATE_KEY_PASSWORD \}\}/);
  assert.match(workflow, /uploadUpdaterJson: true/);
  assert.match(workflow, /updaterJsonPreferNsis: true/);
  assert.match(workflow, /uploadUpdaterSignatures: true/);
  assert.match(workflow, /args: --bundles nsis/);
  assert.match(workflow, /releaseDraft: true/);
  assert.match(workflow, /--example verify_updater_signature/);
  assert.match(workflow, /Invoke-WebRequest -Uri \$downloadUrl/);
  assert.match(workflow, /Get-FileHash -LiteralPath \$downloadedInstaller/);
  assert.match(workflow, /gh release edit .*--draft=false --latest/);
});

test("前端只调用项目命令，不直接扩大 updater 插件权限", () => {
  assert.deepEqual(capability.permissions, ["core:default"]);
});
