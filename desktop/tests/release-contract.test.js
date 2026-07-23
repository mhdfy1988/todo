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

function readWorkflowStep(name) {
  const marker = `      - name: ${name}`;
  const start = workflow.indexOf(marker);
  assert.notEqual(start, -1, `缺少发布步骤：${name}`);
  const next = workflow.indexOf("\n      - name: ", start + marker.length);
  return workflow.slice(start, next === -1 ? workflow.length : next);
}

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
  assert.match(cargoToml, /^tauri-plugin-clipboard-manager = "2\.3\.2"$/m);
  assert.match(cargoToml, /^tauri-plugin-updater = "2\.10\.1"$/m);
});

test("发布工作流先过门禁，再上传安装器、签名和 latest.json", () => {
  assert.match(workflow, /^\s*permissions:\s*\n\s*contents: write/m);
  assert.match(workflow, /runs-on: windows-latest/);
  assert.match(workflow, /npm\.cmd run release:version:check/);
  assert.match(workflow, /npm\.cmd run release:changelog:github-output/);
  assert.match(workflow, /npm\.cmd run desktop:check/);
  assert.match(workflow, /手动发布只能从 main 分支执行/);
  for (const name of [
    "限制手动发布来源",
    "读取标准更新日志",
    "拒绝覆盖已发布版本",
    "验证更新签名与 latest.json",
    "核对草稿 Release 更新日志",
    "发布已验证的 Release",
  ]) {
    assert.match(readWorkflowStep(name), /^\s+shell: pwsh$/m);
  }
  assert.doesNotMatch(workflow, /shell: powershell/);
  assert.match(workflow, /拒绝覆盖已发布版本/);
  assert.match(workflow, /SkipHttpErrorCheck/);
  assert.match(workflow, /禁止覆盖既有安装器和更新元数据/);
  assert.match(workflow, /tauri-apps\/tauri-action@v1/);
  assert.match(workflow, /TAURI_SIGNING_PRIVATE_KEY: \$\{\{ secrets\.TAURI_SIGNING_PRIVATE_KEY \}\}/);
  assert.match(workflow, /TAURI_SIGNING_PRIVATE_KEY_PASSWORD: \$\{\{ secrets\.TAURI_SIGNING_PRIVATE_KEY_PASSWORD \}\}/);
  assert.match(workflow, /uploadUpdaterJson: true/);
  assert.match(workflow, /updaterJsonPreferNsis: true/);
  assert.match(workflow, /uploadUpdaterSignatures: true/);
  assert.match(workflow, /args: --bundles nsis/);
  assert.match(workflow, /releaseDraft: true/);
  assert.match(workflow, /releaseBody: \$\{\{ steps\.changelog\.outputs\.releaseBody \}\}/);
  assert.match(workflow, /generateReleaseNotes: false/);
  assert.doesNotMatch(workflow, /generateReleaseNotes: true/);
  assert.match(workflow, /--example verify_updater_signature/);
  assert.match(workflow, /Invoke-WebRequest -Uri \$downloadUrl/);
  assert.match(workflow, /Get-FileHash -LiteralPath \$downloadedInstaller/);
  assert.match(workflow, /EXPECTED_RELEASE_BODY: \$\{\{ steps\.changelog\.outputs\.releaseBody \}\}/);
  const draftCheck = readWorkflowStep("核对草稿 Release 更新日志");
  assert.match(draftCheck, /id: draft_release/);
  assert.match(draftCheck, /releases\?per_page=100/);
  assert.match(draftCheck, /Where-Object \{ \$_.tag_name -eq \$tag \}/);
  assert.doesNotMatch(draftCheck, /releases\/tags\//);
  assert.match(workflow, /草稿 Release 正文与 CHANGELOG 当前版本段不一致/);
  assert.match(draftCheck, /releaseId=\$\(\$release\.id\)/);
  const publish = readWorkflowStep("发布已验证的 Release");
  assert.match(publish, /steps\.draft_release\.outputs\.releaseId/);
  assert.match(publish, /gh api --method PATCH/);
  assert.match(publish, /releases\/\$releaseId/);
  assert.match(publish, /-F draft=false/);
  assert.match(publish, /-f make_latest=true/);
  assert.doesNotMatch(publish, /gh release edit/);
});

test("前端只开放核心能力和剪贴板写文本权限", () => {
  assert.deepEqual(capability.permissions, [
    "core:default",
    "clipboard-manager:allow-write-text",
  ]);
  assert.equal(capability.permissions.some((permission) => permission.includes("read")), false);
  assert.equal(capability.permissions.some((permission) => permission.includes("clear")), false);
});
