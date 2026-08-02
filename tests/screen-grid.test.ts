// ScreenGrid：byte 網格 shadow 的單元測試。
// fixture 重放的期望值經獨立 Python 對照實作交叉驗證後鎖定（P5 contract），
// 語意規格見 docs/architecture.md「byte 網格語意規格」。
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { ScreenGrid, type RepairCell } from "../src/screen-grid";

const enc = new TextEncoder();

function feedText(grid: ScreenGrid, text: string) {
  return grid.feed(enc.encode(text)).repairs;
}

function utf8Grid(): ScreenGrid {
  const grid = new ScreenGrid();
  grid.switchToUtf8();
  return grid;
}

function loadFixtureChunks(): Uint8Array[] {
  const data = readFileSync(new URL("./fixtures/mainmenu-utf8.bin", import.meta.url));
  const chunks: Uint8Array[] = [];
  let i = 8;
  while (i < data.length) {
    const len = data.readUInt32BE(i);
    i += 4;
    chunks.push(new Uint8Array(data.subarray(i, i + len)));
    i += len;
  }
  return chunks;
}

describe("utf-8 模式：半字覆寫（第五根因核心）", () => {
  it("空格蓋左半 → 回報右半孤兒（xterm 端整格已被清）", () => {
    const grid = utf8Grid();
    expect(feedText(grid, "\x1b[1;1H雨")).toEqual([]); // 完整 pair 無孤兒
    const repairs = feedText(grid, "\x1b[1;1H ");
    expect(repairs).toEqual([
      {
        row: 0,
        col: 1,
        glyph: "雨",
        side: "right",
        fg: -1,
        bg: -1,
        bold: false,
        blink: false,
        inverse: false,
      },
    ]);
    expect(grid.cellAt(0, 1).paired).toBe(false);
    expect(grid.cellAt(0, 0).glyph).toBe(" ");
  });

  it("空格蓋右半 → 回報左半孤兒", () => {
    const grid = utf8Grid();
    feedText(grid, "\x1b[1;1H雨");
    const repairs = feedText(grid, "\x1b[1;2H ");
    expect(repairs).toHaveLength(1);
    expect(repairs[0]).toMatchObject({ row: 0, col: 0, glyph: "雨", side: "left" });
  });

  it("寬字錯相疊寫 → 被劈 pair 的另一半成孤兒", () => {
    const grid = utf8Grid();
    feedText(grid, "\x1b[1;1H雨"); // 佔 1-2 欄
    const repairs = feedText(grid, "\x1b[1;2H天"); // 佔 2-3 欄，劈掉雨的右半
    expect(repairs).toHaveLength(1);
    expect(repairs[0]).toMatchObject({ row: 0, col: 0, glyph: "雨", side: "left" });
    expect(grid.cellAt(0, 1)).toMatchObject({ glyph: "天", half: 1, paired: true });
  });

  it("同批自癒不回報：蓋半字後同批把兩欄都寫掉", () => {
    const grid = utf8Grid();
    feedText(grid, "\x1b[1;1H雨");
    expect(feedText(grid, "\x1b[1;1H \x1b[1;2H天")).toEqual([]);
  });

  it("孤兒只在受影響批次回報一次，後續批不重複", () => {
    const grid = utf8Grid();
    feedText(grid, "\x1b[1;1H雨");
    expect(feedText(grid, "\x1b[1;1H ")).toHaveLength(1);
    expect(feedText(grid, "\x1b[5;5HZ")).toEqual([]); // 無關寫入不再回報舊孤兒
  });

  it("動態看板文法「空格×2 BS×2 寬字」：對齊無孤兒、錯相劈鄰字", () => {
    const grid = utf8Grid();
    feedText(grid, "\x1b[1;1H雨雨"); // pair 佔 1-2、3-4
    expect(feedText(grid, "\x1b[1;1H  \x08\x08天")).toEqual([]); // 對齊：清 1-2 寫 1-2
    // 錯相：清 2-3 欄再寫 2-3——上一批「天」的左半與第二個「雨」的右半同時被劈
    const repairs = feedText(grid, "\x1b[1;2H  \x08\x08天");
    expect(repairs).toHaveLength(2);
    expect(repairs[0]).toMatchObject({ row: 0, col: 0, glyph: "天", side: "left" });
    expect(repairs[1]).toMatchObject({ row: 0, col: 3, glyph: "雨", side: "right" });
  });

  it("EL 清到行尾切在 pair 中間 → 範圍外左半成孤兒", () => {
    const grid = utf8Grid();
    feedText(grid, "\x1b[1;5H雨"); // 佔 5-6 欄
    const repairs = feedText(grid, "\x1b[1;6H\x1b[K"); // 從右半清到行尾
    expect(repairs).toHaveLength(1);
    expect(repairs[0]).toMatchObject({ row: 0, col: 4, glyph: "雨", side: "left" });
  });

  it("ED 全清（2J）：兩模型同步清空，無孤兒", () => {
    const grid = utf8Grid();
    feedText(grid, "\x1b[1;1H雨天");
    expect(feedText(grid, "\x1b[2J")).toEqual([]);
    expect(grid.rowText(0).trim()).toBe("");
  });

  it("行尾邊界：CUP(1,80) 空格蓋 trail → 79 欄左半孤兒（批末行尾虛位，遞延一批）", () => {
    const grid = utf8Grid();
    feedText(grid, "\x1b[1;79H雨"); // 佔 79-80 欄
    expect(feedText(grid, "\x1b[1;80H ")).toEqual([]); // 批末 col=80 不可注入，遞延
    const repairs = feedText(grid, "\x1b[2;1H"); // 游標離開行尾虛位即回報
    expect(repairs).toHaveLength(1);
    expect(repairs[0]).toMatchObject({ row: 0, col: 78, glyph: "雨", side: "left" });
  });

  it("行尾虛位：寫滿 80 欄後 BS×2 回到 78 欄（byte 網格語意，非 xterm pending-wrap）", () => {
    // xterm 的 BS 是「夾 79 再減 → 78」，兩個 BS 會落到 77 → 行尾整段左移一欄。
    // shadow 以 PTT server 意圖（純減一）為準建 truth；此分歧由 feed() 的
    // BS@80 正規化在源頭消除（見下方「BS@80 正規化」測試群）
    const grid = utf8Grid();
    feedText(grid, "\x1b[1;79H12\x08\x08雨"); // 寫滿 79-80 → 虛位 80 → BS×2 → 寬字
    expect(grid.cellAt(0, 78)).toMatchObject({ glyph: "雨", half: 1, paired: true });
    expect(grid.cellAt(0, 79)).toMatchObject({ glyph: "雨", half: 2 });
  });
});

