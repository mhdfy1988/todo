/** 系统剪贴板写入适配器；只暴露本功能需要的纯文本写权限。 */
export class TauriClipboardWriter {
  /** @param {((text: string) => Promise<void>)|null} writeText */
  constructor(writeText) {
    this.writeTextCommand = typeof writeText === "function" ? writeText : null;
  }

  /** @param {Window|Object|null|undefined} hostWindow */
  static fromWindow(hostWindow) {
    const clipboardManager = hostWindow?.__TAURI__?.clipboardManager;
    const writeText = clipboardManager?.writeText;
    return new TauriClipboardWriter(
      typeof writeText === "function" ? (text) => clipboardManager.writeText(text) : null,
    );
  }

  /** @param {string} text */
  async writeText(text) {
    if (!this.writeTextCommand) {
      throw new Error("系统剪贴板写入能力不可用");
    }
    if (typeof text !== "string") {
      throw new TypeError("剪贴板内容必须是文本");
    }
    await this.writeTextCommand(text);
  }
}
