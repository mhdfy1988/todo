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

  const items = completions.map((event, index) => {
    const title = event.titleSnapshot.replace(/[\r\n]+/g, " ");
    return `${index + 1}. ${title}`;
  });
  return `## 本周完成\n\n${items.join("\n")}`;
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
    const count = facts.effectiveCompletions.length;
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
  const validCompletions = validCollections && value.effectiveCompletions.every((event) => (
    event
      && typeof event === "object"
      && event.eventType === "completed"
      && typeof event.titleSnapshot === "string"
      && event.titleSnapshot.trim().length > 0
      && Number.isSafeInteger(event.occurredAtMs)
      && event.occurredAtMs >= expectedRange.fromMs
      && event.occurredAtMs < expectedRange.toMs
  ));

  if (!validCompletions) {
    throw new Error("本地账本返回了无效的本周完成记录");
  }
  return value;
}
