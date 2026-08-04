// byte 網格 shadow（Plans.md P5 建模、P6 接線；語意規格見 docs/architecture.md「byte 網格語意規格」）。
// 觀察 telnet 濾層後的原始下行 bytes（與 autoLogin.feedBytes 同點），
// 以 PTT 的「一欄半字」語意維護 80×24 網格：每欄獨立持有窄字元或寬字元的
// 左半／右半（含各自的 SGR），覆寫只動被寫的欄、partner 欄降為孤兒半字。
// xterm 的原子寬字元模型會把被劈 pair 的另一半整格清空，feed() 回報
// 「受影響且仍為孤兒」的半格，由 main.ts 以 PUA 半字補寫（duotone.buildOrphanRepair）。
// 設計裁決：修復式（觀察＋補寫，現有管線零改動）——P4 實錄顯示損傷為暫態、
// 段末兩模型收斂，重寫式的取代面不值得。
// feed() 同時回傳轉發用串流：行尾虛位（col=80）的 BS 正規化為絕對 CUP——
// xterm 的 BS 是 pending-wrap 語意「夾 79 再減」（InputHandler backspace 先
// _restrictCursor 再減），與 byte 網格意圖「純減一」差一欄，實錄 31 次全數
// 落在動畫文法「BS×2 後寫寬字」，錯位寬字＋行尾殘留為可見損傷（P6 量測：
// 62 格、遍及 23 列）；在源頭替換掉 BS 讓兩游標語意其後恆一致。
import { WIDE_RANGES, ZERO_RANGES } from "./wcwidth-data";

const ROWS = 24;
const COLS = 80;

// 與 unicode-ptt.ts 相同的二分搜尋（資料表共用保證寬度判定與 xterm 一致；
// 模組不得互相依賴，12 行小函式以複製取代 import）
function inRanges(cp: number, table: readonly number[]): boolean {
  let lo = 0;
  let hi = table.length / 2 - 1;
  if (cp < table[0] || cp > table[table.length - 1]) return false;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (cp > table[mid * 2 + 1]) lo = mid + 1;
    else if (cp < table[mid * 2]) hi = mid - 1;
    else return true;
  }
  return false;
}

function wcwidth(cp: number): 0 | 1 | 2 {
  if (cp < 32) return 0;
  if (cp < 127) return 1;
  if (cp >= 0xf0000) return 1; // PUA 半字（正常串流不會出現，防禦性一致）
  if (inRanges(cp, ZERO_RANGES)) return 0;
  if (inRanges(cp, WIDE_RANGES)) return 2;
  return 1;
}

// SGR 打包：bit0 bold / bit1 blink / bit2 inverse / bit3-6 fg+1（0=預設）/ bit7-10 bg+1
const SGR_DEFAULT = 0;

export interface RepairCell {
  row: number; // 0-based
  col: number; // 0-based
  glyph: string; // 原完整字
  side: "left" | "right";
  fg: number; // 0-7，-1 = 預設
  bg: number;
  bold: boolean;
  blink: boolean;
  inverse: boolean;
}

export interface FeedResult {
  /** 轉發給下游（sgr-reorder → codec → xterm）的串流：BS@80 已正規化為絕對 CUP，
   *  其餘 bytes 原樣；本批無替換時直接回傳原陣列（零拷貝） */
  output: Uint8Array;
  /** 受影響且批末仍為孤兒、此刻可安全補寫的半格。批末游標在行尾虛位（col=80，
   *  DECRC 會把 x=80 夾回 79、丟失 pending-wrap）或 ESC/CSI 序列中途（注入會
   *  劈壞跨批序列）時回傳空陣列並保留 dirty，遞延到後續批一併回報 */
  repairs: RepairCell[];
}

export interface CellInfo {
  glyph: string; // 完整字（寬字兩欄同值）；窄字元本身；未配對 big5 byte 為 ""
  half: 0 | 1 | 2; // 0=窄/空, 1=寬字左半, 2=寬字右半
  byte: number; // big5 模式未配對 byte 值，其餘 -1
  paired: boolean; // half≠0 且相鄰欄構成同一個字
  fg: number;
  bg: number;
  bold: boolean;
  blink: boolean;
  inverse: boolean;
}

