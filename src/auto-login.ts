// 自動登入狀態機：只在連線建立後運作，對解碼後的下行文字做 pattern matching；
// 唯「密碼不對」須在原始位元組層比對（見 WRONG_PASSWORD_BIG5 註解）。
// 任一步卡超過 10 秒或密碼不對即放棄、交還手動，絕不自動重試（避免鎖帳號）。

export interface AutoLoginConfig {
  username: string;
  password: string;
  kickOtherLogins: string; // 「您想刪除其他重複登入的連線嗎」的回答（預設 n）
}

export interface AutoLoginCallbacks {
  send(text: string): void;
  onSwitchUtf8(): void; // 送出密碼後通知 codec 切換 utf-8 解碼
  onFinished(reason: "done" | "aborted", detail?: string): void;
}

const STEP_TIMEOUT_MS = 10_000;

// 主功能表偵測字樣：PTT 主選單標題列的已知字樣（pattern 以實錄畫面為準），
// E2E 驗收時若不符再校正。
const MAIN_MENU_PATTERN = "【主功能表】";

// 「密碼不對」的 Big5 位元組。密碼錯時 PTT 從未切到 UTF-8、仍以 Big5 重送畫面，
// 但 codec 在送出密碼後已切 utf-8 解碼，解碼結果永遠不含字面「密碼不對」
// （2026-07-12 E2E 實測：只會落到逾時 + 滿畫面亂碼），故此 pattern 必須比對原始位元組。
const WRONG_PASSWORD_BIG5 = Uint8Array.of(0xb1, 0x4b, 0xbd, 0x58, 0xa4, 0xa3, 0xb9, 0xef);

// 中止原因字樣。main.ts 依它判斷是否把 codec 撥回 big5，勿與比對處各自寫死
export const WRONG_PASSWORD_DETAIL = "密碼不對";

function indexOfBytes(haystack: Uint8Array, needle: Uint8Array): number {
  outer: for (let i = 0; i <= haystack.length - needle.length; i++) {
    for (let j = 0; j < needle.length; j++) {
      if (haystack[i + j] !== needle[j]) continue outer;
    }
    return i;
  }
  return -1;
}

export class AutoLogin {
  private buffer = "";
  private byteTail = new Uint8Array(0); // 位元組比對的跨 chunk 殘尾
  private active = false;
  private sentUsername = false;
  private sentPassword = false;
  private timer: ReturnType<typeof setTimeout> | null = null;

  constructor(
    private config: AutoLoginConfig,
    private cb: AutoLoginCallbacks,
  ) {}

  get running(): boolean {
    return this.active;
  }

  start(): void {
    this.active = true;
    this.armTimer();
  }

  stop(): void {
    this.active = false;
    this.buffer = "";
    this.byteTail = new Uint8Array(0);
    if (this.timer !== null) {
      clearTimeout(this.timer);
      this.timer = null;
    }
  }

  feed(text: string): void {
    if (!this.active) return;
    this.buffer = (this.buffer + text).slice(-4096);
    this.match();
  }

  // 解碼前的原始位元組（telnet 濾層後），只為偵測 Big5「密碼不對」
  feedBytes(data: Uint8Array): void {
    if (!this.active) return;
    const buf = new Uint8Array(this.byteTail.length + data.length);
    buf.set(this.byteTail);
    buf.set(data, this.byteTail.length);
    if (indexOfBytes(buf, WRONG_PASSWORD_BIG5) !== -1) {
      this.finish("aborted", WRONG_PASSWORD_DETAIL);
      return;
    }
    this.byteTail = buf.slice(Math.max(0, buf.length - (WRONG_PASSWORD_BIG5.length - 1)));
  }

  private match(): void {
    if (this.buffer.includes(WRONG_PASSWORD_DETAIL)) {
      this.finish("aborted", WRONG_PASSWORD_DETAIL);
      return;
    }
    if (this.buffer.includes(MAIN_MENU_PATTERN)) {
      this.finish("done");
      return;
    }
    if (this.buffer.includes("請輸入代號")) {
      if (this.sentUsername) {
        this.finish("aborted", "代號提示重複出現");
        return;
      }
      this.sentUsername = true;
      this.act(`${this.config.username},\r`); // 逗號啟用 UTF-8 模式
      return;
    }
    if (this.buffer.includes("請輸入您的密碼")) {
      if (this.sentPassword) {
        this.finish("aborted", "密碼提示重複出現");
        return;
      }
      this.sentPassword = true;
      this.act(`${this.config.password}\r`);
      this.cb.onSwitchUtf8();
      return;
    }
    if (this.buffer.includes("您想刪除其他重複登入的連線嗎")) {
      this.act(`${this.config.kickOtherLogins}\r`);
      return;
    }
    if (this.buffer.includes("按任意鍵繼續")) {
      this.act(" ");
      return;
    }
  }

  // 清空已比對內容避免同一提示重複觸發，送出後重新起算逾時
  private act(data: string): void {
    this.buffer = "";
    this.cb.send(data);
    this.armTimer();
  }

  private armTimer(): void {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = setTimeout(() => this.finish("aborted", "逾時"), STEP_TIMEOUT_MS);
  }

  private finish(reason: "done" | "aborted", detail?: string): void {
    if (!this.active) return;
    this.stop();
    this.cb.onFinished(reason, detail);
  }
}
