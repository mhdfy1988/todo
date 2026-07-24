const PANEL_COPY = Object.freeze({
  tasks: { label: "搜索待办", listId: "taskList" },
  history: { label: "搜索已完成", listId: "historyList" },
});

/** 只管理当前列表的临时搜索状态，不读取账本，也不提交任何命令。 */
export class ListSearchController {
  /**
   * @param {{
   *   root: HTMLElement,
   *   form: HTMLFormElement,
   *   label: HTMLLabelElement,
   *   input: HTMLInputElement,
   *   cancelButton: HTMLButtonElement,
   *   searchAction: HTMLButtonElement,
   *   captureForm: HTMLFormElement,
   *   taskTitleInput: HTMLInputElement,
   *   historyBackButton: HTMLButtonElement,
   *   menuButton: HTMLElement,
   *   onChange: (state: {panel: "tasks"|"history"|null, query: string}) => void,
   * }} dependencies
   */
  constructor({
    root,
    form,
    label,
    input,
    cancelButton,
    searchAction,
    captureForm,
    taskTitleInput,
    historyBackButton,
    menuButton,
    onChange,
  }) {
    this.root = root;
    this.form = form;
    this.label = label;
    this.input = input;
    this.cancelButton = cancelButton;
    this.searchAction = searchAction;
    this.captureForm = captureForm;
    this.taskTitleInput = taskTitleInput;
    this.historyBackButton = historyBackButton;
    this.menuButton = menuButton;
    this.onChange = onChange;
    this.panel = "tasks";
    this.activePanel = null;
    this.query = "";
    this.composing = false;

    form.addEventListener("submit", (event) => event.preventDefault());
    input.addEventListener("input", () => {
      if (!this.composing) this.#updateQuery();
    });
    input.addEventListener("compositionstart", () => {
      this.composing = true;
    });
    input.addEventListener("compositionend", () => {
      this.composing = false;
      this.#updateQuery();
    });
    cancelButton.addEventListener("click", () => this.close());
    this.setPanel("tasks");
  }

  get state() {
    return Object.freeze({ panel: this.activePanel, query: this.query });
  }

  setPanel(panel) {
    const copy = panelCopy(panel);
    this.panel = panel;
    this.searchAction.textContent = copy.label;
    this.searchAction.setAttribute("aria-label", copy.label);
  }

  open(panel = this.panel) {
    const copy = panelCopy(panel);
    if (this.activePanel === panel) {
      this.focus();
      return false;
    }
    if (this.activePanel) this.close({ restoreFocus: false });
    this.setPanel(panel);
    this.activePanel = panel;
    this.query = "";
    this.input.value = "";
    this.label.textContent = copy.label;
    this.input.placeholder = copy.label;
    this.input.setAttribute("aria-label", copy.label);
    this.input.setAttribute("aria-controls", copy.listId);
    this.form.hidden = false;
    this.captureForm.hidden = panel === "tasks";
    this.root.dataset.searchPanel = panel;

    // 先转移焦点，让现有标题编辑器按原 focusout 规则收尾，再触发列表重绘。
    this.focus();
    this.#emit();
    return true;
  }

  close({ restoreFocus = true } = {}) {
    if (!this.activePanel) return false;
    const previousPanel = this.activePanel;
    this.activePanel = null;
    this.query = "";
    this.composing = false;
    this.input.value = "";
    this.form.hidden = true;
    this.captureForm.hidden = false;
    delete this.root.dataset.searchPanel;
    this.#emit();
    if (restoreFocus) {
      const fallback = previousPanel === "history"
        ? this.historyBackButton
        : this.taskTitleInput;
      if (!fallback.disabled && !fallback.hidden) {
        fallback.focus();
      } else {
        this.menuButton.focus();
      }
    }
    return true;
  }

  isActive(panel = this.panel) {
    return this.activePanel === panel;
  }

  isComposing() {
    return this.composing;
  }

  focus({ select = true } = {}) {
    if (!this.activePanel) return false;
    this.input.focus();
    if (select) this.input.select();
    return true;
  }

  #updateQuery() {
    if (!this.activePanel) return;
    const nextQuery = this.input.value.trim();
    if (nextQuery === this.query) return;
    this.query = nextQuery;
    this.#emit();
  }

  #emit() {
    this.onChange(this.state);
  }
}

export function isSearchShortcut(event) {
  return Boolean(
    (event.ctrlKey || event.metaKey)
      && !event.altKey
      && String(event.key).toLocaleLowerCase("en-US") === "f",
  );
}

function panelCopy(panel) {
  const copy = PANEL_COPY[panel];
  if (!copy) throw new Error(`未知搜索面板：${panel}`);
  return copy;
}
