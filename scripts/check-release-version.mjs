import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(import.meta.dirname, "..");
const cargoPackageName = "zuoban-desktop-spike";

export function collectReleaseVersions({
  packageJson,
  packageLock,
  tauriConfig,
  cargoToml,
  cargoLock,
}) {
  const cargoVersion = cargoToml.match(/^version\s*=\s*"([^"]+)"$/m)?.[1];
  const cargoLockVersion = findCargoLockPackageVersion(
    cargoLock,
    cargoPackageName,
  );

  return new Map([
    ["package.json", packageJson.version],
    ["package-lock.json#version", packageLock.version],
    ['package-lock.json#packages[""].version', packageLock.packages?.[""]?.version],
    ["src-tauri/tauri.conf.json", tauriConfig.version],
    ["src-tauri/Cargo.toml", cargoVersion],
    [`src-tauri/Cargo.lock#package:${cargoPackageName}`, cargoLockVersion],
  ]);
}

export function findCargoLockPackageVersion(cargoLock, packageName) {
  const packageBlocks = cargoLock
    .split(/^\[\[package\]\][ \t]*\r?$/m)
    .slice(1);
  const packageBlock = packageBlocks.find(
    (block) => block.match(/^name\s*=\s*"([^"]+)"$/m)?.[1] === packageName,
  );

  return packageBlock?.match(/^version\s*=\s*"([^"]+)"$/m)?.[1];
}

export function assertReleaseVersions(versions, expectedVersion) {
  for (const [source, version] of versions) {
    if (version !== expectedVersion) {
      throw new Error(
        `发布版本不一致：${source} 为 ${version ?? "未设置"}，期望 ${expectedVersion}`,
      );
    }
  }
}

function main() {
  const packageJson = JSON.parse(read("package.json"));
  const versions = collectReleaseVersions({
    packageJson,
    packageLock: JSON.parse(read("package-lock.json")),
    tauriConfig: JSON.parse(read("src-tauri/tauri.conf.json")),
    cargoToml: read("src-tauri/Cargo.toml"),
    cargoLock: read("src-tauri/Cargo.lock"),
  });
  const expectedVersion = packageJson.version;

  assertReleaseVersions(versions, expectedVersion);

  if (process.env.GITHUB_REF_TYPE === "tag") {
    const expectedTag = `v${expectedVersion}`;
    if (process.env.GITHUB_REF_NAME !== expectedTag) {
      throw new Error(
        `发布标签不一致：当前为 ${process.env.GITHUB_REF_NAME}，期望 ${expectedTag}`,
      );
    }
  }

  console.log(`发布版本检查通过：v${expectedVersion}`);
}

function read(relativePath) {
  return readFileSync(resolve(root, relativePath), "utf8");
}

const currentFile = resolve(fileURLToPath(import.meta.url));
const entryFile = process.argv[1] ? resolve(process.argv[1]) : "";
if (currentFile.toLowerCase() === entryFile.toLowerCase()) {
  main();
}
