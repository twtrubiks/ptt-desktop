import { describe, expect, it } from "vitest";
import { Terminal } from "@xterm/xterm";
import {
  ANSI_16,
  DEFAULT_BACKGROUND,
  DEFAULT_FOREGROUND,
  paletteCss,
  scanBlinkCells,
} from "../src/blink-overlay";

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
    expect(scanBlinkCells(term)).toEqual([{ col: 0, row: 0, width: 1, css: "#000080" }]);
  });

  it("反白格的蓋色用前景色，且不做粗體亮化", async () => {
    const term = makeTerm();
    await write(term, "\x1b[1;33;7;5mA\x1b[m \x1b[7;5mB");
    expect(scanBlinkCells(term)).toEqual([
      { col: 0, row: 0, width: 1, css: "#808000" }, // 黃字反白：蓋原前景色，不因粗體變亮黃
      { col: 2, row: 0, width: 1, css: "#c0c0c0" }, // 無色反白：蓋預設前景色
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

// 色盤是 terminal.ts theme 與兩個 overlay 的單一出處，值錯會讓彩繪整體偏色
// 且 overlay 疊繪與 xterm 底層畫面對不上，故以測試釘死
describe("VGA 色盤", () => {
  it("16 色與 PttChrome src/styles/color.scss 一致", () => {
    // prettier-ignore
    expect(ANSI_16).toEqual([
      "#000000", "#800000", "#008000", "#808000", "#000080", "#800080", "#008080", "#c0c0c0",
      "#808080", "#ff0000", "#00ff00", "#ffff00", "#0000ff", "#ff00ff", "#00ffff", "#ffffff",
    ]);
  });

  it("明確黑底（SGR 40）與預設背景同色——否則黑底彩繪浮出灰黑拼布（第七根因）", () => {
    expect(paletteCss(0)).toBe(DEFAULT_BACKGROUND);
  });

  it("預設前景＝color 7——BBS 慣例未標色的字即 SGR 37", () => {
    expect(paletteCss(7)).toBe(DEFAULT_FOREGROUND);
  });
});