describe("SGR 劈字（duotone 語意）", () => {
  it("UTF-8 序列中間夾 SGR 仍成字，左右半各持自己的顏色", () => {
    const grid = utf8Grid();
    const bytes = enc.encode("雨"); // e9 9b a8
    const stream = Uint8Array.from([
      0x1b,
      0x5b,
      0x31,
      0x3b,
      0x33,
      0x31,
      0x6d, // ESC[1;31m 亮紅
      bytes[0],
      bytes[1],
      0x1b,
      0x5b,
      0x33,
      0x34,
      0x6d, // ESC[34m 藍（劈在第 2、3 byte 之間）
      bytes[2],
    ]);
    expect(grid.feed(stream).repairs).toEqual([]);
    expect(grid.cellAt(0, 0)).toMatchObject({ glyph: "雨", half: 1, fg: 1, bold: true });
    expect(grid.cellAt(0, 1)).toMatchObject({ glyph: "雨", half: 2, fg: 4, bold: true });
  });

  it("孤兒半字帶著自己那半的 SGR 回報", () => {
    const grid = utf8Grid();
    feedText(grid, "\x1b[1;31m\x1b[1;1H雨");
    const repairs = feedText(grid, "\x1b[m\x1b[1;1H ");
    expect(repairs[0]).toMatchObject({ side: "right", fg: 1, bold: true });
  });
});

