import { LedgerPhase } from "./state.js";

export function activeCompletionEvents(snapshot) {
  const undone = new Set(
    snapshot.events
      .filter((event) => event.eventType === "completion_undone")
      .map((event) => event.reversesEventId),
  );
  return snapshot.events.filter(
    (event) => event.eventType === "completed" && !undone.has(event.id),
  );
}

export function upcomingTasks(snapshot) {
  return snapshot.queue.slice(1, 4);
}

export function normalizeSearchQuery(query) {
  return String(query ?? "").trim().toLocaleLowerCase("zh-CN");
}

export function filterTasksByTitle(tasks, query) {
  return filterByTitle(tasks, query, (task) => task.title);
}

export function filterCompletionEventsByTitle(events, query) {
  return filterByTitle(events, query, (event) => event.titleSnapshot);
}

export function titleMatchRanges(title, query) {
  const normalizedTitle = String(title ?? "").toLocaleLowerCase("zh-CN");
  const normalizedQuery = normalizeSearchQuery(query);
  if (!normalizedQuery) return [];
  const ranges = [];
  let offset = 0;
  while (offset < normalizedTitle.length) {
    const index = normalizedTitle.indexOf(normalizedQuery, offset);
    if (index < 0) break;
    ranges.push([index, index + normalizedQuery.length]);
    offset = index + normalizedQuery.length;
  }
  return ranges;
}

function filterByTitle(items, query, titleOf) {
  const normalizedQuery = normalizeSearchQuery(query);
  if (!normalizedQuery) return [...items];
  return items.filter((item) => String(titleOf(item) ?? "")
    .toLocaleLowerCase("zh-CN")
    .includes(normalizedQuery));
}

export function ledgerContentKind(state) {
  if (state.phase === LedgerPhase.ERROR) return "error";
  if (state.snapshotReady) return "snapshot";
  if (state.phase === LedgerPhase.RECOVERY) return "recovery";
  return "loading";
}

export function diagnosticsPassed(status, integrity) {
  return Boolean(
    status.inWorkArea
      && status.alwaysOnTop
      && status.trayReady
      && integrity.sqliteQuickCheck
      && integrity.foreignKeys
      && integrity.rewardPrefixBalances
      && integrity.eventRewardLinks
      && integrity.receiptLinks
      && integrity.taskRewardNetValues
      && integrity.taskProjectionMatchesLedger
      && Array.isArray(integrity.failures)
      && integrity.failures.length === 0,
  );
}

export function formatTime(timestamp) {
  const value = new Date(timestamp);
  if (Number.isNaN(value.getTime())) return "";
  return new Intl.DateTimeFormat("zh-CN", {
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(value);
}
