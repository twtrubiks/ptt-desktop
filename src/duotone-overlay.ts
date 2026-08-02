// 雙色字疊繪層：duotone.ts 把雙色字拆成 Plane 15/16 PUA 半字後，xterm 對這些字元
// 只會畫 .notdef（字型無 glyph）。本層仿 blink-overlay 的管線：onRender 後掃 buffer
// 找 PUA 半字，以該格自身的 fg/bg 把原字形的左半／右半疊繪回去——先鋪背景蓋掉
// .notdef，再畫整字並 clip 出半格；左右半共用同一繪製原點，接縫零誤差。
// 方塊與三角（彩繪主力字元）以幾何自繪：整數像素 fillRect 保證滿格無縫、半格 clip
// 邊緣不受字型 AA 影響（2026-07-14「Noto CJK 缺 U+2581」的觀察係當時未捆綁字型的
// 環境所致；捆綁的 PTT Square Mono 實測完整含 U+2500–259F，tests/font-coverage.test.ts 守門）。
// 其餘字元（漢字、CJK 符號）用字型繪製。
// 內容隨畫面靜止，不需 timer。z-index 與 blink-overlay 同為 4，但本層必須先掛載
// （後掛載的同層 sibling 疊在上面），blink 的 off 相位才蓋得住會閃的雙色字。
import type { IBufferCell, Terminal } from "@xterm/xterm";
import {
  coverColor,
  DEFAULT_BACKGROUND,
  DEFAULT_FOREGROUND,
  paletteCss,
  rgbCss,
} from "./blink-overlay";
import { LEFT_BASE, RIGHT_BASE } from "./duotone";

// 顯示的前景色：反白格顯示的是交換後的背景色；粗體亮化作用於交換「後」的前景
// （調色盤 0-7 → 8-15，xterm 的 drawBoldTextInBrightColors 預設 true）
function fgCss(cell: IBufferCell): string {
  const palette = cell.isInverse() ? cell.isBgPalette() : cell.isFgPalette();
  const rgb = cell.isInverse() ? cell.isBgRGB() : cell.isFgRGB();
  const color = cell.isInverse() ? cell.getBgColor() : cell.getFgColor();
  if (palette) return paletteCss(cell.isBold() && color < 8 ? color + 8 : color);
  if (rgb) return rgbCss(color);
  return cell.isInverse() ? DEFAULT_BACKGROUND : DEFAULT_FOREGROUND;
}

export interface DuotoneCell {
  col: number;
  row: number;
  cp: number; // 原字 codepoint
  side: 0 | 1; // 0=左半、1=右半
  fg: string;
  bg: string;
}

// 掃出可視畫面上所有 PUA 半字。與 scanBlinkCells 同級的整數比較，微秒等級
export function scanDuotoneCells(term: Terminal): DuotoneCell[] {
  const buffer = term.buffer.active;
  const cell = buffer.getNullCell();
  const found: DuotoneCell[] = [];
  for (let row = 0; row < term.rows; row++) {
    const line = buffer.getLine(buffer.viewportY + row);
    if (!line) continue;
    for (let col = 0; col < term.cols; col++) {
      if (!line.getCell(col, cell)) continue;
      const chars = cell.getChars();
      if (!chars) continue;
      const cp = chars.codePointAt(0) as number;
      if (cp < LEFT_BASE) continue;
      const side: 0 | 1 = cp >= RIGHT_BASE ? 1 : 0;
      found.push({
        col,
        row,
        cp: cp - (side ? RIGHT_BASE : LEFT_BASE),
        side,
        fg: fgCss(cell),
        bg: coverColor(cell),
      });
    }
  }
  return found;
}

// 象限方塊 U+2596-259F 的填格遮罩，bit 順序 [左上, 右上, 左下, 右下]
// prettier-ignore
const QUADRANTS: readonly number[] = [
  0b0010, 0b0001, 0b1000, 0b1011, 0b1001, 0b1110, 0b1101, 0b0100, 0b0110, 0b0111,
];

