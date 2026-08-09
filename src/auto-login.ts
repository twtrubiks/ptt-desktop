// 自動登入狀態機：只在連線建立後運作，對解碼後的下行文字做 pattern matching；
// 唯編碼切換點的偵測須在原始位元組層比對（見 WRONG_PASSWORD_BIG5 與
// PASSWORD_PROMPT_UTF8 註解）。
// 任一步卡超過 10 秒或密碼不對即放棄、交還手動，絕不自動重試（避免鎖帳號）。

export interface AutoLoginConfig {
  username: string;
  password: string;
  kickOtherLogins: string; // 「您想刪除其他重複登入的連線嗎」的回答（預設 n）
}

export interface AutoLoginCallbacks {
  send(text: string): void;
  // 通知 codec 切換 utf-8 解碼。時機二擇一（見 PASSWORD_PROMPT_UTF8 註解）：
  // 新版 PTT 於位元組層看到 UTF-8 密碼提示時（解碼前）、舊版 PTT 於送出密碼後
  onSwitchUtf8(): void;
  // 位元組層命中 Big5「密碼不對」＝伺服器仍在 Big5，通知 main.ts 撥回 big5 解碼
  onResetBig5(): void;
  onFinished(reason: "done" | "aborted", detail?: string): void;
}

const STEP_TIMEOUT_MS = 10_000;

// 主功能表偵測字樣：PTT 主選單標題列的已知字樣（pattern 以實錄畫面為準），
// E2E 驗收時若不符再校正。
const MAIN_MENU_PATTERN = "【主功能表】";

// 「密碼不對」的 Big5 位元組。舊版 PTT 密碼錯時從未切到 UTF-8、仍以 Big5 重送畫面，
// 但 codec 屆時已切 utf-8 解碼，解碼結果永遠不含字面「密碼不對」
// （2026-07-12 E2E 實測：只會落到逾時 + 滿畫面亂碼），故此 pattern 必須比對原始位元組。
// 命中同時代表伺服器仍在 Big5，據此觸發 onResetBig5（新版 UTF-8 的「密碼不對」
// 解碼後由文字比對命中，不觸發撥回）。
const WRONG_PASSWORD_BIG5 = Uint8Array.of(0xb1, 0x4b, 0xbd, 0x58, 0xa4, 0xa3, 0xb9, 0xef);

// 「請輸入您的密碼」的 UTF-8 位元組。2026-08 PTT 更新後，伺服器收到「id,」即切
// UTF-8 輸出，密碼提示已是 UTF-8（2026-08-09 實錄）；此時 codec 仍在 big5，
// 解碼結果是亂碼，文字比對永遠接不到提示。必須在位元組層偵測、趕在本 chunk
// 解碼前切換 codec——提示才不上亂碼，後續文字比對也才能照常送密碼。
const PASSWORD_PROMPT_UTF8 = new TextEncoder().encode("請輸入您的密碼");

// 位元組比對跨 chunk 殘尾長度：涵蓋最長 needle 差一個 byte 的切割
const BYTE_TAIL_LEN = Math.max(WRONG_PASSWORD_BIG5.length, PASSWORD_PROMPT_UTF8.length) - 1;

// 中止原因字樣。與文字比對處共用，勿各自寫死
const WRONG_PASSWORD_DETAIL = "密碼不對";

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
  private utf8Switched = false; // 保證 onSwitchUtf8 只觸發一次（位元組層或送密碼後二擇一）
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

  // 解碼前的原始位元組（telnet 濾層後）：偵測 Big5「密碼不對」與 UTF-8 密碼提示。
  // main.ts 保證本方法先於該 chunk 的解碼被呼叫，兩個 callback 才來得及在
  // 亂碼上畫面前把 codec 撥到正確編碼
  feedBytes(data: Uint8Array): void {
    if (!this.active) return;
    const buf = new Uint8Array(this.byteTail.length + data.length);
    buf.set(this.byteTail);
    buf.set(data, this.byteTail.length);
    if (indexOfBytes(buf, WRONG_PASSWORD_BIG5) !== -1) {
      this.cb.onResetBig5();
      this.finish("aborted", WRONG_PASSWORD_DETAIL);
      return;
    }
    // 新版 PTT：密碼提示已是 UTF-8 → 解碼前先切，後續文字比對照常送密碼。
    // 只切編碼不送密碼：送密碼仍統一走文字比對，重複提示的防重送保護才不會被繞過
    if (!this.utf8Switched && indexOfBytes(buf, PASSWORD_PROMPT_UTF8) !== -1) {
      this.utf8Switched = true;
      this.cb.onSwitchUtf8();
    }
    this.byteTail = buf.slice(Math.max(0, buf.length - BYTE_TAIL_LEN));
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
      // 舊版 PTT：提示是 Big5、送出密碼後其後輸出才是 UTF-8 → 此時才切；
      // 新版已在位元組層提前切換，不得重切（重切會 flush 掉跨 chunk 的半個字）
      if (!this.utf8Switched) {
        this.utf8Switched = true;
        this.cb.onSwitchUtf8();
      }
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
