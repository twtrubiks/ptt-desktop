// PTT 簽名檔的閃爍字（ANSI SGR 5，ESC[5m）：xterm.js 的 parser 有解析並儲存 blink
// 屬性，但 renderer 未實作動畫（上游 InputHandler.ts 自標 FIXME）。這裡以透明 canvas
// 疊在文字層上補動畫：off 相位用該格的「有效背景色」蓋住文字（字消失、背景留著），
// on 相位清空。只在畫面上有閃爍字且視窗可見時跑 timer，其餘時間完全靜止。
import type { IBufferCell, Terminal } from "@xterm/xterm";

const BLINK_INTERVAL_MS = 500; // on/off 各半，1 秒完整週期，與 xterm 游標閃爍同節奏

// 與 xterm 未設 theme 時的預設調色盤一致（@xterm/xterm src/browser/Types.ts 的 DEFAULT_ANSI_COLORS）
const DEFAULT_FOREGROUND = "#ffffff";
const DEFAULT_BACKGROUND = "#000000";
// 兩行排版對應 ANSI 標準色／亮色兩組
// prettier-ignore
const ANSI_16 = [
  "#2e3436", "#cc0000", "#4e9a06", "#c4a000", "#3465a4", "#75507b", "#06989a", "#d3d7cf",
  "#555753", "#ef2929", "#8ae234", "#fce94f", "#729fcf", "#ad7fa8", "#34e2e2", "#eeeeec",
];

function rgbCss(rgb: number): string {
  return `#${rgb.toString(16).padStart(6, "0")}`;
}

function paletteCss(index: number): string {
  if (index < 16) return ANSI_16[index];
  if (index < 232) {
    // 6×6×6 色立方（16–231）
    const v = [0x00, 0x5f, 0x87, 0xaf, 0xd7, 0xff];
    const i = index - 16;
    return rgbCss((v[(i / 36) | 0] << 16) | (v[((i / 6) | 0) % 6] << 8) | v[i % 6]);
  }
  const c = 8 + (index - 232) * 10; // 灰階（232–255）
  return rgbCss((c << 16) | (c << 8) | c);
}

// off 相位的蓋色＝該格的有效背景色。反白格的背景是其前景色；xterm 的粗體亮化只作用
// 於交換「後」的前景、不影響背景（DomRendererRowFactory.ts），所以這裡不看粗體。
export function coverColor(cell: IBufferCell): string {
  if (cell.isInverse()) {
    if (cell.isFgPalette()) return paletteCss(cell.getFgColor());
    if (cell.isFgRGB()) return rgbCss(cell.getFgColor());
    return DEFAULT_FOREGROUND;
  }
  if (cell.isBgPalette()) return paletteCss(cell.getBgColor());
  if (cell.isBgRGB()) return rgbCss(cell.getBgColor());
  return DEFAULT_BACKGROUND;
}

export interface BlinkCell {
  col: number;
  row: number;
  width: number; // 佔的字元格數（全形字為 2）
  css: string; // off 相位蓋上去的顏色
}

// 掃出可視畫面上所有帶 blink 屬性的格子。80×24 的整數旗標檢查，微秒等級。
export function scanBlinkCells(term: Terminal): BlinkCell[] {
  const buffer = term.buffer.active;
  const cell = buffer.getNullCell();
  const found: BlinkCell[] = [];
  for (let row = 0; row < term.rows; row++) {
    const line = buffer.getLine(buffer.viewportY + row);
    if (!line) continue;
    for (let col = 0; col < term.cols; col++) {
      if (!line.getCell(col, cell)) continue;
      if (cell.getWidth() === 0) continue; // 全形字的第二格，由第一格涵蓋
      if (!cell.isBlink()) continue;
      found.push({ col, row, width: cell.getWidth(), css: coverColor(cell) });
    }
  }
  return found;
}

export class BlinkOverlay {
  private readonly canvas = document.createElement("canvas");
  private cells: BlinkCell[] = [];
  private timer: number | null = null;
  private covered = false; // off 相位＝文字被蓋住

  constructor(private readonly term: Terminal) {
    const screen = term.element?.querySelector<HTMLElement>(".xterm-screen");
    if (!screen) throw new Error("BlinkOverlay 需在 term.open() 之後建立");
    this.canvas.style.position = "absolute";
    this.canvas.style.top = "0";
    this.canvas.style.left = "0";
    this.canvas.style.pointerEvents = "none";
    this.canvas.style.zIndex = "4"; // 高於文字列，低於 xterm 的 helpers（5）與 decorations（6）
    screen.appendChild(this.canvas);
    // 每次重繪後重掃：畫面內容變了，閃爍格子的位置與顏色要跟上
    term.onRender(() => this.refresh());
    document.addEventListener("visibilitychange", () => this.syncTimer());
  }

  private refresh(): void {
    this.cells = scanBlinkCells(this.term);
    if (this.covered) this.paint();
    this.syncTimer();
  }

  private syncTimer(): void {
    const need = this.cells.length > 0 && document.visibilityState === "visible";
    if (need && this.timer === null) {
      this.timer = window.setInterval(() => {
        this.covered = !this.covered;
        this.paint();
      }, BLINK_INTERVAL_MS);
    } else if (!need && this.timer !== null) {
      clearInterval(this.timer);
      this.timer = null;
      if (this.covered) {
        this.covered = false;
        this.paint(); // 停下來時要停在「文字可見」的相位
      }
    }
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
    if (!this.covered) return;
    // 以裝置像素取整定位，相鄰格子共用同一條邊界線，不會漏出底下的文字
    const cellW = deviceW / this.term.cols;
    const cellH = deviceH / this.term.rows;
    for (const cell of this.cells) {
      const x0 = Math.round(cell.col * cellW);
      const x1 = Math.round((cell.col + cell.width) * cellW);
      const y0 = Math.round(cell.row * cellH);
      const y1 = Math.round((cell.row + 1) * cellH);
      ctx.fillStyle = cell.css;
      ctx.fillRect(x0, y0, x1 - x0, y1 - y0);
    }
  }
}
