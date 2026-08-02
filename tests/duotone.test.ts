import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { Terminal } from "@xterm/xterm";
import { PttDecoder } from "../src/codec";
import {
  buildOrphanRepair,
  DUO_END,
  DUO_START,
  DuotoneSplitFilter,
  LEFT_BASE,
  RIGHT_BASE,
  restoreDuotoneText,
} from "../src/duotone";
import { scanDuotoneCells } from "../src/duotone-overlay";
import { ScreenGrid } from "../src/screen-grid";
import { SgrReorderFilter } from "../src/sgr-reorder";
import { PTT_UNICODE_VERSION, PttUnicodeProvider } from "../src/unicode-ptt";

// 與 blink-overlay.test.ts 同法：xterm 在 open() 之前 parser 與 buffer 即可完整運作
function makeTerm(): Terminal {
  const term = new Terminal({ cols: 80, rows: 24, scrollback: 0, allowProposedApi: true });
  term.unicode.register(new PttUnicodeProvider());
  term.unicode.activeVersion = PTT_UNICODE_VERSION;
  return term;
}

function write(term: Terminal, data: string): Promise<void> {
  return new Promise((resolve) => term.write(data, resolve));
}

// 「雨」U+96E8 的左右半字
const RAIN = 0x96e8;
const RAIN_L = String.fromCodePoint(LEFT_BASE + RAIN);
const RAIN_R = String.fromCodePoint(RIGHT_BASE + RAIN);

// 路線可行性的閘門測試：xterm 必須把 Plane 15/16 PUA（astral，2 個 UTF-16 單元）
// 當成普通窄字元存進 buffer——每格一字、各自帶色、寬度取自我們的 provider
describe("xterm 對 PUA 半字的 buffer 行為（閘門）", () => {
  it("左右半各佔 1 格、各自保有前景色", async () => {
    const term = makeTerm();
    await write(term, `\x1b[1;34m${RAIN_L}\x1b[36m${RAIN_R}\x1b[m!`);
    const line = term.buffer.active.getLine(0);
    const c0 = line?.getCell(0);
    const c1 = line?.getCell(1);
    const c2 = line?.getCell(2);
    expect(c0?.getChars()).toBe(RAIN_L);
    expect(c0?.getWidth()).toBe(1);
    expect(c0?.getFgColor()).toBe(4); // 藍
    expect(c0?.isBold()).toBeTruthy();
    expect(c1?.getChars()).toBe(RAIN_R);
    expect(c1?.getWidth()).toBe(1);
    expect(c1?.getFgColor()).toBe(6); // 青
    expect(c2?.getChars()).toBe("!");
  });

  it("半字的背景色也各自獨立（雙色技法可同時換底色）", async () => {
    const term = makeTerm();
    await write(term, `\x1b[44m${RAIN_L}\x1b[41m${RAIN_R}`);
    const line = term.buffer.active.getLine(0);
    expect(line?.getCell(0)?.getBgColor()).toBe(4);
    expect(line?.getCell(1)?.getBgColor()).toBe(1);
  });

  it("安全降級：孤兒 APC marker 被 xterm 靜默吞掉，不上畫面", async () => {
    const term = makeTerm();
    await write(term, `A${DUO_START}B${DUO_END}C`);
    const line = term.buffer.active.getLine(0);
    expect(line?.translateToString(true)).toBe("ABC");
  });
});

