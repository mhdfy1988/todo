import { randomUUID } from "node:crypto";
import { appendFileSync, readFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

export const STANDARD_CATEGORIES = [
  "Added",
  "Changed",
  "Deprecated",
  "Removed",
  "Fixed",
  "Security",
];

const VERSION_PATTERN = /^\d+\.\d+\.\d+$/;
const HEADING_PATTERN =
  /^## \[([^\]]+)\](?: - (\d{4}-\d{2}-\d{2}))?[ \t]*$/gm;

function escapePattern(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function isIsoDate(value) {
  const parsed = new Date(`${value}T00:00:00.000Z`);
  return !Number.isNaN(parsed.valueOf()) && parsed.toISOString().slice(0, 10) === value;
}

function compareVersions(left, right) {
  const leftParts = left.split(".").map(Number);
  const rightParts = right.split(".").map(Number);
  for (let index = 0; index < leftParts.length; index += 1) {
    if (leftParts[index] !== rightParts[index]) {
      return leftParts[index] - rightParts[index];
    }
  }
  return 0;
}

function validateCategories(body, sectionName, allowEmpty) {
  if (!body) {
    if (allowEmpty) {
      return;
    }
    throw new Error(`CHANGELOG 的 ${sectionName} 版本段不能为空`);
  }

  const headings = [
    ...body.matchAll(/^### ([^\r\n]+?)[ \t]*$/gm),
  ];
  if (headings.length === 0) {
    throw new Error(`CHANGELOG 的 ${sectionName} 版本段缺少标准分类`);
  }
  if (body.slice(0, headings[0].index).trim()) {
    throw new Error(`CHANGELOG 的 ${sectionName} 版本段存在未归类内容`);
  }

  let previousCategoryIndex = -1;
  for (let index = 0; index < headings.length; index += 1) {
    const category = headings[index][1];
    const categoryIndex = STANDARD_CATEGORIES.indexOf(category);
    if (categoryIndex === -1) {
      throw new Error(
        `CHANGELOG 的 ${sectionName} 使用了非标准分类：${category}`,
      );
    }
    if (categoryIndex <= previousCategoryIndex) {
      throw new Error(
        `CHANGELOG 的 ${sectionName} 分类顺序或唯一性不符合标准：${category}`,
      );
    }

    const contentStart = headings[index].index + headings[index][0].length;
    const contentEnd =
      index + 1 < headings.length ? headings[index + 1].index : body.length;
    const categoryBody = body.slice(contentStart, contentEnd).trim();
    if (!/^- \S.+$/m.test(categoryBody)) {
      throw new Error(
        `CHANGELOG 的 ${sectionName}/${category} 至少需要一条列表内容`,
      );
    }
    previousCategoryIndex = categoryIndex;
  }
}

export function parseChangelog(markdown) {
  const content = markdown.replace(/\r\n?/g, "\n");
  if (!content.startsWith("# Changelog")) {
    throw new Error("CHANGELOG.md 必须以“# Changelog”开头");
  }

  const linkBlockMatch = content.match(/^\[Unreleased\]:\s+\S+/m);
  if (!linkBlockMatch) {
    throw new Error("CHANGELOG.md 缺少 Unreleased 比较链接");
  }
  const linkBlockStart = linkBlockMatch.index;
  const matches = [...content.matchAll(HEADING_PATTERN)];
  if (matches.length < 2 || matches[0][1] !== "Unreleased") {
    throw new Error("CHANGELOG.md 顶部必须保留 Unreleased，且至少包含一个已发布版本");
  }
  if (matches[0][2]) {
    throw new Error("Unreleased 段不能填写发布日期");
  }

  const seen = new Set();
  const sections = matches.map((match, index) => {
    const name = match[1];
    const date = match[2] ?? null;
    if (seen.has(name)) {
      throw new Error(`CHANGELOG.md 存在重复版本段：${name}`);
    }
    seen.add(name);

    if (name !== "Unreleased") {
      if (!VERSION_PATTERN.test(name)) {
        throw new Error(`CHANGELOG.md 使用了无效版本号：${name}`);
      }
      if (!date || !isIsoDate(date)) {
        throw new Error(`CHANGELOG.md 的 ${name} 缺少合法 ISO 发布日期`);
      }
    }

    const bodyStart = match.index + match[0].length;
    const nextHeadingStart =
      index + 1 < matches.length ? matches[index + 1].index : content.length;
    const bodyEnd = Math.min(
      nextHeadingStart,
      linkBlockStart > bodyStart ? linkBlockStart : nextHeadingStart,
    );
    const body = content.slice(bodyStart, bodyEnd).trim();
    validateCategories(body, name, name === "Unreleased");

    const linkPattern = new RegExp(
      `^\\[${escapePattern(name)}\\]:\\s+https?://\\S+$`,
      "m",
    );
    if (!linkPattern.test(content)) {
      throw new Error(`CHANGELOG.md 缺少 ${name} 的版本或比较链接`);
    }

    return { name, date, body };
  });

  const releasedSections = sections.slice(1);
  for (let index = 0; index + 1 < releasedSections.length; index += 1) {
    if (
      compareVersions(
        releasedSections[index].name,
        releasedSections[index + 1].name,
      ) <= 0
    ) {
      throw new Error("CHANGELOG.md 的已发布版本必须按版本号倒序排列");
    }
  }

  const links = new Map(
    [...content.matchAll(/^\[([^\]]+)\]:\s+(\S+)[ \t]*$/gm)].map((match) => [
      match[1],
      match[2],
    ]),
  );
  const newestVersion = releasedSections[0].name;
  if (!links.get("Unreleased").endsWith(`/compare/v${newestVersion}...HEAD`)) {
    throw new Error("CHANGELOG.md 的 Unreleased 比较链接未指向最新版本");
  }
  for (let index = 0; index + 1 < releasedSections.length; index += 1) {
    const currentVersion = releasedSections[index].name;
    const previousVersion = releasedSections[index + 1].name;
    if (
      !links
        .get(currentVersion)
        .endsWith(`/compare/v${previousVersion}...v${currentVersion}`)
    ) {
      throw new Error(`CHANGELOG.md 的 ${currentVersion} 比较链接不正确`);
    }
  }

  return sections;
}

export function extractReleaseBody(markdown, version) {
  const section = parseChangelog(markdown).find((item) => item.name === version);
  if (!section) {
    throw new Error(
      `CHANGELOG.md 尚无 ${version} 版本段；发布前请把 Unreleased 内容收口到“## [${version}] - YYYY-MM-DD”`,
    );
  }
  return section;
}

function readProjectFiles() {
  const projectRoot = fileURLToPath(new URL("../", import.meta.url));
  const packageJson = JSON.parse(
    readFileSync(path.join(projectRoot, "package.json"), "utf8"),
  );
  const changelog = readFileSync(
    path.join(projectRoot, "CHANGELOG.md"),
    "utf8",
  );
  return { changelog, version: packageJson.version };
}

function writeGithubOutput(body) {
  const outputPath = process.env.GITHUB_OUTPUT;
  if (!outputPath) {
    throw new Error("缺少 GITHUB_OUTPUT，不能生成 GitHub Release 正文");
  }
  const delimiter = `CHANGELOG_${randomUUID()}`;
  appendFileSync(
    outputPath,
    `releaseBody<<${delimiter}\n${body}\n${delimiter}\n`,
    "utf8",
  );
}

function main() {
  const args = new Set(process.argv.slice(2));
  const knownArgs = new Set(["--github-output"]);
  for (const arg of args) {
    if (!knownArgs.has(arg)) {
      throw new Error(`未知参数：${arg}`);
    }
  }

  const { changelog, version } = readProjectFiles();
  const sections = parseChangelog(changelog);
  if (args.has("--github-output")) {
    const release = extractReleaseBody(changelog, version);
    writeGithubOutput(release.body);
    console.log(`标准发布说明已生成：v${version} (${release.date})`);
    return;
  }

  const released = sections.some((section) => section.name === version);
  const suffix = released
    ? `当前版本 v${version} 已有正式版本段`
    : `v${version} 仍在 Unreleased，正式发布前需填写版本段和实际日期`;
  console.log(`CHANGELOG 标准结构检查通过：${suffix}`);
}

const currentFile = path.normalize(fileURLToPath(import.meta.url));
const entryFile = process.argv[1] ? path.normalize(path.resolve(process.argv[1])) : "";
if (currentFile.toLowerCase() === entryFile.toLowerCase()) {
  try {
    main();
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
