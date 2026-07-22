/** 只管理界面壳的临时状态，不接触账本和窗口领域状态。 */
export class ShellController {
  /** @param {{root: HTMLElement, menu: HTMLDetailsElement|null, search?: import("./list-search-controller.js").ListSearchController|null}} dependencies */
  constructor({ root, menu, search = null }) {
    this.root = root;
    this.menu = menu;
    this.search = search;
    this.showTasks();
  }

  showTasks() {
    this.search?.close({ restoreFocus: false });
    this.root.dataset.panel = "tasks";
    this.search?.setPanel("tasks");
  }

  showHistory() {
    this.closeMenu();
    this.search?.close({ restoreFocus: false });
    this.root.dataset.panel = "history";
    this.search?.setPanel("history");
  }

  closeMenu() {
    if (this.menu) this.menu.open = false;
  }

  /** Esc 依次关闭菜单、搜索，再返回待办页；都没有时交还窗口控制器。 */
  closeTransientUi() {
    if (this.menu?.open) {
      this.closeMenu();
      return true;
    }
    if (this.search?.close()) return true;
    if (this.root.dataset.panel === "history") {
      this.showTasks();
      return true;
    }
    return false;
  }
}