const ST_TEXT = 0;
const ST_ESC = 1;
const ST_CSI = 2;

export class ScreenGrid {
  private glyphs: string[] = Array.from({ length: ROWS * COLS }, () => " ");
  private halves = new Uint8Array(ROWS * COLS);
  private bytes = new Int16Array(ROWS * COLS).fill(-1);
  private sgrs = new Uint16Array(ROWS * COLS);

  private row = 0;
  private col = 0; // 可為 COLS（行尾虛位，下一個可印字元才換行）
  private sgr = SGR_DEFAULT;
  private utf8Mode = false;

  private escState = ST_TEXT;
  private csiParams = "";

  // UTF-8 多位元組累積（可跨 chunk；中間夾 ESC 序列不中斷——SGR 劈字）
  private u8Need = 0;
  private u8Have = 0;
  private u8Cp = 0;
  private u8StartSgr = SGR_DEFAULT; // 字元第一個 byte 當下的 SGR（左半用）

  // big5 lead byte 等待 trail 配對（SGR 不中斷、游標移動即失效）
  private leadIdx = -1;
  private leadByte = 0;

  private dirty = new Set<number>();
  private big5 = new TextDecoder("big5");

  /** 餵一批下行 bytes（telnet 濾層後），回傳轉發串流（BS@80 正規化）與可補寫孤兒 */
  feed(data: Uint8Array): FeedResult {
    let subst: number[] | null = null; // BS@80 替換時才建（常態零替換、零拷貝）
    let copied = 0;
    for (let i = 0; i < data.length; i++) {
      const b = data[i];
      // BS@80 → 絕對 CUP（同列 80 欄＝純減一的落點）。u8Need/leadIdx 進行中
      // 表示 BS 劈在多位元組字中間（實錄不存在）：此時不替換——插入的 ASCII
      // 會讓 codec 端解出 U+FFFD，寧可退回 xterm 原生 BS 行為
      if (
        b === 0x08 &&
        this.escState === ST_TEXT &&
        this.col === COLS &&
        this.u8Need === 0 &&
        this.leadIdx < 0
      ) {
        if (!subst) subst = [];
        for (; copied < i; copied++) subst.push(data[copied]);
        const cup = `\x1b[${this.row + 1};${COLS}H`;
        for (let k = 0; k < cup.length; k++) subst.push(cup.charCodeAt(k));
        copied = i + 1; // 跳過原 BS byte
      }
      this.step(b);
    }
    let output: Uint8Array;
    if (subst === null) {
      output = data;
    } else {
      for (; copied < data.length; copied++) subst.push(data[copied]);
      output = Uint8Array.from(subst);
    }
    const injectSafe = this.escState === ST_TEXT && this.col < COLS;
    return { output, repairs: injectSafe ? this.collectRepairs() : [] };
  }

  /** 與 codec 相同時機呼叫（送出密碼後） */
  switchToUtf8(): void {
    this.utf8Mode = true;
    this.u8Need = this.u8Have = this.u8Cp = 0;
    this.leadIdx = -1;
  }

  reset(): void {
    this.glyphs.fill(" ");
    this.halves.fill(0);
    this.bytes.fill(-1);
    this.sgrs.fill(SGR_DEFAULT);
    this.row = this.col = 0;
    this.sgr = SGR_DEFAULT;
    this.utf8Mode = false;
    this.escState = ST_TEXT;
    this.csiParams = "";
    this.u8Need = this.u8Have = this.u8Cp = 0;
    this.leadIdx = -1;
    this.dirty.clear();
  }

  get cursor(): { row: number; col: number } {
    return { row: this.row, col: Math.min(this.col, COLS - 1) };
  }

  cellAt(row: number, col: number): CellInfo {
    const i = row * COLS + col;
    const s = this.sgrs[i];
    return {
      glyph: this.glyphs[i],
      half: this.halves[i] as 0 | 1 | 2,
      byte: this.bytes[i],
      paired: this.pairedAt(i),
      fg: ((s >> 3) & 0xf) - 1,
      bg: ((s >> 7) & 0xf) - 1,
      bold: (s & 1) !== 0,
      blink: (s & 2) !== 0,
      inverse: (s & 4) !== 0,
    };
  }

