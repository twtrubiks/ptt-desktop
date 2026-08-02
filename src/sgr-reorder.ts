// 雙色字重排濾層:PTT 彩繪慣用「把 SGR 色碼(ESC[...m)插在一個全形字的兩個 byte
// 中間」讓左右半字異色。TextDecoder 遇到被劈開的字會吐 U+FFFD、孤兒 byte 變雜訊字母,
// 甚至級聯錯位(2026-07-13 實錄登入畫面 4347 bytes 中有 111 處)。
// 本濾層在 decode 之前把插在字中間的 SGR 序列搬到整個字後面,讓 decoder 正確配對,
// 並把整段包進 APC marker(ASCII,對 decoder 透明),交由解碼後的 DuotoneSplitFilter
// 改寫成「左半字 + SGR + 右半字」保留雙色(見 duotone.ts)。
// 只搬 SGR:其他 ESC 序列(游標移動等)插在字中間不屬此技法,維持原樣。
// SGR 可能與字元 byte 一起被切在兩個 frame 之間,因此濾層必須有狀態(同 TelnetFilter)。
import { DUO_END_BYTES, DUO_START_BYTES } from "./duotone";

const ESC = 0x1b;
const CSI_OPEN = 0x5b; // "["
const SGR_FINAL = 0x6d; // "m"

// 防呆上限:單字被連續 SGR 夾住的 byte 數不可能超過此值,超過即放棄重排原樣送出
const MAX_HELD_ESC_BYTES = 64;

type State =
  | "data" // 一般資料
  | "trail" // 已收字元前導 byte,等後續 byte(Big5 trail / UTF-8 continuation)
  | "esc" // trail 狀態下遇到 ESC,等 "["
  | "csi" // CSI 參數中,等 final byte
  | "afterSgr"; // SGR 序列收完,等字元後續 byte 或下一個 ESC

export class SgrReorderFilter {
  private state: State = "data";
  private utf8 = false;
  private charBytes: number[] = []; // 已收的字元 bytes(前導 + 已到的後續)
  private needed = 0; // 還缺幾個後續 byte
  private held: number[] = []; // 被扣住待搬移的 ESC 序列 bytes
  private escSeq: number[] = []; // 正在收的單一 ESC 序列

  process(data: Uint8Array): Uint8Array {
    const out: number[] = [];
    for (let i = 0; i < data.length; i++) {
      const b = data[i];
      switch (this.state) {
        case "data":
          if (this.isLead(b)) {
            this.charBytes = [b];
            this.needed = this.continuationCount(b);
            this.state = "trail";
          } else {
            out.push(b);
          }
          break;

        case "trail":
          if (b === ESC) {
            this.escSeq = [b];
            this.state = "esc";
          } else {
            // 不驗證合法性,原樣配對送出;非法組合交給 decoder 處理(與無濾層時相同)
            this.charBytes.push(b);
            if (--this.needed === 0) this.flushChar(out);
          }
          break;

        case "esc":
          if (b === CSI_OPEN) {
            this.escSeq.push(b);
            this.state = "csi";
          } else {
            this.bail(out, b); // 非 CSI(如 ESC M):不重排
          }
          break;

        case "csi":
          if (b >= 0x30 && b <= 0x3f) {
            // 參數 byte
            this.escSeq.push(b);
            if (this.heldBytes() > MAX_HELD_ESC_BYTES) this.bail(out, null);
          } else if (b === SGR_FINAL) {
            this.escSeq.push(b);
            this.held.push(...this.escSeq);
            this.escSeq = [];
            this.state = "afterSgr";
          } else {
            this.bail(out, b); // 非 SGR 的 final byte(游標移動等):不重排
          }
          break;

        case "afterSgr":
          if (b === ESC) {
            this.escSeq = [b];
            this.state = "esc";
          } else {
            this.charBytes.push(b);
            if (--this.needed === 0) this.flushChar(out);
            else this.state = "trail"; // UTF-8 還缺 continuation,繼續收
          }
          break;
      }
    }
    return Uint8Array.from(out);
  }

  // 登入後 PTT 切為 UTF-8 輸出,改以 UTF-8 序列規則配對。
  // 回傳殘留的未完成 bytes(理論上切換點是乾淨的提示畫面,應為空),
  // 呼叫端應先以舊編碼解碼,避免位元組被靜默丟棄
  switchToUtf8(): Uint8Array {
    const residue = this.residue();
    this.utf8 = true;
    return residue;
  }

  reset(): void {
    this.utf8 = false;
    this.clear();
  }

  private isLead(b: number): boolean {
    if (this.utf8) return b >= 0xc2 && b <= 0xf4;
    return b >= 0x81 && b <= 0xfe;
  }

  private continuationCount(lead: number): number {
    if (!this.utf8) return 1; // Big5 trail 固定 1 byte
    if (lead >= 0xf0) return 3;
    if (lead >= 0xe0) return 2;
    return 1;
  }

  // 字元收齊:先送完整字元,再送被扣住的 SGR(= 重排)。
  // 有扣住的 SGR = 雙色字:包進 APC 區段供 DuotoneSplitFilter 於解碼後拆成左右半字
  private flushChar(out: number[]): void {
    if (this.held.length) {
      out.push(...DUO_START_BYTES, ...this.charBytes, ...this.held, ...DUO_END_BYTES);
    } else {
      out.push(...this.charBytes);
    }
    this.clear();
  }

  // 放棄重排:依原始順序送出所有扣住的 bytes;current 若非 null 一併送出。
  // 送出後回到 data 狀態(current 為 ESC 序列內容,必為 ASCII,不需重新分類)
  private bail(out: number[], current: number | null): void {
    out.push(...this.charBytes, ...this.held, ...this.escSeq);
    if (current !== null) out.push(current);
    this.clear();
  }

  private heldBytes(): number {
    return this.held.length + this.escSeq.length;
  }

  private residue(): Uint8Array {
    const bytes = Uint8Array.from([...this.charBytes, ...this.held, ...this.escSeq]);
    this.clear();
    return bytes;
  }

  private clear(): void {
    this.state = "data";
    this.charBytes = [];
    this.needed = 0;
    this.held = [];
    this.escSeq = [];
  }
}
