// 進入點：組裝各模組。
// 下行：wss binary → telnet 濾層 →（byte 網格 shadow 觀察＋BS@80 正規化）
//   → 雙色字重排 → codec 解碼 → 雙色字拆分 → xterm →（批末孤兒補寫）；
// 同時餵給自動登入狀態機。
// 上行：xterm.onData →（快捷鍵層在 key event 階段先攔）→ UTF-8 binary 送出。
import { getCurrentWindow } from "@tauri-apps/api/window";
import { AutoLogin, WRONG_PASSWORD_DETAIL } from "./auto-login";
import { PttDecoder } from "./codec";
import { loadConfig } from "./config";
import { PttConnection } from "./connection";
import { DevRecorder } from "./dev-recorder";
import { buildOrphanRepair, DuotoneSplitFilter } from "./duotone";
import { HotkeyHandler } from "./hotkeys";
import { ScreenGrid } from "./screen-grid";
import { SgrReorderFilter } from "./sgr-reorder";
import { createTerminal, fitFontSize } from "./terminal";
import { TelnetFilter } from "./telnet";

async function init(): Promise<void> {
  const { config, error: configError } = await loadConfig();
  const container = document.querySelector<HTMLElement>("#terminal");
  if (!container) throw new Error("找不到 #terminal 容器");

  const term = createTerminal(container, config.fontFamily, config.fontSize, config.lineHeight);
  const telnet = new TelnetFilter();
  const grid = new ScreenGrid(); // byte 網格 shadow：第五根因（半字覆寫）修復式接線
  const reorder = new SgrReorderFilter();
  const codec = new PttDecoder();
  const duotone = new DuotoneSplitFilter();
  // dev 錄製（P4 探勘）：release build 的 DEV 摺疊為 false，整個模組被 tree-shake
  const recorder = import.meta.env.DEV ? new DevRecorder() : null;
  let autoLogin: AutoLogin | null = null;
  let awaitingReconnect = false;

  const conn = new PttConnection({
    onBinary(data) {
      const filtered = telnet.filter(data);
      if (filtered.length === 0) return;
      recorder?.push(filtered); // 錄製與位元組層比對都收正規化前的原始 bytes
      // 位元組層先餵：密碼錯時偵測到會（在 onFinished 內）把 codec 與 reorder 撥回
      // big5，本 chunk 隨後就以正確編碼配對、解碼，亂碼不會先上畫面
      autoLogin?.feedBytes(filtered);
      // shadow 觀察同點 bytes；下游改吃 output（BS@80 已正規化為絕對 CUP）
      const { output, repairs } = grid.feed(filtered);
      const text = duotone.process(codec.decode(reorder.process(output)));
      term.write(text);
      // 批末仍存活的孤兒半字（xterm 端整格已被原子清空）以 PUA 半字補寫回去，
      // 字形由 duotone-overlay 繪回；序列自含游標與 SGR 的保存還原
      if (repairs.length > 0) term.write(buildOrphanRepair(repairs));
      autoLogin?.feed(text);
    },
    onClose(reason) {
      recorder?.mark("close");
      autoLogin?.stop();
      autoLogin = null;
      awaitingReconnect = true;
      const residue = duotone.flush(); // 扣住的殘留原樣吐回,孤兒 marker 由 xterm 吞掉
      if (residue) term.write(residue);
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
          recorder?.mark("utf8-switch"); // 標記之後的錄製 chunk 屬 utf-8 串流
          grid.switchToUtf8(); // shadow 與 codec 同時機切換（送出密碼後）
          // reorder 的殘留 bytes 屬 big5 串流,須趕在 codec 切換前解碼;
          // 兩段殘留都過 duotone 濾層,維持與主管線相同的處理順序
          const residue = reorder.switchToUtf8();
          if (residue.length) term.write(duotone.process(codec.decode(residue)));
          const tail = codec.switchToUtf8();
          if (tail) term.write(duotone.process(tail));
        },
        onFinished: (reason, detail) => {
          autoLogin = null;
          if (reason === "aborted") {
            // 密碼錯時 PTT 從未切到 UTF-8，撥回 big5 讓後續手動操作畫面可讀；
            // 其他中止原因（如登入成功後主功能表字樣不符）不能撥，PTT 可能已在 UTF-8。
            // reorder 殘留的少量 bytes 直接丟棄：錯誤畫面隨後會重繪，不值得為此倒序解碼
            if (detail === WRONG_PASSWORD_DETAIL) {
              codec.reset();
              reorder.reset();
              duotone.reset();
              grid.reset(); // shadow 一併回 big5、清網格；後續重繪會重建
            }
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
    grid.reset();
    reorder.reset();
    codec.reset();
    duotone.reset();
    recorder?.mark("connect"); // 每次連線（含重連）都回到 big5 模式起算
    term.write("連線 wss://ws.ptt.cc/bbs 中...\r\n");
    try {
      // 先啟動自動登入再連線：歡迎畫面可能在 connect() resolve 前就到達，
      // 得先有 autoLogin 才接得到「請輸入代號」（逾時計時因此涵蓋連線期間）
      startAutoLogin();
      await conn.connect();
    } catch (error) {
      autoLogin?.stop();
      autoLogin = null;
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
