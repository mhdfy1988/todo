/**
 * 返回当前本地自然周的半开区间：[周一 00:00，下周一 00:00)。
 * 使用日历日期推进，避免用固定毫秒数跨越夏令时。
 *
 * @param {Date} [now]
 * @returns {{fromMs: number, toMs: number}}
 */
export function currentLocalWeekRange(now = new Date()) {
  if (!(now instanceof Date) || Number.isNaN(now.getTime())) {
    throw new TypeError("当前时间无效");
  }

  const start = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const daysSinceMonday = (start.getDay() + 6) % 7;
  start.setDate(start.getDate() - daysSinceMonday);

  const end = new Date(start);
  end.setDate(end.getDate() + 7);
  return { fromMs: start.getTime(), toMs: end.getTime() };
}

/**
 * @param {Array<{titleSnapshot: string}>} completions
 * @returns {string}
 */
export function formatWeeklyCompletionMarkdown(completions) {
  if (!Array.isArray(completions) || completions.length === 0) {
    throw new TypeError("本周完成记录不能为空");
  }
  const entries = weeklyCompletionEntries(completions);
  const items = entries.map((entry, index) => {
    const lines = [`${index + 1}. ${entry.title}`];
    entry.children.forEach((title) => lines.push(`   - ${title}`));
    return lines.join("\n");
  });
  return `## 本周完成\n\n${items.join("\n")}`;
}

/** 父项本周完成时只输出一个父项，子完成事实作为缩进明细。 */
export function weeklyCompletionEntries(completions) {
  const parentCompletions = new Map(
    completions
      .filter((event) => event.eventType === "completed")
      .map((event) => [event.taskId, event]),
  );
  const childrenByParent = new Map();
  completions
    .filter((event) => event.eventType === "subtask_completed")
    .forEach((event) => {
      const parentTaskId = event.metadata.parentTaskId;
      const existing = childrenByParent.get(parentTaskId) ?? [];
      existing.push(event);
      childrenByParent.set(parentTaskId, existing);
    });

  const emittedParents = new Set();
  const entries = [];
  completions.forEach((event) => {
    if (event.eventType === "completed") {
      if (emittedParents.has(event.taskId)) return;
      emittedParents.add(event.taskId);
      entries.push({
        title: cleanTitle(event.titleSnapshot),
        children: (childrenByParent.get(event.taskId) ?? [])
          .map((child) => cleanTitle(child.titleSnapshot)),
      });
      return;
    }
    const parentTaskId = event.metadata.parentTaskId;
    if (parentCompletions.has(parentTaskId)) return;
    entries.push({
      title: `${cleanTitle(event.metadata.parentTitle)} / ${cleanTitle(event.titleSnapshot)}`,
      children: [],
    });
  });
  return entries;
}

/**
 * 只读应用服务：读取本周有效完成事实，生成 Markdown，并在用户操作后写入剪贴板。
 */
export class WeeklyCompletionController {
  /**
   * @param {Object} dependencies
   * @param {{weeklyFacts(fromMs: number, toMs: number): Promise<unknown>}} dependencies.gateway
   * @param {{writeText(text: string): Promise<void>}} dependencies.clipboard
   * @param {{show(message: string): void}} dependencies.toast
   * @param {() => Date} [dependencies.now]
   */
  constructor({ gateway, clipboard, toast, now = () => new Date() }) {
    if (!gateway || typeof gateway.weeklyFacts !== "function") {
      throw new TypeError("WeeklyCompletionController 缺少周报事实查询端口");
    }
    if (!clipboard || typeof clipboard.writeText !== "function") {
      throw new TypeError("WeeklyCompletionController 缺少剪贴板写入端口");
    }
    if (!toast || typeof toast.show !== "function") {
      throw new TypeError("WeeklyCompletionController 缺少提示视图");
    }
    if (typeof now !== "function") {
      throw new TypeError("WeeklyCompletionController 的时钟无效");
    }

    this.gateway = gateway;
    this.clipboard = clipboard;
    this.toast = toast;
    this.now = now;
  }

  async copyCurrentWeek() {
    const range = currentLocalWeekRange(this.now());
    const facts = assertWeeklyFacts(
      await this.gateway.weeklyFacts(range.fromMs, range.toMs),
      range,
    );
    const count = weeklyCompletionEntries(facts.effectiveCompletions).length;
    if (count === 0) {
      this.toast.show("本周还没有完成记录");
      return { copied: false, count, ...range };
    }

    const markdown = formatWeeklyCompletionMarkdown(facts.effectiveCompletions);
    await this.clipboard.writeText(markdown);
    this.toast.show(`已复制本周完成（${count} 项）`);
    return { copied: true, count, markdown, ...range };
  }
}

function assertWeeklyFacts(value, expectedRange) {
  const validRange = value
    && typeof value === "object"
    && Number.isSafeInteger(value.fromMs)
    && Number.isSafeInteger(value.toMs)
    && value.fromMs === expectedRange.fromMs
    && value.toMs === expectedRange.toMs;
  const validCollections = validRange
    && Array.isArray(value.effectiveCompletions)
    && Array.isArray(value.ongoingTasks);
  const parentCompletionsInRange = new Set(
    validCollections
      ? value.effectiveCompletions
        .filter((event) => event?.eventType === "completed"
          && hasValidCompletionShape(event)
          && isWithinRange(event.occurredAtMs, expectedRange))
        .map((event) => event.taskId)
      : [],
  );
  const validCompletions = validCollections && value.effectiveCompletions.every((event) => {
    if (!hasValidCompletionShape(event)) return false;
    if (event.eventType === "completed") {
      return isWithinRange(event.occurredAtMs, expectedRange);
    }
    const validMetadata = event.metadata
      && typeof event.metadata === "object"
      && typeof event.metadata.parentTaskId === "string"
      && event.metadata.parentTaskId.length > 0
      && typeof event.metadata.parentTitle === "string"
      && event.metadata.parentTitle.trim().length > 0;
    if (!validMetadata) return false;
    return isWithinRange(event.occurredAtMs, expectedRange)
      || (event.occurredAtMs < expectedRange.fromMs
        && parentCompletionsInRange.has(event.metadata.parentTaskId));
  });

  if (!validCompletions) {
    throw new Error("本地账本返回了无效的本周完成记录");
  }
  return value;
}

function hasValidCompletionShape(event) {
  return Boolean(
    event
      && typeof event === "object"
      && (event.eventType === "completed" || event.eventType === "subtask_completed")
      && typeof event.taskId === "string"
      && event.taskId.length > 0
      && typeof event.titleSnapshot === "string"
      && event.titleSnapshot.trim().length > 0
      && Number.isSafeInteger(event.occurredAtMs),
  );
}

function isWithinRange(occurredAtMs, range) {
  return occurredAtMs >= range.fromMs && occurredAtMs < range.toMs;
}

function cleanTitle(value) {
  return String(value ?? "").replace(/[\r\n]+/g, " ").trim();
}
