// 進入點：組裝各模組。
// 下行：wss binary → telnet 濾層 → codec 解碼 → xterm；同時餵給自動登入狀態機。
// 上行：xterm.onData →（快捷鍵層在 key event 階段先攔）→ UTF-8 binary 送出。
import { getCurrentWindow } from "@tauri-apps/api/window";
import { AutoLogin } from "./auto-login";
import { PttDecoder } from "./codec";
import { loadConfig } from "./config";
import { PttConnection } from "./connection";
import { HotkeyHandler } from "./hotkeys";
import { createTerminal, fitFontSize } from "./terminal";
import { TelnetFilter } from "./telnet";

async function init(): Promise<void> {
  const { config, error: configError } = await loadConfig();
  const container = document.querySelector<HTMLElement>("#terminal");
  if (!container) throw new Error("找不到 #terminal 容器");

  const term = createTerminal(container, config.fontFamily, config.fontSize);
  const telnet = new TelnetFilter();
  const codec = new PttDecoder();
  let autoLogin: AutoLogin | null = null;
  let awaitingReconnect = false;

  const conn = new PttConnection({
    onBinary(data) {
      const filtered = telnet.filter(data);
      if (filtered.length === 0) return;
      const text = codec.decode(filtered);
      term.write(text);
      autoLogin?.feed(text);
    },
    onClose(reason) {
      autoLogin?.stop();
      autoLogin = null;
      awaitingReconnect = true;
      term.write(`\r\n\x1b[1;31m[已斷線：${reason}，按 Enter 重連]\x1b[m`);
    },
  });

  function startAutoLogin(): void {
    if (!config.username || !config.password) return;
    autoLogin = new AutoLogin(
      {
        username: config.username,
        password: config.password,
        kickOtherLogins: config.kickOtherLogins,
      },
      {
        send: (text) => conn.sendText(text),
        onSwitchUtf8: () => {
          const tail = codec.switchToUtf8();
          if (tail) term.write(tail);
        },
        onFinished: (reason, detail) => {
          autoLogin = null;
          if (reason === "aborted") {
            term.write(`\r\n\x1b[1;33m[自動登入中止（${detail ?? "未知原因"}），請手動操作]\x1b[m`);
          }
        },
      },
    );
    autoLogin.start();
  }

  // 連線／重連流程進行中擋掉重入：連按 Ctrl+R / Enter 不會開出第二條連線
  // （否則舊連線變 ghost 掛在 PTT 上，兩個 AutoLogin 也會互相干擾）。
  // reconnect 的 disconnect 等待期間也在防護範圍內，旗標在交棒給 connect 前才釋放。
  let connecting = false;

  async function connect(): Promise<void> {
    if (connecting) return;
    connecting = true;
    term.reset();
    telnet.reset();
    codec.reset();
    term.write("連線 wss://ws.ptt.cc/bbs 中...\r\n");
    try {
      await conn.connect();
      startAutoLogin();
    } catch (error) {
      awaitingReconnect = true;
      term.write(`\r\n\x1b[1;31m[連線失敗：${error}，按 Enter 重試]\x1b[m`);
    } finally {
      connecting = false;
    }
  }

  async function reconnect(): Promise<void> {
    if (connecting) return;
    connecting = true;
    try {
      autoLogin?.stop();
      autoLogin = null;
      await conn.disconnect();
      awaitingReconnect = false;
    } finally {
      connecting = false;
    }
    await connect(); // 釋放旗標到 connect 重新持有之間無 await，不會被插隊
  }

  term.onData((data) => {
    if (awaitingReconnect) {
      if (data === "\r") void reconnect();
      return;
    }
    conn.sendText(data);
  });

  const appWindow = getCurrentWindow();
  const hotkeyHandler = new HotkeyHandler(config.hotkeys, {
    send: (sequence) => {
      if (!awaitingReconnect) conn.sendText(sequence);
    },
    reconnect: () => void reconnect(),
    toggleFullscreen: () =>
      void appWindow
        .isFullscreen()
        .then((full) => appWindow.setFullscreen(!full))
        .catch(() => {}),
  });
  term.attachCustomKeyEventHandler((event) => hotkeyHandler.handleKeyEvent(event));

  window.addEventListener("resize", () => fitFontSize(term));
  fitFontSize(term);
  void document.fonts.ready.then(() => fitFontSize(term)); // 字型載入後格寬會變，重新量測

  term.focus();
  if (configError) {
    // 設定檔壞掉不能默默用預設值連線（使用者只會看到「沒有自動登入」卻查不到原因）；
    // 借用斷線重連機制暫停在此，訊息看完按 Enter 再連
    awaitingReconnect = true;
    term.write(
      `\x1b[1;33m[設定檔解析失敗：${configError}]\r\n` +
        `[改用預設值繼續（不會自動登入），請檢查 ~/.config/ptt-desktop/config.json，按 Enter 連線]\x1b[m`,
    );
  } else {
    await connect();
  }
}

window.addEventListener("DOMContentLoaded", () => void init());