describe("DuotoneSplitFilter", () => {
  it("區段改寫為「左半 + SGR + 右半」，半字帶 conceal 讓 xterm 不畫字形", () => {
    const filter = new DuotoneSplitFilter();
    const out = filter.process(`${DUO_START}雨\x1b[36m${DUO_END}`);
    expect(out).toBe(`\x1b[8m${RAIN_L}\x1b[36m\x1b[8m${RAIN_R}\x1b[28m`);
  });

  it("區段外的文字原樣通過", () => {
    const filter = new DuotoneSplitFilter();
    expect(filter.process("ab\x1b[31m中\x1b[m")).toBe("ab\x1b[31m中\x1b[m");
  });

  it("多個 SGR 全數保留在兩半之間，全重置後 conceal 重新補上", () => {
    const filter = new DuotoneSplitFilter();
    const out = filter.process(`${DUO_START}雨\x1b[m\x1b[1;36m${DUO_END}`);
    expect(out).toBe(`\x1b[8m${RAIN_L}\x1b[m\x1b[1;36m\x1b[8m${RAIN_R}\x1b[28m`);
  });

  it("區段被切在多個 chunk 之間", () => {
    const filter = new DuotoneSplitFilter();
    const whole = `${DUO_START}雨\x1b[36m${DUO_END}後續`;
    let out = "";
    for (const ch of whole) out += filter.process(ch);
    expect(out).toBe(`\x1b[8m${RAIN_L}\x1b[36m\x1b[8m${RAIN_R}\x1b[28m後續`);
  });

  it("chunk 尾端的 ESC（可能是被切斷的 marker）扣到下一個 chunk，語意不變", () => {
    const filter = new DuotoneSplitFilter();
    const first = filter.process("abc\x1b");
    const second = filter.process("[31mX");
    expect(first + second).toBe("abc\x1b[31mX");
    expect(first).toBe("abc"); // ESC 被扣住
  });

  it("解碼失敗的字（U+FFFD）不拆，原樣送出", () => {
    const filter = new DuotoneSplitFilter();
    const out = filter.process(`${DUO_START}�\x1b[36m${DUO_END}`);
    expect(out).toBe("�\x1b[36m");
  });

  it("超出 BMP 的字不拆，原樣送出", () => {
    const filter = new DuotoneSplitFilter();
    const ext = String.fromCodePoint(0x20000); // CJK Ext B
    const out = filter.process(`${DUO_START}${ext}\x1b[36m${DUO_END}`);
    expect(out).toBe(`${ext}\x1b[36m`);
  });

  it("等不到區段結尾（超過上限）：放棄改寫原樣送出", () => {
    const filter = new DuotoneSplitFilter();
    let out = filter.process(`${DUO_START}雨`);
    expect(out).toBe(""); // 區段未收齊,扣住
    out += filter.process("x".repeat(200)); // 損壞:結尾永遠不來
    expect(out).toBe(`雨${"x".repeat(200)}`);
  });

  it("flush 吐回扣住的殘留", () => {
    const filter = new DuotoneSplitFilter();
    expect(filter.process(`${DUO_START}雨`)).toBe("");
    expect(filter.flush()).toBe(`${DUO_START}雨`);
    expect(filter.process("ok")).toBe("ok"); // flush 後狀態乾淨
  });
});

describe("端對端（實錄 fixture → 完整下行管線 → xterm buffer）", () => {
  it("登入畫面的雙色字以左右半字對進駐 buffer，全畫面無 FFFD", async () => {
    const fixture = new Uint8Array(
      readFileSync(new URL("./fixtures/welcome-big5.bin", import.meta.url)),
    );
    const term = makeTerm();
    const reorder = new SgrReorderFilter();
    const codec = new PttDecoder();
    const duotone = new DuotoneSplitFilter();
    await write(term, duotone.process(codec.decode(reorder.process(fixture))));

    // 解碼零損失:畫面上不得有任何替代字元
    const buffer = term.buffer.active;
    for (let row = 0; row < term.rows; row++) {
      expect(buffer.getLine(row)?.translateToString(true)).not.toContain("�");
    }

    // 雙色字成對進駐:每個左半的右鄰是同一個字的右半
    const cells = scanDuotoneCells(term);
    expect(cells.length).toBeGreaterThan(0);
    const lefts = cells.filter((c) => c.side === 0);
    expect(lefts.length).toBeGreaterThan(0);
    for (const left of lefts) {
      const right = cells.find((c) => c.row === left.row && c.col === left.col + 1);
      expect(right?.side).toBe(1);
      expect(right?.cp).toBe(left.cp);
    }

    // 至少存在一對左右異色(雙色資訊真的保留下來,而非兩半同色)
    expect(
      lefts.some((left) => {
        const right = cells.find((c) => c.row === left.row && c.col === left.col + 1);
        return right !== undefined && right.fg !== left.fg;
      }),
    ).toBe(true);
  });
});

