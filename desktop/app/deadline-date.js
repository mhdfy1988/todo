import { assertDeadlineOn } from "./ledger-contract.js";

const DAY_MS = 24 * 60 * 60 * 1000;

/** 把本地时间压缩为不带时区的日历日期。 */
export function localDateOnly(now = new Date()) {
  return [
    String(now.getFullYear()).padStart(4, "0"),
    String(now.getMonth() + 1).padStart(2, "0"),
    String(now.getDate()).padStart(2, "0"),
  ].join("-");
}

/** 返回距离下一个本地日历日开始的毫秒数；由 Date 处理夏令时边界。 */
export function millisecondsUntilNextLocalDay(now = new Date()) {
  const next = new Date(now.getTime());
  next.setHours(24, 0, 0, 0);
  return Math.max(1, next.getTime() - now.getTime());
}

/**
 * 截止日期只派生展示，不改变任务状态或顺序。
 * @param {string|null} deadlineOn
 * @param {string} todayOn
 */
export function deadlinePresentation(deadlineOn, todayOn = localDateOnly()) {
  if (deadlineOn === null) return null;
  assertDeadlineOn(deadlineOn);
  assertDeadlineOn(todayOn);

  const difference = epochDay(deadlineOn) - epochDay(todayOn);
  let label;
  let state;
  if (difference < 0) {
    label = `逾期 ${Math.abs(difference)} 天`;
    state = "overdue";
  } else if (difference === 0) {
    label = "今天";
    state = "today";
  } else if (difference === 1) {
    label = "明天";
    state = "upcoming";
  } else {
    const [year, month, day] = deadlineOn.split("-").map(Number);
    label = deadlineOn.slice(0, 4) === todayOn.slice(0, 4)
      ? `${month}/${day}`
      : `${year}/${month}/${day}`;
    state = "upcoming";
  }

  return {
    label,
    state,
    title: `截止日期：${deadlineOn}`,
  };
}

function epochDay(value) {
  const [year, month, day] = value.split("-").map(Number);
  const date = new Date(0);
  date.setUTCHours(0, 0, 0, 0);
  date.setUTCFullYear(year, month - 1, day);
  return Math.floor(date.getTime() / DAY_MS);
}
