import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AutoLogin } from "../src/auto-login";

interface Harness {
  login: AutoLogin;
  sent: string[];
  switched: () => number;
  resetBig5: () => number;
  finished: () => { reason: string; detail?: string } | null;
}

function setup(): Harness {
  const sent: string[] = [];
  let switchCount = 0;
  let resetCount = 0;
  let result: { reason: string; detail?: string } | null = null;
  const login = new AutoLogin(
    { username: "myid", password: "secret", kickOtherLogins: "n" },
    {
      send: (text) => sent.push(text),
      onSwitchUtf8: () => {
        switchCount++;
      },
      onResetBig5: () => {
        resetCount++;
      },
      onFinished: (reason, detail) => {
        result = { reason, detail };
      },
    },
  );
  login.start();
  return {
    login,
    sent,
    switched: () => switchCount,
    resetBig5: () => resetCount,
    finished: () => result,
  };
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("AutoLogin：完整登入流程（以實錄畫面近似文字餵入）", () => {
  it("代號 → 密碼（切 utf-8）→ 重複登入 → 連續按任意鍵 → 主功能表", () => {
    const h = setup();

    h.login.feed("請輸入代號，或以 guest 參觀，或以 new 註冊: ");
    expect(h.sent).toEqual(["myid,\r"]);

    h.login.feed("\r\n請輸入您的密碼: ");
    expect(h.sent).toEqual(["myid,\r", "secret\r"]);
    expect(h.switched()).toBe(1);

    h.login.feed("您想刪除其他重複登入的連線嗎？[Y/n] ");
    expect(h.sent).toEqual(["myid,\r", "secret\r", "n\r"]);

    h.login.feed("歡迎您再度拜訪，上次您是從 x.x.x.x 連往本站。\r\n請按任意鍵繼續... ");
    h.login.feed("本日簽到 ...\r\n請按任意鍵繼續... "); // 進站後可連續多頁
    expect(h.sent).toEqual(["myid,\r", "secret\r", "n\r", " ", " "]);

    h.login.feed("【主功能表】\r\n(A)nnounce 精華公佈欄");
    expect(h.finished()).toEqual({ reason: "done", detail: undefined });
    expect(h.login.running).toBe(false);
  });

  it("pattern 跨 chunk 切割仍能比對", () => {
    const h = setup();
    h.login.feed("請輸入");
    expect(h.sent).toEqual([]);
    h.login.feed("代號，或以 guest 參觀: ");
    expect(h.sent).toEqual(["myid,\r"]);
  });
});

// 「請輸入您的密碼」的 UTF-8 位元組（同 src/auto-login.ts 的 PASSWORD_PROMPT_UTF8）
const PASSWORD_PROMPT_UTF8 = new TextEncoder().encode("請輸入您的密碼");

describe("AutoLogin：新版 PTT（2026-08 起送出「id,」後伺服器立即切 UTF-8）", () => {
  it("UTF-8 密碼提示位元組 → 解碼前先切 utf-8，送密碼仍由文字比對觸發、總共只切一次", () => {
    const h = setup();
    h.login.feed("請輸入代號，或以 guest 參觀，或以 new 註冊: ");
    expect(h.sent).toEqual(["myid,\r"]);

    // 模擬 main.ts 時序：位元組先餵（echo + ANSI + UTF-8 提示，取自 2026-08-09 實錄）
    const echo = new TextEncoder().encode("myid,\x1b[m\x1b[22;1H\x1b[K\x1b[m");
    h.login.feedBytes(Uint8Array.of(...echo, ...PASSWORD_PROMPT_UTF8, 0x3a, 0x20));
    expect(h.switched()).toBe(1); // 本 chunk 解碼前就得切，亂碼才不會上畫面
    expect(h.sent).toEqual(["myid,\r"]); // 位元組層只切編碼，不送密碼

    // 切換後同一 chunk 以 utf-8 正確解碼，文字比對送密碼；不得重複切
    h.login.feed("myid,請輸入您的密碼: ");
    expect(h.sent).toEqual(["myid,\r", "secret\r"]);
    expect(h.switched()).toBe(1);
  });

  it("UTF-8 密碼提示位元組跨 chunk 切割仍能偵測", () => {
    const h = setup();
    h.login.feed("請輸入代號：");
    h.login.feedBytes(PASSWORD_PROMPT_UTF8.slice(0, 5));
    expect(h.switched()).toBe(0);
    h.login.feedBytes(PASSWORD_PROMPT_UTF8.slice(5));
    expect(h.switched()).toBe(1);
  });

  it("新版密碼錯（UTF-8 文字「密碼不對」）→ 中止、不撥回 big5、不再送任何東西", () => {
    const h = setup();
    h.login.feed("請輸入代號：");
    h.login.feedBytes(PASSWORD_PROMPT_UTF8);
    h.login.feed("請輸入您的密碼: ");
    expect(h.sent).toEqual(["myid,\r", "secret\r"]);

    h.login.feed("\r\n密碼不對或無此帳號。");
    expect(h.finished()).toEqual({ reason: "aborted", detail: "密碼不對" });
    expect(h.resetBig5()).toBe(0); // 伺服器已在 UTF-8 模式，撥回 big5 反而會亂碼
    expect(h.sent).toEqual(["myid,\r", "secret\r"]);
  });

  it("切換後密碼提示文字再次出現 → 中止（防重送保護不因位元組層切換而失效）", () => {
    const h = setup();
    h.login.feed("請輸入代號：");
    h.login.feedBytes(PASSWORD_PROMPT_UTF8);
    h.login.feed("請輸入您的密碼: ");
    h.login.feed("請輸入您的密碼: ");
    expect(h.finished()).toEqual({ reason: "aborted", detail: "密碼提示重複出現" });
    expect(h.sent).toEqual(["myid,\r", "secret\r"]);
  });
});

// 「密碼不對」的 Big5 位元組（同 src/auto-login.ts 的 WRONG_PASSWORD_BIG5）
const WRONG_PASSWORD_BIG5 = Uint8Array.of(0xb1, 0x4b, 0xbd, 0x58, 0xa4, 0xa3, 0xb9, 0xef);

describe("AutoLogin：密碼錯誤走位元組層偵測（PTT 回應仍是 Big5）", () => {
  it("Big5「密碼不對」位元組（夾雜 ANSI 與其他內容）→ 中止", () => {
    const h = setup();
    h.login.feed("請輸入代號：");
    h.login.feed("請輸入您的密碼：");
    // 模擬 PTT 回應：ESC[1;37m + Big5「密碼不對」+ 其他位元組
    const chunk = Uint8Array.of(
      0x1b,
      0x5b,
      0x31,
      0x3b,
      0x33,
      0x37,
      0x6d,
      ...WRONG_PASSWORD_BIG5,
      0xa9,
      0xce,
    );
    h.login.feedBytes(chunk);
    expect(h.finished()).toEqual({ reason: "aborted", detail: "密碼不對" });
    expect(h.resetBig5()).toBe(1); // 伺服器仍在 Big5，須撥回讓錯誤畫面可讀
    expect(h.sent).toEqual(["myid,\r", "secret\r"]); // 不再送任何東西
  });

  it("Big5 pattern 跨 chunk 切割仍能比對", () => {
    const h = setup();
    h.login.feedBytes(WRONG_PASSWORD_BIG5.slice(0, 3));
    expect(h.finished()).toBeNull();
    h.login.feedBytes(WRONG_PASSWORD_BIG5.slice(3));
    expect(h.finished()).toEqual({ reason: "aborted", detail: "密碼不對" });
    expect(h.resetBig5()).toBe(1);
  });

  it("stop 後 feedBytes 不動作", () => {
    const h = setup();
    h.login.stop();
    h.login.feedBytes(WRONG_PASSWORD_BIG5);
    expect(h.finished()).toBeNull();
  });
});

describe("AutoLogin：放棄自動登入、絕不自動重試", () => {
  it("密碼不對 → 中止且不再送任何東西", () => {
    const h = setup();
    h.login.feed("請輸入代號：");
    h.login.feed("請輸入您的密碼：");
    h.login.feed("\r\n密碼不對喔！請仔細檢查後重新輸入。");
    expect(h.finished()).toEqual({ reason: "aborted", detail: "密碼不對" });

    h.login.feed("請輸入您的密碼："); // 中止後餵入任何提示都不動作
    expect(h.sent).toEqual(["myid,\r", "secret\r"]);
  });

  it("密碼提示重複出現 → 中止（避免鎖帳號）", () => {
    const h = setup();
    h.login.feed("請輸入代號：");
    h.login.feed("請輸入您的密碼：");
    h.login.feed("請輸入您的密碼：");
    expect(h.finished()).toEqual({ reason: "aborted", detail: "密碼提示重複出現" });
    expect(h.sent).toEqual(["myid,\r", "secret\r"]);
  });

  it("代號提示重複出現 → 中止", () => {
    const h = setup();
    h.login.feed("請輸入代號：");
    h.login.feed("請輸入代號：");
    expect(h.finished()).toEqual({ reason: "aborted", detail: "代號提示重複出現" });
    expect(h.sent).toEqual(["myid,\r"]);
  });

  it("任一步卡超過 10 秒 → 中止交還手動", () => {
    const h = setup();
    h.login.feed("請輸入代號：");
    vi.advanceTimersByTime(10_000);
    expect(h.finished()).toEqual({ reason: "aborted", detail: "逾時" });
  });

  it("每步動作會重新起算逾時", () => {
    const h = setup();
    vi.advanceTimersByTime(9_000);
    h.login.feed("請輸入代號：");
    vi.advanceTimersByTime(9_000); // 距上一步動作未滿 10 秒
    expect(h.finished()).toBeNull();
    vi.advanceTimersByTime(1_000);
    expect(h.finished()).toEqual({ reason: "aborted", detail: "逾時" });
  });

  it("stop 後不再觸發逾時 callback", () => {
    const h = setup();
    h.login.stop();
    vi.advanceTimersByTime(20_000);
    expect(h.finished()).toBeNull();
  });
});
