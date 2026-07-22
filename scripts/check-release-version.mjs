import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const packageJson = JSON.parse(read("package.json"));
const tauriConfig = JSON.parse(read("src-tauri/tauri.conf.json"));
const cargoToml = read("src-tauri/Cargo.toml");
const cargoVersion = cargoToml.match(/^version\s*=\s*"([^"]+)"$/m)?.[1];
const versions = new Map([
  ["package.json", packageJson.version],
  ["src-tauri/tauri.conf.json", tauriConfig.version],
  ["src-tauri/Cargo.toml", cargoVersion],
]);
const expectedVersion = packageJson.version;

for (const [source, version] of versions) {
  if (version !== expectedVersion) {
    throw new Error(`发布版本不一致：${source} 为 ${version ?? "未设置"}，期望 ${expectedVersion}`);
  }
}

if (process.env.GITHUB_REF_TYPE === "tag") {
  const expectedTag = `v${expectedVersion}`;
  if (process.env.GITHUB_REF_NAME !== expectedTag) {
    throw new Error(`发布标签不一致：当前为 ${process.env.GITHUB_REF_NAME}，期望 ${expectedTag}`);
  }
}

console.log(`发布版本检查通过：v${expectedVersion}`);

function read(relativePath) {
  return readFileSync(resolve(root, relativePath), "utf8");
}