  /** 配對後的一列內容（內容檢視用，不保證欄位對齊；孤兒與未配對 byte 以 ? 呈現） */
  rowText(row: number): string {
    let out = "";
    for (let c = 0; c < COLS;) {
      const i = row * COLS + c;
      if (this.pairedAt(i)) {
        out += this.glyphs[i];
        c += 2;
      } else if (this.halves[i] !== 0 || this.bytes[i] >= 0x80) {
        out += "?";
        c += 1;
      } else {
        out += this.bytes[i] >= 0 ? String.fromCharCode(this.bytes[i]) : this.glyphs[i];
        c += 1;
      }
    }
    return out;
  }

  // ---- byte 層狀態機 ----

  private step(b: number): void {
    if (this.escState === ST_ESC) {
      // 非 CSI 的 ESC 序列視為兩 byte 結束——PTT 實錄的 VT 子集除 CSI 外只有
      // ESC M（RI，文章往上捲頁）。注意：本狀態機不認得 OSC/DCS/APC 等字串序列；
      // 若未來串流真的出現，須先擴充這裡再依賴 injectSafe（否則批末誤判安全、
      // 補寫劈進序列中途）
      this.escState = b === 0x5b ? ST_CSI : ST_TEXT;
      if (this.escState === ST_CSI) this.csiParams = "";
      else if (b === 0x4d) {
        this.reverseIndex();
        this.leadIdx = -1;
      }
      return;
    }
    if (this.escState === ST_CSI) {
      if (b >= 0x30 && b <= 0x3f) {
        this.csiParams += String.fromCharCode(b);
        return;
      }
      if (b >= 0x20 && b <= 0x2f) return; // intermediate bytes，PTT 不用，吃掉
      this.escState = ST_TEXT;
      if (b >= 0x40 && b <= 0x7e) {
        this.dispatchCsi(String.fromCharCode(b));
        return;
      }
      // 非法終止 byte：放回一般路徑
    }
    if (b === 0x1b) {
      this.escState = ST_ESC;
      return; // UTF-8 累積與 big5 lead 都保留——SGR 可劈在字中間
    }
    if (b === 0x0d) {
      this.col = 0;
      this.leadIdx = -1;
      return;
    }
    if (b === 0x0a) {
      this.lineFeed();
      this.leadIdx = -1;
      return;
    }
    if (b === 0x08) {
      // 行尾虛位（col=80）採 byte 網格意圖「純減一」（→79）。xterm 實際是
      // pending-wrap 語意「夾 79 再減」（→78），實錄 BS@80 共 31 次，xterm 端
      // 行尾寫入因此整段左移一欄——屬孤兒回報表達不了的錯位損傷，P6 接線處理
      this.col = Math.max(0, Math.min(this.col, COLS) - 1);
      this.leadIdx = -1;
      return;
    }
    if (b < 0x20 || b === 0x7f) return; // 其餘 C0（BEL 等）忽略，不動配對狀態
    if (this.utf8Mode) this.stepUtf8(b);
    else this.stepBig5(b);
  }

  private stepUtf8(b: number): void {
    if (this.u8Need > 0) {
      if (b >= 0x80 && b <= 0xbf) {
        this.u8Cp = (this.u8Cp << 6) | (b & 0x3f);
        if (++this.u8Have === this.u8Need) {
          const cp = this.u8Cp;
          this.u8Need = 0;
          this.emitChar(cp);
        }
        return;
      }
      // 序列斷裂：以 U+FFFD 呈現（與 TextDecoder non-fatal 同精神），原 byte 重新處理
      this.u8Need = 0;
      this.emitChar(0xfffd);
      this.stepUtf8(b);
      return;
    }
    if (b < 0x80) {
      this.u8StartSgr = this.sgr;
      this.emitChar(b);
      return;
    }
    if (b >= 0xc2 && b <= 0xdf) this.startU8(b & 0x1f, 2);
    else if (b >= 0xe0 && b <= 0xef) this.startU8(b & 0x0f, 3);
    else if (b >= 0xf0 && b <= 0xf4) this.startU8(b & 0x07, 4);
    else {
      this.u8StartSgr = this.sgr;
      this.emitChar(0xfffd); // 孤兒 continuation / 非法 starter
    }
  }

