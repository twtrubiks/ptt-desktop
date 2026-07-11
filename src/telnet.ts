// telnet-over-WebSocket 下行濾層：
// 1. 丟棄串流最前面的 `HTTP/1.1 200 OK\r\n\r\n` 前導（PTT 閘道行為）；
//    只在開頭確實是 "HTTP/" 時才丟棄，否則把已收位元組回放進一般處理，不吞資料
// 2. 濾掉 telnet IAC 協商序列（MVP 只濾不回應，spike 實測可行）
// IAC 序列可能被切在兩個 frame 之間，因此濾層必須有狀態。

const IAC = 0xff;
const SE = 0xf0;
const SB = 0xfa;
const WILL = 0xfb;
const WONT = 0xfc;
const DO = 0xfd;
const DONT = 0xfe;

const PREAMBLE_PREFIX = [0x48, 0x54, 0x54, 0x50, 0x2f]; // "HTTP/"
const PREAMBLE_END = [0x0d, 0x0a, 0x0d, 0x0a]; // \r\n\r\n

type State =
  | "prefix" // 比對串流開頭是否為 "HTTP/"，不符則回放已收位元組
  | "preamble" // 已確認 HTTP 前導，丟棄直到讀完 \r\n\r\n
  | "data" // 一般資料
  | "iac" // 已讀到 IAC，等指令 byte
  | "option" // WILL/WONT/DO/DONT 之後，等選項 byte
  | "sub" // SB 子協商內容
  | "subIac"; // 子協商內讀到 IAC，等 SE

export class TelnetFilter {
  private state: State = "prefix";
  private prefixHeld: number[] = [];
  private preambleMatched = 0;

  reset(): void {
    this.state = "prefix";
    this.prefixHeld = [];
    this.preambleMatched = 0;
  }

  filter(chunk: Uint8Array): Uint8Array {
    const out: number[] = [];
    for (const byte of chunk) {
      this.step(byte, out);
    }
    return Uint8Array.from(out);
  }

  private step(byte: number, out: number[]): void {
    switch (this.state) {
      case "prefix":
        if (byte === PREAMBLE_PREFIX[this.prefixHeld.length]) {
          this.prefixHeld.push(byte);
          if (this.prefixHeld.length === PREAMBLE_PREFIX.length) {
            this.prefixHeld = [];
            this.state = "preamble";
          }
          break;
        }
        // 開頭不是 HTTP 前導：已收位元組與當前 byte 回放進一般 telnet 處理
        this.state = "data";
        for (const held of this.prefixHeld) this.step(held, out);
        this.prefixHeld = [];
        this.step(byte, out);
        break;
      case "preamble":
        if (byte === PREAMBLE_END[this.preambleMatched]) {
          this.preambleMatched++;
          if (this.preambleMatched === PREAMBLE_END.length) {
            this.state = "data";
          }
        } else {
          this.preambleMatched = byte === 0x0d ? 1 : 0;
        }
        break;
      case "data":
        if (byte === IAC) {
          this.state = "iac";
        } else {
          out.push(byte);
        }
        break;
      case "iac":
        if (byte === IAC) {
          out.push(IAC); // IAC IAC = 資料中的 0xFF
          this.state = "data";
        } else if (byte === SB) {
          this.state = "sub";
        } else if (byte === WILL || byte === WONT || byte === DO || byte === DONT) {
          this.state = "option";
        } else {
          this.state = "data"; // NOP 等單 byte 指令，丟棄
        }
        break;
      case "option":
        this.state = "data"; // 吃掉選項 byte
        break;
      case "sub":
        if (byte === IAC) {
          this.state = "subIac";
        }
        break;
      case "subIac":
        this.state = byte === SE ? "data" : "sub";
        break;
    }
  }
}
