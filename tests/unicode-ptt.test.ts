import { describe, expect, it } from "vitest";
import { Terminal } from "@xterm/xterm";
import { PTT_UNICODE_VERSION, PttUnicodeProvider } from "../src/unicode-ptt";

describe("PttUnicodeProvider.wcwidth", () => {
  const provider = new PttUnicodeProvider();
  const w = (ch: string) => provider.wcwidth(ch.codePointAt(0) ?? 0);

  it("PTT 彩繪主力字元(EAW Ambiguous)算 2 格", () => {
    expect(w("▂")).toBe(2); // 方塊
    expect(w("█")).toBe(2);
    expect(w("▔")).toBe(2);
    expect(w("◤")).toBe(2); // 三角
    expect(w("─")).toBe(2); // 框線
    expect(w("※")).toBe(2);
    expect(w("×")).toBe(2);
  });

  it("CJK 與全形(EAW W/F)算 2 格", () => {
    expect(w("中")).toBe(2);
    expect(w("ㄅ")).toBe(2);
    expect(w("Ａ")).toBe(2);
  });

  it("ASCII 算 1 格,控制字元 0 格", () => {
    expect(w("A")).toBe(1);
    expect(w(" ")).toBe(1);
    expect(provider.wcwidth(0x1b)).toBe(0);
  });

  it("組合字元算 0 格", () => {
    expect(provider.wcwidth(0x0301)).toBe(0); // combining acute accent
  });
});

describe("PttUnicodeProvider.charProperties", () => {
  const provider = new PttUnicodeProvider();
  const extractWidth = (v: number) => (v >> 1) & 3;

  it("寬度編碼與 xterm 內部格式一致", () => {
    expect(extractWidth(provider.charProperties(0x2582, 0))).toBe(2); // ▂
    expect(extractWidth(provider.charProperties(0x41, 0))).toBe(1); // A
  });

  it("組合字元跟隨前一個字的寬度並標記 join", () => {
    const preceding = provider.charProperties(0x4e2d, 0); // 中
    const v = provider.charProperties(0x0301, preceding);
    expect(extractWidth(v)).toBe(2);
    expect(v & 1).toBe(1);
  });
});

describe("整合:xterm buffer 的實際格寬", () => {
  // 不需要 DOM:xterm 在 open() 之前 parser 與 buffer 即可完整運作(同 blink-overlay.test.ts)
  function makeTerm(): Terminal {
    const term = new Terminal({ cols: 80, rows: 24, scrollback: 0, allowProposedApi: true });
    term.unicode.register(new PttUnicodeProvider());
    term.unicode.activeVersion = PTT_UNICODE_VERSION;
    return term;
  }
  const write = (term: Terminal, data: string) =>
    new Promise<void>((resolve) => term.write(data, resolve));

  it("彩繪方塊字在 buffer 佔 2 格(修跑版的核心)", async () => {
    const term = makeTerm();
    await write(term, "▂▃X");
    const line = term.buffer.active.getLine(0);
    expect(line?.getCell(0)?.getWidth()).toBe(2);
    expect(line?.getCell(2)?.getWidth()).toBe(2);
    expect(line?.getCell(4)?.getChars()).toBe("X"); // 兩個方塊後,X 應落在第 4 欄
  });

  it("未掛 provider 時方塊字只佔 1 格(佐證跑版根因)", async () => {
    const term = new Terminal({ cols: 80, rows: 24, scrollback: 0 });
    await write(term, "▂▃X");
    expect(term.buffer.active.getLine(0)?.getCell(2)?.getChars()).toBe("X");
  });
});