// 幾何自繪：以「整字外框（左右兩格）」為座標畫滿格幾何，呼叫端已 clip 出半格。
// 回傳 false 表示非幾何字元，交給字型
function drawGeometry(
  ctx: CanvasRenderingContext2D,
  cp: number,
  x: number,
  y: number,
  w: number,
  h: number,
): boolean {
  if (cp === 0x2580) {
    ctx.fillRect(x, y, w, h / 2); // ▀ 上半
  } else if (cp >= 0x2581 && cp <= 0x2588) {
    const k = (cp - 0x2580) / 8; // ▁▂▃▄▅▆▇█ 下方 k/8
    ctx.fillRect(x, y + h * (1 - k), w, h * k);
  } else if (cp >= 0x2589 && cp <= 0x258f) {
    const k = (0x2590 - cp) / 8; // ▉▊▋▌▍▎▏ 左方 k/8
    ctx.fillRect(x, y, w * k, h);
  } else if (cp === 0x2590) {
    ctx.fillRect(x + w / 2, y, w / 2, h); // ▐ 右半
  } else if (cp >= 0x2591 && cp <= 0x2593) {
    ctx.save(); // ░▒▓ 以透明度近似 25/50/75% 網點
    ctx.globalAlpha = (cp - 0x2590) * 0.25;
    ctx.fillRect(x, y, w, h);
    ctx.restore();
  } else if (cp === 0x2594) {
    ctx.fillRect(x, y, w, h / 8); // ▔ 上 1/8
  } else if (cp === 0x2595) {
    ctx.fillRect(x + (w * 7) / 8, y, w / 8, h); // ▕ 右 1/8
  } else if (cp >= 0x2596 && cp <= 0x259f) {
    const mask = QUADRANTS[cp - 0x2596];
    if (mask & 0b1000) ctx.fillRect(x, y, w / 2, h / 2);
    if (mask & 0b0100) ctx.fillRect(x + w / 2, y, w / 2, h / 2);
    if (mask & 0b0010) ctx.fillRect(x, y + h / 2, w / 2, h / 2);
    if (mask & 0b0001) ctx.fillRect(x + w / 2, y + h / 2, w / 2, h / 2);
  } else if (cp >= 0x25e2 && cp <= 0x25e5) {
    // ◢◣◤◥ 實心直角三角形
    ctx.beginPath();
    if (cp === 0x25e2) {
      ctx.moveTo(x + w, y);
      ctx.lineTo(x + w, y + h);
      ctx.lineTo(x, y + h);
    } else if (cp === 0x25e3) {
      ctx.moveTo(x, y);
      ctx.lineTo(x + w, y + h);
      ctx.lineTo(x, y + h);
    } else if (cp === 0x25e4) {
      ctx.moveTo(x, y);
      ctx.lineTo(x + w, y);
      ctx.lineTo(x, y + h);
    } else {
      ctx.moveTo(x, y);
      ctx.lineTo(x + w, y);
      ctx.lineTo(x + w, y + h);
    }
    ctx.closePath();
    ctx.fill();
  } else {
    return false;
  }
  return true;
}

export class DuotoneOverlay {
  private readonly canvas = document.createElement("canvas");

  constructor(private readonly term: Terminal) {
    const screen = term.element?.querySelector<HTMLElement>(".xterm-screen");
    if (!screen) throw new Error("DuotoneOverlay 需在 term.open() 之後建立");
    this.canvas.style.position = "absolute";
    this.canvas.style.top = "0";
    this.canvas.style.left = "0";
    this.canvas.style.pointerEvents = "none";
    this.canvas.style.zIndex = "4"; // 高於文字列，低於 xterm 的 helpers（5）與 decorations（6）
    screen.appendChild(this.canvas);
    // onWriteParsed:資料進 buffer 就重畫,不依賴 renderer 的重繪時機(實測 headless
    // 環境 onRender 可能只觸發一次);onRender 補渲染驅動的更新;ResizeObserver 補
    // 字型縮放(fitFontSize)造成的畫面尺寸變化
    term.onWriteParsed(() => this.paint());
    term.onRender(() => this.paint());
    new ResizeObserver(() => this.paint()).observe(screen);
  }

  private paint(): void {
    const screen = this.canvas.parentElement;
    const ctx = this.canvas.getContext("2d");
    if (!screen || !ctx) return;
    const width = screen.clientWidth;
    const height = screen.clientHeight;
    if (width === 0 || height === 0) return;
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
    const cells = scanDuotoneCells(this.term);
    if (cells.length === 0) return;

    const cellW = deviceW / this.term.cols;
    const cellH = deviceH / this.term.rows;
    // 字級以全形 advance 校準到剛好 2 格寬（xterm 的格寬含取整與 letterSpacing，
    // 不必然等於 fontSize），基線取「字面在格內垂直置中」
    const family = this.term.options.fontFamily ?? "monospace";
    const base = (this.term.options.fontSize ?? 18) * dpr;
    ctx.font = `${base}px ${family}`;
    const advance = ctx.measureText("█").width || base;
    const size = (base * 2 * cellW) / advance;
    ctx.font = `${size}px ${family}`;
    const m = ctx.measureText("█");
    const ascent = m.fontBoundingBoxAscent ?? size * 0.8;
    const descent = m.fontBoundingBoxDescent ?? size * 0.2;
    const baselineOffset = (cellH - (ascent + descent)) / 2 + ascent;
    ctx.textBaseline = "alphabetic";

    for (const cell of cells) {
      // 以裝置像素取整定位，相鄰格共用同一條邊界線（同 blink-overlay）
      const x0 = Math.round(cell.col * cellW);
      const x1 = Math.round((cell.col + 1) * cellW);
      const y0 = Math.round(cell.row * cellH);
      const y1 = Math.round((cell.row + 1) * cellH);
      // 整字外框＝從左半格左緣起兩格寬；左右半共用同一原點與外框，接縫無誤差
      const gx = Math.round((cell.col - cell.side) * cellW);
      const gx1 = Math.round((cell.col - cell.side + 2) * cellW);
      ctx.fillStyle = cell.bg;
      ctx.fillRect(x0, y0, x1 - x0, y1 - y0);
      ctx.save();
      ctx.beginPath();
      ctx.rect(x0, y0, x1 - x0, y1 - y0);
      ctx.clip();
      ctx.fillStyle = cell.fg;
      if (!drawGeometry(ctx, cell.cp, gx, y0, gx1 - gx, y1 - y0)) {
        // 不套合成粗體：捆綁字型無 Bold face，合成出的字形會被格邊界裁出 1px 暗線
        // （第八根因），且 xterm 側已設 fontWeightBold: "normal"，此處跟著才不會讓
        // 同一個字「是不是雙色字」粗細不同。亮色由 cell.fg 決定，與字重無關
        ctx.font = `${size}px ${family}`;
        ctx.fillText(String.fromCodePoint(cell.cp), gx, y0 + baselineOffset);
      }
      ctx.restore();
    }
  }
}
