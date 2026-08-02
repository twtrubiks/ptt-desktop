// 雙色字拆解：PTT 彩繪把 SGR 插在全形字兩 byte 中間讓左右半字異色（偵測見 sgr-reorder.ts）。
// sgr-reorder 在 byte 層把這種字包成 APC 區段（marker 全為 ASCII，big5/utf-8 解碼皆透明），
// 本模組在解碼後的字串層把區段改寫成「左半字 + SGR + 右半字」：
//   左半 = U+F0000 + cp（Plane 15 PUA），右半 = U+100000 + cp（Plane 16 PUA），加減法可逆。
// xterm buffer 因此以「兩個窄字元、各自帶色」保存雙色字——cell 模型天生支援兩格各一色，
// 再由 duotone-overlay 讀 buffer 把原字形的左右半各以該格顏色疊繪回去。
// 安全降級：若區段漏處理，APC 序列會被 xterm parser 當字串序列吞掉，畫面退回「前半色」，
// 與拆字前的行為相同。
// 已知限制：PTT 串流若真的送出 Plane 15/16 PUA 字元會被誤認為半字；Big5-UAO 造字區
// 映射在 BMP PUA（U+E000–F8FF），不受影響。

import type { RepairCell } from "./screen-grid";

export const DUO_START = "\x1b_ptt-duo\x1b\\";
export const DUO_END = "\x1b_ptt-end\x1b\\";
export const DUO_START_BYTES: readonly number[] = [...DUO_START].map((c) => c.charCodeAt(0));
export const DUO_END_BYTES: readonly number[] = [...DUO_END].map((c) => c.charCodeAt(0));

export const LEFT_BASE = 0xf0000;
export const RIGHT_BASE = 0x100000;

// 區段內容上限：1 個字（≤2 UTF-16 單元）+ 被扣住的 SGR（sgr-reorder 的 MAX_HELD_ESC_BYTES=64），
// 加上餘裕。超過視為串流損壞，放棄改寫原樣送出
const MAX_SECTION_CHARS = 128;

// 把「APC 區段包住的（字 + SGR 序列）」改寫成「左半 PUA + SGR + 右半 PUA」。
// 區段可能被切在多個 chunk 之間，故有狀態；chunk 尾端疑似 marker 開頭的部分先扣住。
export class DuotoneSplitFilter {
  private buf = "";

  process(chunk: string): string {
    this.buf += chunk;
    let out = "";
    for (;;) {
      const start = this.buf.indexOf(DUO_START);
      if (start === -1) {
        const keep = this.partialMarkerSuffix();
        out += this.buf.slice(0, this.buf.length - keep);
        this.buf = keep ? this.buf.slice(this.buf.length - keep) : "";
        return out;
      }
      out += this.buf.slice(0, start);
      const contentStart = start + DUO_START.length;
      const end = this.buf.indexOf(DUO_END, contentStart);
      if (end === -1) {
        if (this.buf.length - contentStart > MAX_SECTION_CHARS) {
          // 區段損壞（等不到結尾）：丟棄 start marker、其餘原樣送出。
          // 若殘缺的 end marker 稍後抵達，會被 xterm 當 APC 吞掉，不上畫面
          out += this.buf.slice(contentStart);
          this.buf = "";
          return out;
        }
        this.buf = this.buf.slice(start); // 區段未收齊，整段扣住等下一個 chunk
        return out;
      }
      out += splitSection(this.buf.slice(contentStart, end));
      this.buf = this.buf.slice(end + DUO_END.length);
    }
  }

  // 斷線前把扣住的殘留吐回（原樣，不改寫）：孤兒 marker 會被 xterm 吞掉，內容字元照常顯示
  flush(): string {
    const rest = this.buf;
    this.buf = "";
    return rest;
  }

  reset(): void {
    this.buf = "";
  }

  // buf 尾端與 DUO_START 開頭重疊的長度（可能是被切斷的 marker，扣住待下個 chunk）。
  // DUO_END 與 DUO_START 共享開頭，此檢查同時涵蓋兩者
  private partialMarkerSuffix(): number {
    const max = Math.min(DUO_START.length - 1, this.buf.length);
    for (let k = max; k > 0; k--) {
      if (DUO_START.startsWith(this.buf.slice(this.buf.length - k))) return k;
    }
    return 0;
  }
}

