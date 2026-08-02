// xterm.js 初始化：固定 80x24 字元格；視窗縮放只調字型大小，不改行列數。
import { Terminal } from "@xterm/xterm";
import { WebglAddon } from "@xterm/addon-webgl";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ANSI_16, BlinkOverlay, DEFAULT_BACKGROUND, DEFAULT_FOREGROUND } from "./blink-overlay";
import { restoreDuotoneText } from "./duotone";
import { DuotoneOverlay } from "./duotone-overlay";
import { ImagePreview } from "./image-preview";
import { LinkUnderline, URL_REGEX } from "./link-underline";
import { PTT_UNICODE_VERSION, PttUnicodeProvider } from "./unicode-ptt";
import "@xterm/xterm/css/xterm.css";

const COLS = 80;
const ROWS = 24;
const MIN_FONT_SIZE = 8;
const MAX_FONT_SIZE = 72;

export function createTerminal(
  container: HTMLElement,
  fontFamily: string,
  fontSize: number,
  lineHeight: number,
): Terminal {
  const term = new Terminal({
    cols: COLS,
    rows: ROWS,
    fontFamily: `"${fontFamily}", monospace`,
    fontSize,
    // 呼叫端須確保 >= 1（config.ts 的 sanitizeLineHeight），xterm 對 < 1 會拋例外。
    // > 1 時 cell 變高、字形在格內垂直置中（webgl 的 char.top），三個 overlay 都以
    // deviceH / rows 動態算格高故自動跟上；但方塊字形固定 1em，彩繪列間會露縫
    lineHeight,
    // 捆綁字型 PTT Square Mono 只有 Regular，SGR 1 會讓引擎「合成粗體」：字形被撐大
    // 後在字形圖集邊界被裁掉，滿格方塊的右緣與底緣各露一條 1px 暗線，彩繪整片變磁磚
    // （第八根因，WebKitGTK／Blink 兩引擎實測相同；改為 normal 後接縫歸零）。
    // BBS 的 SGR 1 語意本來就是「選亮色」而非加粗，亮色映射由 drawBoldTextInBrightColors
    // （預設 true）負責，與本選項互不相干，改此值不影響顏色
    fontWeightBold: "normal",
    cursorBlink: true,
    scrollback: 0, // BBS 為全螢幕應用，畫面由 PTT 自行管理
    allowProposedApi: true, // term.unicode(自訂字寬 provider)是 proposed API
    // webgl 的 customGlyphs 自繪一律只畫單格寬（addon-webgl 只把單格 deviceCellWidth
    // 傳給 tryDrawCustomChar），而 PTT 的框線/方塊經 provider 報 2 格 → 右半格永遠
    // 空白：分隔線 U+2500 成虛線、方塊彩繪梳齒碎裂（第六根因）。捆綁字型完整含
    // U+2500–259F 全形字形（tests/font-coverage.test.ts 守門；方塊墨水滿行高箱為
    // fonttools 一次性實測，回歸交手動 e2e），字型路徑的光柵化允許 2 格寬，
    // 關閉自繪改與漢字同路徑滿版繪製
    customGlyphs: false,
    // PTT 傳統 VGA 色盤（單一出處在 blink-overlay.ts，overlay 疊繪同表）：
    // xterm 預設 Tango 的「黑」#2e3436 與預設背景 #000000 不同色，SGR 40 黑底
    // 彩繪會浮出灰黑拼布；BBS 慣例明確黑底＝預設背景、預設前景＝color 7 的
    // #c0c0c0（第七根因，與 term.ptt.cc／PttChrome 對齊）
    theme: {
      foreground: DEFAULT_FOREGROUND,
      background: DEFAULT_BACKGROUND,
      black: ANSI_16[0],
      red: ANSI_16[1],
      green: ANSI_16[2],
      yellow: ANSI_16[3],
      blue: ANSI_16[4],
      magenta: ANSI_16[5],
      cyan: ANSI_16[6],
      white: ANSI_16[7],
      brightBlack: ANSI_16[8],
      brightRed: ANSI_16[9],
      brightGreen: ANSI_16[10],
      brightYellow: ANSI_16[11],
      brightBlue: ANSI_16[12],
      brightMagenta: ANSI_16[13],
      brightCyan: ANSI_16[14],
      brightWhite: ANSI_16[15],
    },
  });
  // PTT 版面以 Big5 語意編排:方塊、框線等 Ambiguous 字元必須佔 2 格,否則彩繪跑版
  term.unicode.register(new PttUnicodeProvider());
  term.unicode.activeVersion = PTT_UNICODE_VERSION;
  term.open(container);
  // webgl renderer：DOM renderer 逐 <span> 排版，實心方塊在拼接處露出黑縫；
  // webgl 以 glyph quad 整數格對齊平鋪無縫（customGlyphs 已停用，見上，字形一律
  // 走字型光柵化）。純視覺強化：WebKitGTK 拿不到 WebGL context 時退回 DOM
  // renderer（彩繪有細縫但功能完好），絕不中斷連線主流程
  try {
    const webgl = new WebglAddon();
    // context 丟失（GPU 重置、驅動異常）即釋放，xterm 自動退回 DOM renderer
    webgl.onContextLoss(() => webgl.dispose());
    term.loadAddon(webgl);
  } catch (error) {
    console.warn("WebGL renderer 啟用失敗，退回 DOM renderer：", error);
  }
  // 雙色字疊繪須先掛載（同 z-index 下後掛載者在上），blink 的 off 相位才蓋得住雙色字
  new DuotoneOverlay(term); // 雙色字：拆成 PUA 半字存 buffer，由 overlay 繪回左右異色字形
  new BlinkOverlay(term); // 簽名檔閃爍字：xterm 不渲染 SGR 5，由 overlay 補動畫
  // 選取複製時把 PUA 半字還原成原字（capture 階段，先於 xterm 自身的 copy 處理）
  container.addEventListener(
    "copy",
    (event) => {
      if (!term.hasSelection() || !event.clipboardData) return;
      event.clipboardData.setData("text/plain", restoreDuotoneText(term.getSelection()));
      event.preventDefault();
    },
    true,
  );
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
