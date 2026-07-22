/** 桌面窗口控制器：隔离窗口模式与状态文案，不处理账本业务。 */
export class WindowController {
  /**
   * @param {Object} dependencies
   * @param {import("./ledger-contract.js").WindowGateway|null} dependencies.gateway
   * @param {HTMLElement} dependencies.root
   * @param {HTMLElement|null} dependencies.statusText
   */
  constructor({ gateway, root, statusText }) {
    this.gateway = gateway;
    this.root = root;
    this.statusText = statusText;
    this.mode = root.dataset.mode || "capsule";
  }

  /** @param {Object} status */
  applyStatus(status) {
    this.mode = status.mode;
    this.root.dataset.mode = status.mode;
    if (this.statusText) {
      const focus = status.focused ? "当前有焦点" : "未抢焦点";
      const screen = status.inWorkArea ? "位置安全" : "位置需校正";
      this.statusText.textContent = `置顶 · ${focus} · ${screen}`;
    }
  }

  /** @param {string} nextMode */
  async setMode(nextMode) {
    const status = await this.gateway.setWindowMode(nextMode, nextMode === "expanded");
    this.applyStatus(status);
  }

  hideToTray() {
    return this.gateway.hideToTray();
  }

  subscribeToStatusChanges() {
    return this.gateway.subscribeWindowStatus((status) => this.applyStatus(status));
  }

  showStaticPreview() {
    this.mode = "expanded";
    this.root.dataset.mode = "expanded";
    if (this.statusText) {
      this.statusText.textContent = "浏览器预览 · 未连接本地账本";
    }
  }
}
