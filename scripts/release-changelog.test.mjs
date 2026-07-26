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

test("当前 CHANGELOG 使用标准结构并已收口 0.1.5", () => {
  const sections = parseChangelog(projectChangelog);

  assert.equal(sections[0].name, "Unreleased");
  assert.equal(sections[0].body, "");
  assert.equal(sections[1].name, "0.1.5");
  assert.equal(sections[1].date, "2026-07-27");
  assert.match(sections[1].body, /统一待办页与已完成页的父子待办折叠入口/);
  assert.match(sections[1].body, /透明缩进区域与只覆盖子项范围的短轨道/);
  assert.match(sections[1].body, /鼠标悬浮或键盘聚焦时显示轻量行级反馈/);
  assert.match(sections[1].body, /悬浮区域补足前后留白/);
  assert.equal(sections[2].name, "0.1.4");
  assert.equal(sections[2].date, "2026-07-24");
  assert.match(sections[2].body, /软件名称由“代办”更正为“待办”/);
});

test("Windows CRLF 换行不会破坏标准结构解析", () => {
  const sections = parseChangelog(projectChangelog.replaceAll("\n", "\r\n"));

  assert.equal(sections[0].name, "Unreleased");
  assert.equal(sections[1].name, "0.1.5");
});

test("发布正文只提取指定版本的标准分类内容", () => {
  const release = extractReleaseBody(projectChangelog, "0.1.5");

  assert.equal(release.date, "2026-07-27");
  assert.match(release.body, /^### Changed/m);
  assert.match(release.body, /^### Fixed/m);
  assert.doesNotMatch(release.body, /0\.1\.4/);
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