// 區段內容 =「1 個字 + 插在它中間的 SGR 序列們」。改寫為左半 + SGR + 右半，
// 半字前後包 SGR 8/28（conceal）：xterm 對隱形字只畫背景不畫字形，PUA 半字
// 因此不會被畫成 .notdef（其 glyph 比 cell 寬，溢出鄰格的殘影無法被重繪清掉），
// fg/bg 屬性仍完整存在 buffer，由 duotone-overlay 讀取自繪。中間的 SGR 可能含
// 全重置（ESC[m）會清掉 conceal，故右半前再補一次 SGR 8。
// 已知限制：若原串流本身開著 SGR 8（PTT 隱形字，罕見），結尾的 SGR 28 會提前解除。
// 字超出 BMP（Plane 15/16 塞不下）或解碼失敗（U+FFFD）時原樣送出（退回前半色）
function splitSection(content: string): string {
  const cp = content.codePointAt(0);
  if (cp === undefined || cp > 0xffff || cp === 0xfffd) return content;
  const char = String.fromCodePoint(cp);
  const sgrs = content.slice(char.length);
  return (
    `\x1b[8m${String.fromCodePoint(LEFT_BASE + cp)}${sgrs}` +
    `\x1b[8m${String.fromCodePoint(RIGHT_BASE + cp)}\x1b[28m`
  );
}

// 孤兒半字補寫序列（P6 修復式接線）：screen-grid 回報的批末孤兒以
// 「絕對 CUP + 該半自己的 SGR + conceal + PUA 半字」寫回 xterm，字形由
// duotone-overlay 繪回（與雙色字同一機制，restoreDuotoneText 的複製還原亦通用）。
// 整段以 ESC 7 / ESC 8 包裹：xterm 的 saveCursor/restoreCursor 保存游標位置與
// SGR 屬性，補寫對串流狀態透明（screen-grid 保證批末不在行尾虛位／序列中途
// 才回報，避開 restoreCursor 夾掉 pending-wrap 與劈壞跨批序列兩個坑）。
// 字形超出 BMP 或為 U+FFFD（PUA 編不進／無字形可繪）退為帶色空格——
// xterm 端該格已被原子清空，至少把背景色補對
export function buildOrphanRepair(repairs: readonly RepairCell[]): string {
  if (repairs.length === 0) return "";
  let out = "\x1b7";
  for (const r of repairs) {
    let sgr = "0";
    if (r.bold) sgr += ";1";
    if (r.blink) sgr += ";5";
    if (r.inverse) sgr += ";7";
    if (r.fg >= 0) sgr += `;3${r.fg}`;
    if (r.bg >= 0) sgr += `;4${r.bg}`;
    const cp = r.glyph.codePointAt(0) ?? 0xfffd;
    const half =
      cp > 0xffff || cp === 0xfffd
        ? " "
        : `\x1b[8m${String.fromCodePoint((r.side === "left" ? LEFT_BASE : RIGHT_BASE) + cp)}`;
    // conceal 不需成對解除：每格開頭的 SGR 0 重設、結尾 ESC 8 還原批末屬性
    out += `\x1b[${r.row + 1};${r.col + 1}H\x1b[${sgr}m${half}`;
  }
  return out + "\x1b8";
}

// 複製／轉存時把 PUA 半字還原成原字：相鄰的「左半＋同字右半」合併為一個字，
// 落單的半字（畫面被部分覆寫）也還原為原字
export function restoreDuotoneText(text: string): string {
  let out = "";
  let pendingLeft = -1; // 前一個字元是此 cp 的左半（原字已輸出，等右半來略過）
  for (const ch of text) {
    const cp = ch.codePointAt(0) as number;
    if (cp >= RIGHT_BASE) {
      const base = cp - RIGHT_BASE;
      if (pendingLeft !== base) out += String.fromCodePoint(base);
      pendingLeft = -1;
      continue;
    }
    pendingLeft = -1;
    if (cp >= LEFT_BASE) {
      const base = cp - LEFT_BASE;
      out += String.fromCodePoint(base);
      pendingLeft = base;
      continue;
    }
    out += ch;
  }
  return out;
}