describe("跨 chunk 狀態機", () => {
  it("UTF-8 三 byte 字在任意位置切割，結果與一次餵完相同", () => {
    const whole = utf8Grid();
    feedText(whole, "\x1b[3;7H雨");
    const bytes = enc.encode("\x1b[3;7H雨");
    for (let cut = 1; cut < bytes.length; cut++) {
      const grid = utf8Grid();
      grid.feed(bytes.subarray(0, cut));
      grid.feed(bytes.subarray(cut));
      expect(grid.cellAt(2, 6), `cut=${cut}`).toEqual(whole.cellAt(2, 6));
      expect(grid.cellAt(2, 7), `cut=${cut}`).toEqual(whole.cellAt(2, 7));
    }
  });

  it("CSI 參數切在 chunk 邊界", () => {
    const grid = utf8Grid();
    grid.feed(Uint8Array.from([0x1b, 0x5b, 0x31, 0x32])); // ESC[12
    grid.feed(Uint8Array.from([0x3b, 0x33, 0x34, 0x48])); // ;34H
    feedText(grid, "A");
    expect(grid.cellAt(11, 33).glyph).toBe("A");
  });

  it("孤兒判定跨批成立：蓋左半與後續寫入分屬不同批", () => {
    const grid = utf8Grid();
    feedText(grid, "\x1b[1;1H雨");
    const r1 = feedText(grid, "\x1b[1;1H "); // 批 N：孤兒立刻回報
    const r2 = feedText(grid, "\x1b[1;2H天"); // 批 N+1：孤兒被新字覆蓋，無新孤兒
    expect(r1).toHaveLength(1);
    expect(r2).toEqual([]);
  });
});

describe("big5 模式（登入前）", () => {
  it("lead+trail 相鄰配對成字", () => {
    const grid = new ScreenGrid();
    grid.feed(Uint8Array.from([0xa4, 0x40])); // 一
    expect(grid.cellAt(0, 0)).toMatchObject({ glyph: "一", half: 1, paired: true });
    expect(grid.rowText(0).trim()).toBe("一");
  });

  it("lead 與 trail 之間夾 SGR 仍配對（劈字彩繪），左右半異色", () => {
    const grid = new ScreenGrid();
    grid.feed(
      Uint8Array.from([
        0x1b,
        0x5b,
        0x33,
        0x31,
        0x6d, // ESC[31m
        0xa4,
        0x1b,
        0x5b,
        0x33,
        0x34,
        0x6d, // ESC[34m
        0x40,
      ]),
    );
    expect(grid.cellAt(0, 0)).toMatchObject({ glyph: "一", half: 1, fg: 1 });
    expect(grid.cellAt(0, 1)).toMatchObject({ glyph: "一", half: 2, fg: 4 });
  });

  it("空格蓋 trail → 回報 left 孤兒（字形已知）", () => {
    const grid = new ScreenGrid();
    grid.feed(Uint8Array.from([0xa4, 0x40]));
    const repairs = feedText(grid, "\x1b[1;2H ");
    expect(repairs).toHaveLength(1);
    expect(repairs[0]).toMatchObject({ row: 0, col: 0, glyph: "一", side: "left" });
  });

  it("孤兒 lead byte（trail 未到就移游標）不配對、不回報、byte 可查", () => {
    const grid = new ScreenGrid();
    const { repairs } = grid.feed(
      Uint8Array.from([0xa4, ...enc.encode("\x1b[5;1H"), 0x41]), // lead 後游標跳走
    );
    expect(repairs).toEqual([]);
    expect(grid.cellAt(0, 0)).toMatchObject({ glyph: "", half: 0, byte: 0xa4 });
    expect(grid.rowText(0)[0]).toBe("?");
    expect(grid.cellAt(4, 0).glyph).toBe("A");
  });

  it("lead/trail 切在 chunk 邊界仍配對", () => {
    const grid = new ScreenGrid();
    grid.feed(Uint8Array.from([0xa4]));
    grid.feed(Uint8Array.from([0x40]));
    expect(grid.cellAt(0, 0)).toMatchObject({ glyph: "一", paired: true });
  });

  it("switchToUtf8 後改以 UTF-8 解讀，寬窄與 xterm provider 一致", () => {
    const grid = new ScreenGrid();
    grid.feed(Uint8Array.from([0x41])); // big5 模式 ASCII
    grid.switchToUtf8();
    feedText(grid, "雨▄"); // CJK 與 EAW Ambiguous 方塊都是 2 欄
    expect(grid.cellAt(0, 1)).toMatchObject({ glyph: "雨", half: 1 });
    expect(grid.cellAt(0, 3)).toMatchObject({ glyph: "▄", half: 1 });
  });

  it("reset 回到 big5 模式並清空網格", () => {
    const grid = utf8Grid();
    feedText(grid, "雨");
    grid.reset();
    expect(grid.rowText(0).trim()).toBe("");
    grid.feed(Uint8Array.from([0xa4, 0x40])); // reset 後 big5 又可配對
    expect(grid.cellAt(0, 0).glyph).toBe("一");
  });
});

