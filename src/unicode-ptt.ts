// PTT 專用的 xterm.js 字元寬度 provider。
// xterm 內建 wcwidth 把 East Asian Ambiguous(方塊、框線、◤◥ 等彩繪主力字元)算 1 格,
// 但 PTT 版面以 Big5 語意(2 格)編排,導致彩繪被壓縮跑版;
// 本 provider 把「Big5 index 映射像 ∪ EAW W/F/A」一律回報 2 格(表見 wcwidth-data.ts)。
import type { IUnicodeVersionProvider } from "@xterm/xterm";
import { WIDE_RANGES, ZERO_RANGES } from "./wcwidth-data";

export const PTT_UNICODE_VERSION = "ptt-big5";

// table 為 [lo, hi] 交錯的扁平陣列,已排序,二分搜尋
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

export class PttUnicodeProvider implements IUnicodeVersionProvider {
  readonly version = PTT_UNICODE_VERSION;

  wcwidth(codepoint: number): 0 | 1 | 2 {
    if (codepoint < 32) return 0;
    if (codepoint < 127) return 1;
    // 雙色字拆出的半字（Plane 15/16 PUA，見 duotone.ts）一律 1 格。
    // 必須先於 WIDE_RANGES：EAW 把 Plane 15/16 列為 Ambiguous，資料表算 2 格
    if (codepoint >= 0xf0000) return 1;
    if (inRanges(codepoint, ZERO_RANGES)) return 0;
    if (inRanges(codepoint, WIDE_RANGES)) return 2;
    return 1;
  }

  // 與內建 UnicodeV6 相同的行為與 bit 編碼(自 @xterm/xterm lib 驗證):
  // value = (state & 0xffffff) << 3 | (width & 3) << 1 | (shouldJoin ? 1 : 0)
  charProperties(codepoint: number, preceding: number): number {
    let width: number = this.wcwidth(codepoint);
    let shouldJoin = width === 0 && preceding !== 0;
    if (shouldJoin) {
      const oldWidth = (preceding >> 1) & 3;
      if (oldWidth === 0) shouldJoin = false;
      else if (oldWidth > width) width = oldWidth;
    }
    return ((width & 3) << 1) | (shouldJoin ? 1 : 0);
  }
}