  private startU8(bits: number, need: number): void {
    this.u8Cp = bits;
    this.u8Need = need;
    this.u8Have = 1;
    this.u8StartSgr = this.sgr; // 左半的 SGR 在字元起點取樣
  }

  private stepBig5(b: number): void {
    // trail 配對：lead 之後（允許夾 SGR）下一個資料 byte 落在 lead 右鄰欄
    if (
      this.leadIdx >= 0 &&
      ((b >= 0x40 && b <= 0x7e) || (b >= 0xa1 && b <= 0xfe)) &&
      this.row * COLS + this.col === this.leadIdx + 1 &&
      this.col < COLS
    ) {
      const glyph = this.big5.decode(Uint8Array.from([this.leadByte, b]));
      const li = this.leadIdx;
      this.leadIdx = -1;
      this.orphanPartnerOf(li + 1);
      this.glyphs[li] = glyph;
      this.halves[li] = 1;
      this.bytes[li] = -1; // lead 欄的 sgr 維持 lead byte 寫入當下的值（duotone）
      this.glyphs[li + 1] = glyph;
      this.halves[li + 1] = 2;
      this.bytes[li + 1] = -1;
      this.sgrs[li + 1] = this.sgr;
      this.dirty.add(li + 1);
      this.col++;
      return;
    }
    this.leadIdx = -1;
    if (b < 0x80) {
      this.putNarrow(String.fromCharCode(b), this.sgr);
      return;
    }
    // 未配對的 big5 byte：byte 網格語意，先佔一欄；0x81-0xFE 掛起等 trail
    this.wrapIfNeeded(1);
    const i = this.row * COLS + this.col;
    this.orphanPartnerOf(i);
    this.glyphs[i] = "";
    this.halves[i] = 0;
    this.bytes[i] = b;
    this.sgrs[i] = this.sgr;
    this.dirty.add(i);
    if (b >= 0x81) {
      this.leadIdx = i;
      this.leadByte = b;
    }
    this.col++;
  }

  private emitChar(cp: number): void {
    if (cp > 0x10ffff) cp = 0xfffd; // 非法 4-byte 序列，fromCodePoint 會拋
    const w = wcwidth(cp);
    if (w === 0) return; // 組合字元：PTT 不會送，忽略（xterm 併入前一格）
    const ch = String.fromCodePoint(cp);
    if (w === 1) this.putNarrow(ch, this.u8StartSgr);
    else this.putWide(ch, this.u8StartSgr, this.sgr); // SGR 劈字時右半取字尾當下值
  }

  private wrapIfNeeded(width: number): void {
    if (this.col + width > COLS) {
      this.lineFeed();
      this.col = 0;
    }
  }

  // LF／自動換行：末列時整個網格上捲一列（xterm lineFeed 在 scrollBottom 呼叫
  // bufferService.scroll，scrollback:0 → 捲出的列直接丟棄）。PTT 往下捲頁正是
  // 「CUP 到 24 列 + 連續 LF」，shadow 若夾住不捲，兩模型會整段錯列，
  // 其後的孤兒判定全部落在舊畫面上（實錄：畫面首欄被補出憑空半字）
  private lineFeed(): void {
    if (this.row < ROWS - 1) {
      this.row++;
      return;
    }
    this.glyphs.copyWithin(0, COLS);
    this.halves.copyWithin(0, COLS);
    this.bytes.copyWithin(0, COLS);
    this.sgrs.copyWithin(0, COLS);
    this.clearRow((ROWS - 1) * COLS);
    this.shiftPending(-COLS);
  }

