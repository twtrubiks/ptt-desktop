import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AutoLogin } from "../src/auto-login";

interface Harness {
  login: AutoLogin;
  sent: string[];
  switched: () => number;
  finished: () => { reason: string; detail?: string } | null;
}

function setup(): Harness {
  const sent: string[] = [];
  let switchCount = 0;
  let result: { reason: string; detail?: string } | null = null;
  const login = new AutoLogin(
    { username: "myid", password: "secret", kickOtherLogins: "n" },
    {
      send: (text) => sent.push(text),
      onSwitchUtf8: () => {
        switchCount++;
      },
      onFinished: (reason, detail) => {
        result = { reason, detail };
      },
    },
  );
  login.start();
  return { login, sent, switched: () => switchCount, finished: () => result };
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
    expect(h.sent).toEqual(["myid,\r", "secret\r"]); // 不再送任何東西
  });

  it("Big5 pattern 跨 chunk 切割仍能比對", () => {
    const h = setup();
    h.login.feedBytes(WRONG_PASSWORD_BIG5.slice(0, 3));
    expect(h.finished()).toBeNull();
    h.login.feedBytes(WRONG_PASSWORD_BIG5.slice(3));
    expect(h.finished()).toEqual({ reason: "aborted", detail: "密碼不對" });
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