describe("buildOrphanRepair（P6 孤兒半字補寫）", () => {
  it("空清單回傳空字串（呼叫端不多寫一次 write）", () => {
    expect(buildOrphanRepair([])).toBe("");
  });

  it("預設色左半：ESC7 + CUP + SGR 0 + conceal + PUA + ESC8", () => {
    const out = buildOrphanRepair([
      {
        row: 0,
        col: 1,
        glyph: "雨",
        side: "left",
        fg: -1,
        bg: -1,
        bold: false,
        blink: false,
        inverse: false,
      },
    ]);
    expect(out).toBe(`\x1b7\x1b[1;2H\x1b[0m\x1b[8m${RAIN_L}\x1b8`);
  });

  it("全屬性右半：bold/blink/inverse 與 fg/bg 逐一入列", () => {
    const out = buildOrphanRepair([
      {
        row: 17,
        col: 45,
        glyph: "▇",
        side: "right",
        fg: 1,
        bg: 7,
        bold: true,
        blink: true,
        inverse: true,
      },
    ]);
    const half = String.fromCodePoint(RIGHT_BASE + 0x2587);
    expect(out).toBe(`\x1b7\x1b[18;46H\x1b[0;1;5;7;31;47m\x1b[8m${half}\x1b8`);
  });

  it("U+FFFD 與超出 BMP 的字退為帶色空格（至少補對背景）", () => {
    const fffd = buildOrphanRepair([
      {
        row: 0,
        col: 0,
        glyph: "�",
        side: "left",
        fg: -1,
        bg: 1,
        bold: false,
        blink: false,
        inverse: false,
      },
    ]);
    expect(fffd).toBe("\x1b7\x1b[1;1H\x1b[0;41m \x1b8");
    const ext = buildOrphanRepair([
      {
        row: 2,
        col: 3,
        glyph: String.fromCodePoint(0x20000),
        side: "right",
        fg: 2,
        bg: -1,
        bold: false,
        blink: false,
        inverse: false,
      },
    ]);
    expect(ext).toBe("\x1b7\x1b[3;4H\x1b[0;32m \x1b8");
  });

  it("端對端進真 xterm：半字補回、游標與 SGR 還原到批末狀態", async () => {
    const term = makeTerm();
    const grid = new ScreenGrid();
    grid.switchToUtf8();
    const enc = new TextEncoder();
    const dec = new TextDecoder();
    // 批 1：寫「雨」pair
    let r = grid.feed(enc.encode("\x1b[1;1H雨"));
    await write(term, dec.decode(r.output));
    expect(r.repairs).toEqual([]);
    // 批 2：紅字空格蓋左半 → xterm 原子清空右半格，shadow 回報右半孤兒
    r = grid.feed(enc.encode("\x1b[31m\x1b[1;1H "));
    await write(term, dec.decode(r.output));
    expect(r.repairs).toHaveLength(1);
    expect(term.buffer.active.getLine(0)?.getCell(1)?.getChars()).toBe(""); // 原子清空實證
    // 補寫：右半 PUA 進格
    await write(term, buildOrphanRepair(r.repairs));
    const line = term.buffer.active.getLine(0);
    expect(line?.getCell(1)?.getChars()).toBe(RAIN_R);
    // 游標還原到批末位置（空格寫完後的 0,1）
    expect(term.buffer.active.cursorY).toBe(0);
    expect(term.buffer.active.cursorX).toBe(1);
    // SGR 還原到批末狀態（紅字）：接著寫的字必須仍是紅色
    await write(term, "Z");
    expect(line?.getCell(1)?.getChars()).toBe("Z");
    expect(line?.getCell(1)?.getFgColor()).toBe(1);
  });
});

describe("restoreDuotoneText（複製還原）", () => {
  it("相鄰的左右半合併為一個原字", () => {
    expect(restoreDuotoneText(`${RAIN_L}${RAIN_R}`)).toBe("雨");
  });

  it("落單的半字還原為原字", () => {
    expect(restoreDuotoneText(`${RAIN_L}x${RAIN_R}`)).toBe("雨x雨");
  });

  it("一般文字不受影響，真正的疊字不被合併", () => {
    expect(restoreDuotoneText("哈哈 abc 雨雨")).toBe("哈哈 abc 雨雨");
  });

  it("不同字的左右半不合併", () => {
    const otherR = String.fromCodePoint(RIGHT_BASE + 0x4e2d); // 中的右半
    expect(restoreDuotoneText(`${RAIN_L}${otherR}`)).toBe("雨中");
  });
});
