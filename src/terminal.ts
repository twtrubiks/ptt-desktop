// xterm.js 初始化：固定 80x24 字元格；視窗縮放只調字型大小，不改行列數。
import { Terminal } from "@xterm/xterm";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { openUrl } from "@tauri-apps/plugin-opener";
import { BlinkOverlay } from "./blink-overlay";
import { ImagePreview } from "./image-preview";
import { LinkUnderline, URL_REGEX } from "./link-underline";
import "@xterm/xterm/css/xterm.css";

const COLS = 80;
const ROWS = 24;
const MIN_FONT_SIZE = 8;
const MAX_FONT_SIZE = 72;

export function createTerminal(
  container: HTMLElement,
  fontFamily: string,
  fontSize: number,
): Terminal {
  const term = new Terminal({
    cols: COLS,
    rows: ROWS,
    fontFamily: `"${fontFamily}", monospace`,
    fontSize,
    cursorBlink: true,
    scrollback: 0, // BBS 為全螢幕應用，畫面由 PTT 自行管理
  });
  term.open(container);
  new BlinkOverlay(term); // 簽名檔閃爍字：xterm 不渲染 SGR 5，由 overlay 補動畫
  new LinkUnderline(term); // 網址常駐底線：xterm 的連結底線只在 hover 時顯示
  // 文章內網址點擊以系統預設瀏覽器開啟；預設 handler 是 window.open，在 Tauri WebView 內無效
  const preview = new ImagePreview();
  term.loadAddon(
    new WebLinksAddon((_event, uri) => void openUrl(uri), {
      urlRegex: URL_REGEX, // 與 LinkUnderline 同一條，保證「畫了底線」＝「可點擊」
      hover: (event, uri) => preview.show(uri, event),
      leave: () => preview.hide(),
    }),
  );
  term.onData(() => preview.hide()); // 按鍵後畫面會變，殘留的預覽圖會對不上內容
  return term;
}

export function fitFontSize(term: Terminal): void {
  // 縮放比例對字型大小非嚴格線性，迭代兩次收斂
  for (let i = 0; i < 2; i++) {
    const screen = term.element?.querySelector<HTMLElement>(".xterm-screen");
    if (!screen || screen.clientWidth === 0 || screen.clientHeight === 0) return;
    const scale = Math.min(
      (window.innerWidth - 16) / screen.clientWidth,
      (window.innerHeight - 16) / screen.clientHeight,
    );
    const current = term.options.fontSize ?? 18;
    const next = Math.max(MIN_FONT_SIZE, Math.min(MAX_FONT_SIZE, Math.floor(current * scale)));
    if (next === current) return;
    term.options.fontSize = next;
  }
}
