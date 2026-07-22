import test from "node:test";
import assert from "node:assert/strict";

import { assertDeadlineOn } from "../app/ledger-contract.js";
import {
  deadlinePresentation,
  localDateOnly,
  millisecondsUntilNextLocalDay,
} from "../app/deadline-date.js";

test("无期限不产生任何常态展示", () => {
  assert.equal(deadlinePresentation(null, "2026-07-18"), null);
});

test("截止日期按今天、明天、普通日期和逾期派生紧凑文案", () => {
  assert.deepEqual(deadlinePresentation("2026-07-18", "2026-07-18"), {
    label: "今天",
    state: "today",
    title: "截止日期：2026-07-18",
  });
  assert.equal(deadlinePresentation("2026-07-19", "2026-07-18").label, "明天");
  assert.equal(deadlinePresentation("2026-07-24", "2026-07-18").label, "7/24");
  assert.equal(deadlinePresentation("2027-01-02", "2026-07-18").label, "2027/1/2");
  assert.deepEqual(deadlinePresentation("2026-07-16", "2026-07-18"), {
    label: "逾期 2 天",
    state: "overdue",
    title: "截止日期：2026-07-16",
  });
});

test("日期差按日历日计算并正确跨越闰日", () => {
  assert.equal(deadlinePresentation("2028-03-01", "2028-02-28").label, "3/1");
  assert.equal(deadlinePresentation("2028-02-29", "2028-02-28").label, "明天");
});

test("本地日期输出固定为 YYYY-MM-DD", () => {
  assert.equal(localDateOnly(new Date(2026, 6, 8, 23, 59)), "2026-07-08");
});

test("午夜展示刷新按本地日历日计算且始终返回正数", () => {
  assert.equal(
    millisecondsUntilNextLocalDay(new Date(2026, 6, 18, 12, 0, 0, 0)),
    12 * 60 * 60 * 1000,
  );
  assert.equal(
    millisecondsUntilNextLocalDay(new Date(2026, 6, 18, 23, 59, 59, 500)),
    500,
  );
});

test("严格日期校验支持早期年份但拒绝零年与不存在日期", () => {
  assert.doesNotThrow(() => assertDeadlineOn("0001-01-01"));
  for (const value of ["0000-01-01", "2026-02-29", "2026-2-03"]) {
    assert.throws(() => assertDeadlineOn(value));
  }
});
