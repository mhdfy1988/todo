/**
 * @typedef {"normal" | "smoke"} RuntimeProfile
 *
 * @typedef {Object} LedgerSnapshot
 * @property {Object | null} currentTask
 * @property {Array<Object>} queue
 * @property {Array<Object>} completed
 * @property {Array<Object>} subtasks
 * @property {Array<Object>} effectiveCompletions
 * @property {Array<Object>} events
 * @property {Array<Object>} rewards
 * @property {number} balance
 *
 * @typedef {Object} PendingOperation
 * @property {string} key
 * @property {string} operationId
 * @property {string} command
 * @property {Record<string, unknown>} payload
 * @property {boolean} committed
 */

export const LedgerPhase = Object.freeze({
  LOADING: "loading",
  READY: "ready",
  ERROR: "error",
  RECOVERY: "recovery",
  BUSY: "busy",
});

/** @returns {LedgerSnapshot} */
export function emptySnapshot() {
  return {
    currentTask: null,
    queue: [],
    completed: [],
    subtasks: [],
    effectiveCompletions: [],
    events: [],
    rewards: [],
    balance: 0,
  };
}

export function createInitialState() {
  return Object.freeze({
    phase: LedgerPhase.LOADING,
    profile: null,
    snapshot: emptySnapshot(),
    snapshotReady: false,
    pendingOperation: null,
    error: null,
  });
}

/**
 * 很小的可观察状态容器。状态迁移集中在应用服务，视图只订阅结果。
 *
 * @param {ReturnType<typeof createInitialState>} [initialState]
 */
export function createLedgerStore(initialState = createInitialState()) {
  let state = initialState;
  const listeners = new Set();

  return {
    getState() {
      return state;
    },

    /** @param {Object | ((current: Object) => Object)} change */
    update(change) {
      const patch = typeof change === "function" ? change(state) : change;
      state = Object.freeze({ ...state, ...patch });
      listeners.forEach((listener) => listener(state));
      return state;
    },

    subscribe(listener) {
      listeners.add(listener);
      listener(state);
      return () => listeners.delete(listener);
    },
  };
}

export function isLedgerInteractive(state) {
  return state.phase === LedgerPhase.READY && !state.pendingOperation;
}
