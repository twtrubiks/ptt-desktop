import { describe, expect, it } from "vitest";
import { Terminal } from "@xterm/xterm";
import { scanLinkSpans } from "../src/link-underline";

// 不需要 DOM：xterm 在 open() 之前 parser 與 buffer 即可完整運作，
// 讓測試走真實的 ANSI 解析路徑，而不是自造假 cell。
function makeTerm(): Terminal {
  return new Terminal({ cols: 80, rows: 24, scrollback: 0 });
}

function write(term: Terminal, data: string): Promise<void> {
  return new Promise((resolve) => term.write(data, resolve));
}

describe("scanLinkSpans", () => {
  it("行首網址，格位與寬度正確", async () => {
    const term = makeTerm();
    await write(term, "https://example.com");
    expect(scanLinkSpans(term)).toEqual([{ row: 0, col: 0, width: 19 }]);
  });

  it("全形字前綴會位移起始格：一個全形字佔兩格", async () => {
    const term = makeTerm();
    await write(term, "看板 https://example.com");
    expect(scanLinkSpans(term)).toEqual([{ row: 0, col: 5, width: 19 }]);
  });

  it("ANSI 色碼不影響匹配", async () => {
    const term = makeTerm();
    await write(term, "\x1b[1;33mhttps://example.com\x1b[m");
    expect(scanLinkSpans(term)).toEqual([{ row: 0, col: 0, width: 19 }]);
  });

  it("尾端標點不算網址的一部分", async () => {
    const term = makeTerm();
    await write(term, "https://example.com,");
    expect(scanLinkSpans(term)).toEqual([{ row: 0, col: 0, width: 19 }]);
  });

  it("游標定位後的網址：中間的空格子也要對位", async () => {
    const term = makeTerm();
    await write(term, "\x1b[3;10Hhttps://example.com");
    expect(scanLinkSpans(term)).toEqual([{ row: 2, col: 9, width: 19 }]);
  });

  it("同一行兩個網址", async () => {
    const term = makeTerm();
    await write(term, "https://a.bc http://d.ef");
    expect(scanLinkSpans(term)).toEqual([
      { row: 0, col: 0, width: 12 },
      { row: 0, col: 13, width: 11 },
    ]);
  });

  it("沒有網址時回傳空陣列", async () => {
    const term = makeTerm();
    await write(term, "推 這篇文章寫得真好");
    expect(scanLinkSpans(term)).toEqual([]);
  });
});