describe("游標運動", () => {
  it("CUP/CUU/CUD/CUF/CUB/CR/LF/BS 與夾限", () => {
    const grid = utf8Grid();
    feedText(grid, "\x1b[12;40H");
    expect(grid.cursor).toEqual({ row: 11, col: 39 });
    feedText(grid, "\x1b[3A\x1b[2C");
    expect(grid.cursor).toEqual({ row: 8, col: 41 });
    feedText(grid, "\x1b[5D\x1b[2B");
    expect(grid.cursor).toEqual({ row: 10, col: 36 });
    feedText(grid, "\r\n");
    expect(grid.cursor).toEqual({ row: 11, col: 0 });
    feedText(grid, "\x08"); // 行首 BS 夾住
    expect(grid.cursor).toEqual({ row: 11, col: 0 });
    feedText(grid, "\x1b[99;99H"); // 超界夾到 24,80
    expect(grid.cursor).toEqual({ row: 23, col: 79 });
    feedText(grid, "\x1b[1;81H"); // 實錄出現過 CUP 到 81 欄
    expect(grid.cursor).toEqual({ row: 0, col: 79 });
  });
});

describe("BS@80 正規化（P6：xterm pending-wrap 語意錯位的源頭修復）", () => {
  it("行尾虛位的 BS 替換為絕對 CUP（同列 80 欄），後續 BS 原樣", () => {
    const grid = utf8Grid();
    const { output } = grid.feed(enc.encode("\x1b[1;79H12\x08\x08雨"));
    expect(new TextDecoder().decode(output)).toBe("\x1b[1;79H12\x1b[1;80H\x08雨");
  });

  it("無 BS@80 時零拷貝回傳原陣列（行中 BS 不受影響）", () => {
    const grid = utf8Grid();
    const bytes = enc.encode("\x1b[1;1HAB\x08C");
    expect(grid.feed(bytes).output).toBe(bytes);
  });

  it("跨批：寫滿行尾與 BS 分屬不同批仍正規化", () => {
    const grid = utf8Grid();
    grid.feed(enc.encode("\x1b[3;79H12"));
    const { output } = grid.feed(enc.encode("\x08\x08天"));
    expect(new TextDecoder().decode(output)).toBe("\x1b[3;80H\x08天");
  });

  it("BS 劈在多位元組字中間（防禦分支）：不替換，退回 xterm 原生 BS 行為", () => {
    // big5：lead 落在 80 欄後接 BS——leadIdx 進行中不注入（CUP 會劈壞 lead/trail 配對）
    const b5 = new ScreenGrid();
    const b5bytes = Uint8Array.from([...enc.encode("\x1b[1;80H"), 0xa4, 0x08]);
    expect(b5.feed(b5bytes).output).toBe(b5bytes);
    // utf-8：三 byte 字收到一半接 BS——u8 累積進行中不注入
    const u8 = utf8Grid();
    u8.feed(enc.encode("\x1b[1;79H12"));
    const rain = enc.encode("雨");
    const mid = Uint8Array.from([rain[0], 0x08, rain[1], rain[2]]);
    expect(u8.feed(mid).output).toBe(mid);
  });

  it("正規化不改變 shadow 語意（仍為純減一落點）", () => {
    const grid = utf8Grid();
    grid.feed(enc.encode("\x1b[1;79H12\x08\x08雨"));
    expect(grid.cellAt(0, 78)).toMatchObject({ glyph: "雨", half: 1, paired: true });
    expect(grid.cellAt(0, 79)).toMatchObject({ glyph: "雨", half: 2 });
  });
});