  // RI（ESC M）：頂列時整個網格下捲一列（xterm reverseIndex 在 scrollTop 會
  // shiftElements 把行下移、頂端插入新行）。PTT 文章往上捲頁的文法是
  // 「ESC[24;1H ESC[J ESC[H ESC M」，與末列 LF 的往下捲頁互為鏡像；
  // 漏掉任一邊，shadow 都會停在舊畫面而把捲走的半字補寫到新畫面上
  private reverseIndex(): void {
    if (this.row > 0) {
      this.row--;
      return;
    }
    const span = (ROWS - 1) * COLS;
    this.glyphs.copyWithin(COLS, 0, span);
    this.halves.copyWithin(COLS, 0, span);
    this.bytes.copyWithin(COLS, 0, span);
    this.sgrs.copyWithin(COLS, 0, span);
    this.clearRow(0);
    this.shiftPending(COLS);
  }

  private clearRow(base: number): void {
    for (let i = base; i < base + COLS; i++) {
      this.glyphs[i] = " ";
      this.halves[i] = 0;
      this.bytes[i] = -1;
      this.sgrs[i] = this.sgr; // BCE：與 erase() 同樣以當前屬性清
    }
  }

  // 捲動後待補寫的孤兒索引與掛起的 lead 隨內容平移；移出畫面者作廢
  private shiftPending(delta: number): void {
    if (this.dirty.size > 0) {
      const moved = new Set<number>();
      for (const i of this.dirty) {
        const j = i + delta;
        if (j >= 0 && j < ROWS * COLS) moved.add(j);
      }
      this.dirty = moved;
    }
    const lead = this.leadIdx + delta;
    this.leadIdx = this.leadIdx >= 0 && lead >= 0 && lead < ROWS * COLS ? lead : -1;
  }

  private putNarrow(ch: string, sgr: number): void {
    this.wrapIfNeeded(1);
    const i = this.row * COLS + this.col;
    this.orphanPartnerOf(i);
    this.glyphs[i] = ch;
    this.halves[i] = 0;
    this.bytes[i] = -1;
    this.sgrs[i] = sgr;
    this.dirty.add(i);
    this.col++;
  }

  private putWide(ch: string, sgrL: number, sgrR: number): void {
    this.wrapIfNeeded(2);
    const i = this.row * COLS + this.col;
    this.orphanPartnerOf(i);
    this.orphanPartnerOf(i + 1);
    this.glyphs[i] = ch;
    this.halves[i] = 1;
    this.bytes[i] = -1;
    this.sgrs[i] = sgrL;
    this.glyphs[i + 1] = ch;
    this.halves[i + 1] = 2;
    this.bytes[i + 1] = -1;
    this.sgrs[i + 1] = sgrR;
    this.dirty.add(i);
    this.dirty.add(i + 1);
    this.col += 2;
  }

  // 覆寫 i 前呼叫：若 i 正處於真配對，partner 在 xterm 會被原子清空 → 記 dirty。
  // shadow 端 partner 不動（保留字形與左右半記號）＝孤兒半字語意
  private orphanPartnerOf(i: number): void {
    if (this.pairedAt(i)) {
      this.dirty.add(this.halves[i] === 1 ? i + 1 : i - 1);
    }
  }

  private pairedAt(i: number): boolean {
    if (this.halves[i] === 1) {
      return (
        i % COLS < COLS - 1 && this.halves[i + 1] === 2 && this.glyphs[i + 1] === this.glyphs[i]
      );
    }
    if (this.halves[i] === 2) {
      return i % COLS > 0 && this.halves[i - 1] === 1 && this.glyphs[i - 1] === this.glyphs[i];
    }
    return false;
  }

