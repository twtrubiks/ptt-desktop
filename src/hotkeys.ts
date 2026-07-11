// 快捷鍵攔截層：掛在 xterm attachCustomKeyEventHandler。
// 命中快捷鍵展開為指令序列，否則放行給 xterm 正常處理。
// 回傳 false = 事件已被攔截，xterm 不再處理。

export interface HotkeyActions {
  send(sequence: string): void;
  reconnect(): void;
  toggleFullscreen(): void;
}

export class HotkeyHandler {
  constructor(
    private hotkeys: Record<string, string>,
    private actions: HotkeyActions,
  ) {}

  handleKeyEvent(event: KeyboardEvent): boolean {
    if (event.type !== "keydown") return true;
    // 保留本地鍵 Ctrl+R = 手動重連（不送給 PTT）
    if (event.ctrlKey && !event.altKey && !event.metaKey && event.key.toLowerCase() === "r") {
      this.actions.reconnect();
      return false;
    }
    // 保留本地鍵 F11 = 全螢幕切換，優先於設定檔快捷鍵
    if (event.key === "F11" && !event.ctrlKey && !event.altKey && !event.metaKey) {
      this.actions.toggleFullscreen();
      return false;
    }
    if (event.ctrlKey || event.altKey || event.metaKey) return true;
    // Shift+特殊鍵（如 Shift+F3）視為不同組合，不觸發 F3 的快捷鍵；
    // 可列印字元不在此限——shift 已反映在 event.key（如 "?"），排除會讓這類設定失效
    if (event.shiftKey && event.key.length > 1) return true;
    const sequence = this.hotkeys[event.key];
    if (sequence !== undefined) {
      this.actions.send(sequence);
      return false;
    }
    return true;
  }
}