describe("補寫遞延（批末不可注入時 dirty 跨批累積）", () => {
  it("批末停在 CSI 中途 → 本批不回報（注入會劈壞跨批序列），補完後回報", () => {
    const grid = utf8Grid();
    feedText(grid, "\x1b[1;1H雨");
    const r1 = grid.feed(enc.encode("\x1b[1;1H \x1b[3")); // 孤兒已生，批末在 CSI 中途
    expect(r1.repairs).toEqual([]);
    const r2 = grid.feed(enc.encode("1m"));
    expect(r2.repairs).toHaveLength(1);
    expect(r2.repairs[0]).toMatchObject({ row: 0, col: 1, glyph: "雨", side: "right" });
  });

  it("批末停在行尾虛位（col=80，DECRC 會丟 pending-wrap）→ 游標離開後回報", () => {
    const grid = utf8Grid();
    feedText(grid, "\x1b[1;1H雨");
    const r1 = grid.feed(enc.encode("\x1b[1;1H \x1b[2;79H12")); // 孤兒 + 批末 col=80
    expect(r1.repairs).toEqual([]);
    const r2 = grid.feed(enc.encode("\x1b[5;5HZ"));
    expect(r2.repairs).toHaveLength(1);
    expect(r2.repairs[0]).toMatchObject({ row: 0, col: 1, glyph: "雨", side: "right" });
  });

  it("遞延期間被後續寫入自癒的孤兒不回報", () => {
    const grid = utf8Grid();
    feedText(grid, "\x1b[1;1H雨");
    expect(grid.feed(enc.encode("\x1b[1;1H \x1b[3")).repairs).toEqual([]); // 遞延
    expect(grid.feed(enc.encode("1m\x1b[1;1H天")).repairs).toEqual([]); // 兩欄被新字蓋掉
  });
});

describe("fixture 重放（期望值經獨立 Python 實作交叉驗證）", () => {
  const chunks = loadFixtureChunks();

  function replay() {
    const grid = utf8Grid();
    const all: RepairCell[] = [];
    for (const c of chunks) all.push(...grid.feed(c).repairs);
    return { grid, all };
  }

  it("主功能表段可補寫孤兒共 5 件，逐項與交叉驗證結果一致", () => {
    const { all } = replay();
    // 動態看板動畫的批內 orphan 事件 5253 件、自癒率 99.8%——
    // xterm 一次 write() 吃整個 chunk，批內暫態不上畫面，僅存活者需補寫。
    // P5 錄得批末孤兒 8 件；P6 補寫遞延（批末停在序列中途／行尾虛位時不可
    // 注入）後，其中 3 件（23,50全／6,29◎／18,50進）的孤兒化批批末不可注入、
    // 遞延期間即被下一批動畫蓋掉自癒（xterm 端同批修復），無需補寫——
    // 逐批溯源驗證記於 Plans.md P6。其餘 5 件回報批次與 P5 一致
    expect(all.map((r) => [r.row, r.col, r.side, r.glyph, r.fg, r.bg].join("|"))).toEqual([
      "7|55|right|進|-1|-1",
      "16|58|right|底|-1|-1",
      "17|45|right|▇|1|-1",
      "9|29|right|◎|-1|-1",
      "6|48|right|規|-1|-1",
    ]);
  });

  it("重放後畫面重建正確（標題列、遮蔽後狀態列、游標落點）", () => {
    const { grid } = replay();
    expect(grid.rowText(0)).toContain("主功能表");
    expect(grid.rowText(0)).toContain("批踢踢實業坊");
    expect(grid.rowText(23)).toContain("我是xxxxxxxxx"); // fixture 已遮蔽
    expect(grid.cursor).toEqual({ row: 14, col: 20 });
  });

  it("效能 sanity：整段重放 ×10 遠低於秒級（防 O(n²) 回歸）", () => {
    const t0 = performance.now();
    for (let n = 0; n < 10; n++) replay();
    expect(performance.now() - t0).toBeLessThan(1000); // 本機實測約 60ms
  });
});