  private dispatchCsi(final: string): void {
    const p = this.csiParams.split(";").map((s) => parseInt(s, 10));
    const p0 = Number.isNaN(p[0]) ? 0 : p[0];
    switch (final) {
      case "H":
      case "f": {
        const r = p0 || 1;
        const p1 = p.length > 1 && !Number.isNaN(p[1]) ? p[1] : 0;
        this.row = Math.max(0, Math.min(ROWS - 1, r - 1));
        this.col = Math.max(0, Math.min(COLS - 1, (p1 || 1) - 1));
        this.leadIdx = -1;
        break;
      }
      case "A":
        this.row = Math.max(0, this.row - (p0 || 1));
        this.leadIdx = -1;
        break;
      case "B":
        this.row = Math.min(ROWS - 1, this.row + (p0 || 1));
        this.leadIdx = -1;
        break;
      case "C":
        this.col = Math.min(COLS - 1, Math.min(this.col, COLS - 1) + (p0 || 1));
        this.leadIdx = -1;
        break;
      case "D":
        this.col = Math.max(0, Math.min(this.col, COLS) - (p0 || 1));
        this.leadIdx = -1;
        break;
      case "J":
        this.erase(
          p0 === 2 ? 0 : p0 === 1 ? 0 : this.row * COLS + Math.min(this.col, COLS - 1),
          p0 === 2
            ? ROWS * COLS
            : p0 === 1
              ? this.row * COLS + Math.min(this.col, COLS - 1) + 1
              : ROWS * COLS,
        );
        this.leadIdx = -1;
        break;
      case "K": {
        const base = this.row * COLS;
        const cur = Math.min(this.col, COLS - 1);
        this.erase(
          p0 === 2 || p0 === 1 ? base : base + cur,
          p0 === 1 ? base + cur + 1 : base + COLS,
        );
        this.leadIdx = -1;
        break;
      }
      case "m":
        this.applySgr();
        break;
      default:
        break; // 其餘 CSI 對網格無影響，吃掉
    }
  }

  private erase(from: number, to: number): void {
    // 範圍邊界劈到 pair：範圍外的 partner 成孤兒（xterm 端整格被清）
    if (from > 0) this.orphanPartnerOf(from);
    if (to < ROWS * COLS) this.orphanPartnerOf(to - 1);
    for (let i = from; i < to; i++) {
      this.glyphs[i] = " ";
      this.halves[i] = 0;
      this.bytes[i] = -1;
      this.sgrs[i] = this.sgr; // BCE：以當前屬性清（背景色沿用）
    }
  }

  private applySgr(): void {
    const parts = this.csiParams === "" ? [""] : this.csiParams.split(";");
    for (const part of parts) {
      const n = part === "" ? 0 : parseInt(part, 10);
      if (Number.isNaN(n)) continue;
      if (n === 0) this.sgr = SGR_DEFAULT;
      else if (n === 1) this.sgr |= 1;
      else if (n === 22) this.sgr &= ~1;
      else if (n === 5) this.sgr |= 2;
      else if (n === 25) this.sgr &= ~2;
      else if (n === 7) this.sgr |= 4;
      else if (n === 27) this.sgr &= ~4;
      else if (n >= 30 && n <= 37) this.sgr = (this.sgr & ~(0xf << 3)) | ((n - 30 + 1) << 3);
      else if (n === 39) this.sgr &= ~(0xf << 3);
      else if (n >= 40 && n <= 47) this.sgr = (this.sgr & ~(0xf << 7)) | ((n - 40 + 1) << 7);
      else if (n === 49) this.sgr &= ~(0xf << 7);
      // 其餘（38;5 等）PTT 不用，忽略
    }
  }

  // 回報後才清 dirty：遞延批（行尾虛位／序列中途）的 dirty 跨批累積，
  // 期間被後續寫入自癒的格子在此處的 pairedAt 檢查中自然濾掉
  private collectRepairs(): RepairCell[] {
    const out: RepairCell[] = [];
    const sorted = [...this.dirty].sort((a, b) => a - b);
    this.dirty.clear();
    for (const i of sorted) {
      const half = this.halves[i];
      if (half === 0 || this.pairedAt(i) || this.glyphs[i] === "") continue;
      const s = this.sgrs[i];
      out.push({
        row: (i / COLS) | 0,
        col: i % COLS,
        glyph: this.glyphs[i],
        side: half === 1 ? "left" : "right",
        fg: ((s >> 3) & 0xf) - 1,
        bg: ((s >> 7) & 0xf) - 1,
        bold: (s & 1) !== 0,
        blink: (s & 2) !== 0,
        inverse: (s & 4) !== 0,
      });
    }
    return out;
  }
}
