import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  extractReleaseBody,
  parseChangelog,
} from "./release-changelog.mjs";

const projectChangelog = readFileSync(
  new URL("../CHANGELOG.md", import.meta.url),
  "utf8",
);

test("当前 CHANGELOG 使用标准结构并已收口 0.1.6", () => {
  const sections = parseChangelog(projectChangelog);

  assert.equal(sections[0].name, "Unreleased");
  assert.equal(sections[0].body, "");
  assert.equal(sections[1].name, "0.1.6");
  assert.equal(sections[1].date, "2026-08-26");
  assert.match(sections[1].body, /新建顶层待办默认插入列表最上方/);
  assert.match(sections[1].body, /阻止正式桌面应用重复打开/);
  assert.equal(sections[2].name, "0.1.5");
  assert.equal(sections[2].date, "2026-07-27");
  assert.match(sections[2].body, /统一待办页与已完成页的父子待办折叠入口/);
});

test("Windows CRLF 换行不会破坏标准结构解析", () => {
  const sections = parseChangelog(projectChangelog.replaceAll("\n", "\r\n"));

  assert.equal(sections[0].name, "Unreleased");
  assert.equal(sections[1].name, "0.1.6");
});

test("发布正文只提取指定版本的标准分类内容", () => {
  const release = extractReleaseBody(projectChangelog, "0.1.6");

  assert.equal(release.date, "2026-08-26");
  assert.match(release.body, /^### Changed/m);
  assert.match(release.body, /^### Fixed/m);
  assert.match(release.body, /新建顶层待办默认插入列表最上方/);
  assert.match(release.body, /阻止正式桌面应用重复打开/);
  assert.doesNotMatch(release.body, /0\.1\.5/);
});

test("请求不存在的版本时拒绝生成发布正文", () => {
  assert.throws(
    () => extractReleaseBody(projectChangelog, "9.9.9"),
    /尚无 9\.9\.9 版本段/,
  );
});

test("非标准分类会被拒绝", () => {
  const invalid = projectChangelog.replace("### Added", "### Improvements");

  assert.throws(
    () => parseChangelog(invalid),
    /非标准分类：Improvements/,
  );
});
