import { describe, expect, it } from "vitest";
import { Terminal } from "@xterm/xterm";
import { scanBlinkCells } from "../src/blink-overlay";

// 不需要 DOM：xterm 在 open() 之前 parser 與 buffer 即可完整運作，
// 讓測試走真實的 ESC[5m 解析路徑，而不是自造假 cell。
function makeTerm(): Terminal {
  return new Terminal({ cols: 80, rows: 24, scrollback: 0 });
}

function write(term: Terminal, data: string): Promise<void> {
  return new Promise((resolve) => term.write(data, resolve));
}

describe("scanBlinkCells", () => {
  it("找出 SGR 5 的格子，全形字佔兩格", async () => {
    const term = makeTerm();
    await write(term, "\x1b[1;33;5m閃\x1b[m穩");
    expect(scanBlinkCells(term)).toEqual([{ col: 0, row: 0, width: 2, css: "#000000" }]);
  });

  it("沒有閃爍字時回傳空陣列", async () => {
    const term = makeTerm();
    await write(term, "\x1b[1;33m一般文字\x1b[m");
    expect(scanBlinkCells(term)).toEqual([]);
  });

  it("SGR 25 取消閃爍", async () => {
    const term = makeTerm();
    await write(term, "\x1b[5mA\x1b[25mB");
    expect(scanBlinkCells(term)).toEqual([{ col: 0, row: 0, width: 1, css: "#000000" }]);
  });

  it("蓋色用該格的背景色", async () => {
    const term = makeTerm();
    await write(term, "\x1b[5;44mA"); // 藍底
    expect(scanBlinkCells(term)).toEqual([{ col: 0, row: 0, width: 1, css: "#3465a4" }]);
  });

  it("反白格的蓋色用前景色，且不做粗體亮化", async () => {
    const term = makeTerm();
    await write(term, "\x1b[1;33;7;5mA\x1b[m \x1b[7;5mB");
    expect(scanBlinkCells(term)).toEqual([
      { col: 0, row: 0, width: 1, css: "#c4a000" }, // 黃字反白：蓋原前景色，不因粗體變亮黃
      { col: 2, row: 0, width: 1, css: "#ffffff" }, // 無色反白：蓋預設前景色
    ]);
  });

  it("256 色與 RGB 背景", async () => {
    const term = makeTerm();
    await write(term, "\x1b[5;48;5;196mA\x1b[m\x1b[5;48;5;244mB\x1b[m\x1b[5;48;2;18;52;86mC");
    expect(scanBlinkCells(term)).toEqual([
      { col: 0, row: 0, width: 1, css: "#ff0000" }, // 色立方 196
      { col: 1, row: 0, width: 1, css: "#808080" }, // 灰階 244
      { col: 2, row: 0, width: 1, css: "#123456" }, // 24-bit RGB
    ]);
  });

  it("跨列與非零起始欄位", async () => {
    const term = makeTerm();
    await write(term, "\r\n\r\n  \x1b[5mXY");
    expect(scanBlinkCells(term)).toEqual([
      { col: 2, row: 2, width: 1, css: "#000000" },
      { col: 3, row: 2, width: 1, css: "#000000" },
    ]);
  });
});
