const AUTO_CHECK_INTERVAL_MS = 24 * 60 * 60 * 1000;

/**
 * 更新入口只编排“检查、提示、用户确认安装”，签名验证和安装均由 Rust 适配器负责。
 */
export class UpdateController {
  constructor({ gateway, actionButton, root, toast, timerHost = globalThis }) {
    if (!gateway) throw new TypeError("UpdateController 缺少 TauriGateway");
    if (!actionButton) throw new TypeError("UpdateController 缺少更新按钮");
    if (!root) throw new TypeError("UpdateController 缺少桌面根节点");
    if (!toast || typeof toast.show !== "function") {
      throw new TypeError("UpdateController 缺少提示视图");
    }
    this.gateway = gateway;
    this.actionButton = actionButton;
    this.root = root;
    this.toast = toast;
    this.timerHost = timerHost;
    this.session = null;
    this.availableVersion = null;
    this.busy = false;
    this.installing = false;
    this.autoCheckTimer = null;
    this.started = false;
    this.#render();
  }

  async start(profile, session) {
    this.stop();
    this.session = session;
    this.started = profile === "normal";
    this.actionButton.hidden = !this.started;
    this.#render();
    if (!this.started) return;

    await this.check({ silent: true });
    this.autoCheckTimer = this.timerHost.setInterval?.(
      () => { void this.check({ silent: true }); },
      AUTO_CHECK_INTERVAL_MS,
    ) ?? null;
  }

  stop() {
    if (this.autoCheckTimer !== null) {
      this.timerHost.clearInterval?.(this.autoCheckTimer);
      this.autoCheckTimer = null;
    }
  }

  async handleAction() {
    if (this.availableVersion) {
      await this.#installAvailableUpdate();
      return;
    }
    await this.check({ silent: false });
  }

  async check({ silent = false } = {}) {
    if (!this.started || this.busy) return null;
    this.busy = true;
    this.#render();
    try {
      const status = await this.gateway.checkForUpdate();
      this.availableVersion = typeof status?.availableVersion === "string"
        ? status.availableVersion
        : null;
      if (this.availableVersion) {
        this.toast.show(`发现新版本 v${this.availableVersion}`);
      } else if (!silent) {
        this.toast.show("已是最新版本");
      }
      return status;
    } catch (error) {
      if (!silent) throw error;
      console.error("自动检查更新失败", error);
      return null;
    } finally {
      this.busy = false;
      this.#render();
    }
  }

  async #installAvailableUpdate() {
    if (this.busy || !this.availableVersion) return;
    if (!this.session?.canMutate()) {
      throw new Error("请先等当前待办操作完成，再安装更新");
    }

    const version = this.availableVersion;
    this.busy = true;
    this.installing = true;
    this.root.inert = true;
    this.root.setAttribute("aria-busy", "true");
    this.#render();
    this.toast.show("正在下载更新，完成后会自动重启");
    try {
      await this.gateway.installUpdate(version);
    } finally {
      this.root.inert = false;
      this.root.removeAttribute("aria-busy");
      this.installing = false;
      this.busy = false;
      this.#render();
    }
  }

  #render() {
    if (this.installing) {
      this.actionButton.textContent = "正在安装…";
    } else if (this.busy) {
      this.actionButton.textContent = "正在检查…";
    } else if (this.availableVersion) {
      this.actionButton.textContent = `安装更新 v${this.availableVersion}`;
    } else {
      this.actionButton.textContent = "检查更新";
    }
    this.actionButton.disabled = !this.started || this.busy;
  }
}
