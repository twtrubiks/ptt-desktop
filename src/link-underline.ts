// 網址常駐底線：xterm.js 的連結底線是 hover 裝飾（link API 設計如此），滑過才顯示。
// 這裡每次重繪後掃描可視畫面，用透明 canvas 在網址下畫底線，一眼就能看出哪裡可點。
// 80×24 逐格掃描＋regex，微秒等級；canvas 重畫幾個矩形，無 DOM 增刪。
import type { Terminal } from "@xterm/xterm";

// 與 @xterm/addon-web-links 0.12.0 的預設 regex 相同（套件未匯出，只能複製）。
// terminal.ts 會把這條傳給 WebLinksAddon，保證「畫了底線」與「可點擊」範圍一致。
export const URL_REGEX =
  // oxlint-disable-next-line no-useless-escape -- 逐字複製自套件原始碼，不改寫
  /(https?|HTTPS?):[/]{2}[^\s"'!*(){}|\\\^<>`]*[^\s"':,.!?{}|\\\^~\[\]`()<>]/;

// 同 addon 的 isUrl 過濾：regex 匹配後再用 URL 建構子驗證，行為需一致
function isUrl(urlString: string): boolean {
  try {
    const url = new URL(urlString);
    const parsedBase =
      url.password && url.username
        ? `${url.protocol}//${url.username}:${url.password}@${url.host}`
        : url.username
          ? `${url.protocol}//${url.username}@${url.host}`
          : `${url.protocol}//${url.host}`;
    return urlString.toLocaleLowerCase().startsWith(parsedBase.toLocaleLowerCase());
  } catch {
    return false;
  }
}

export interface LinkSpan {
  row: number; // 可視區列，0-based
  col: number; // 起始格
  width: number; // 佔的字元格數
}

// 掃出可視畫面上所有網址的格子範圍。逐行處理即可：PTT 是 server 端 80 欄硬換行，
// 不會出現 isWrapped 的軟換行，跨行網址本來就連 hover 也點不到（既有限制）。
export function scanLinkSpans(term: Terminal): LinkSpan[] {
  const buffer = term.buffer.active;
  const cell = buffer.getNullCell();
  const found: LinkSpan[] = [];
  const rex = new RegExp(URL_REGEX.source, "g");
  for (let row = 0; row < term.rows; row++) {
    const line = buffer.getLine(buffer.viewportY + row);
    if (!line) continue;
    // 組出整行字串，並記下每個字串索引（UTF-16 code unit）對應的格位與格寬，
    // 全形字佔 2 格，字串索引不能直接當欄位用
    let text = "";
    const cols: number[] = [];
    const widths: number[] = [];
    for (let col = 0; col < term.cols; col++) {
      if (!line.getCell(col, cell)) continue;
      const width = cell.getWidth();
      if (width === 0) continue; // 全形字的第二格，由第一格涵蓋
      const chars = cell.getChars() || " "; // 空格子補空白，確保索引對位
      for (let i = 0; i < chars.length; i++) {
        text += chars[i];
        cols.push(col);
        widths.push(width);
      }
    }
    let match;
    while ((match = rex.exec(text))) {
      if (!isUrl(match[0])) continue;
      const last = match.index + match[0].length - 1;
      const start = cols[match.index];
      found.push({ row, col: start, width: cols[last] + widths[last] - start });
    }
  }
  return found;
}

const UNDERLINE_COLOR = "rgba(255, 255, 255, 0.6)";

export class LinkUnderline {
  private readonly canvas = document.createElement("canvas");

  constructor(private readonly term: Terminal) {
    const screen = term.element?.querySelector<HTMLElement>(".xterm-screen");
    if (!screen) throw new Error("LinkUnderline 需在 term.open() 之後建立");
    this.canvas.style.position = "absolute";
    this.canvas.style.top = "0";
    this.canvas.style.left = "0";
    this.canvas.style.pointerEvents = "none";
    this.canvas.style.zIndex = "3"; // 高於文字列，低於 BlinkOverlay（4）：閃爍蓋色時底線一併被蓋
    screen.appendChild(this.canvas);
    term.onRender(() => this.paint());
  }

  private paint(): void {
    const screen = this.canvas.parentElement;
    const ctx = this.canvas.getContext("2d");
    if (!screen || !ctx) return;
    const width = screen.clientWidth;
    const height = screen.clientHeight;
    if (width === 0 || height === 0) return;
    // 字型縮放後畫面尺寸會變，畫之前對齊一次；canvas 改尺寸時內容自動清空
    const dpr = window.devicePixelRatio || 1;
    const deviceW = Math.round(width * dpr);
    const deviceH = Math.round(height * dpr);
    if (this.canvas.width !== deviceW || this.canvas.height !== deviceH) {
      this.canvas.width = deviceW;
      this.canvas.height = deviceH;
      this.canvas.style.width = `${width}px`;
      this.canvas.style.height = `${height}px`;
    }
    ctx.clearRect(0, 0, deviceW, deviceH);
    const cellW = deviceW / this.term.cols;
    const cellH = deviceH / this.term.rows;
    const thickness = Math.max(1, Math.round(dpr));
    ctx.fillStyle = UNDERLINE_COLOR;
    for (const span of scanLinkSpans(this.term)) {
      const x0 = Math.round(span.col * cellW);
      const x1 = Math.round((span.col + span.width) * cellW);
      const y = Math.round((span.row + 1) * cellH) - thickness;
      ctx.fillRect(x0, y, x1 - x0, thickness);
    }
  }
}
