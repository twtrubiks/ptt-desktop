import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { DUO_END, DUO_START } from "../src/duotone";
import { SgrReorderFilter } from "../src/sgr-reorder";

const big5 = () => new TextDecoder("big5");
const bytes = (...parts: (number[] | string)[]) =>
  Uint8Array.from(
    parts.flatMap((p) => (typeof p === "string" ? [...p].map((c) => c.charCodeAt(0)) : p)),
  );

describe("SgrReorderFilter(big5 模式)", () => {
  it("SGR 插在 Big5 字中間:搬到字後並包進 APC 區段,decoder 可正確配對", () => {
    const filter = new SgrReorderFilter();
    // ▁ 的 big5 是 0xA2 0x62,中間插 ESC[31m(2026-07-13 實錄登入畫面的實際樣式)
    const out = filter.process(bytes([0xa2], "\x1b[31m", [0x62]));
    expect([...out]).toEqual([...bytes(DUO_START, [0xa2, 0x62], "\x1b[31m", DUO_END)]);
    expect(big5().decode(out)).toBe(`${DUO_START}▁\x1b[31m${DUO_END}`);
  });

  it("連續多個 SGR 夾在字中間:全部搬到字後,維持相對順序", () => {
    const filter = new SgrReorderFilter();
    const out = filter.process(bytes([0xa2], "\x1b[1m\x1b[;31m", [0x62]));
    expect(big5().decode(out)).toBe(`${DUO_START}▁\x1b[1m\x1b[;31m${DUO_END}`);
  });

  it("字元與 SGR 被切在多個 frame 之間", () => {
    const filter = new SgrReorderFilter();
    const chunks = [
      filter.process(bytes([0xa2])),
      filter.process(bytes("\x1b[3")),
      filter.process(bytes("1m")),
      filter.process(bytes([0x62])),
    ];
    const total = bytes(...chunks.map((c) => [...c]));
    expect(big5().decode(total)).toBe(`${DUO_START}▁\x1b[31m${DUO_END}`);
  });

  it("非 SGR 的 CSI(游標移動)插在字中間:不重排,原樣送出", () => {
    const filter = new SgrReorderFilter();
    const input = bytes([0xa2], "\x1b[2;3H", [0x62]);
    expect([...filter.process(input)]).toEqual([...input]);
  });

  it("非 CSI 的 ESC 序列插在字中間:不重排,原樣送出", () => {
    const filter = new SgrReorderFilter();
    const input = bytes([0xa2], "\x1bM", [0x62]);
    expect([...filter.process(input)]).toEqual([...input]);
  });

  it("一般資料(ASCII、完整 Big5 字、獨立 SGR)原樣通過", () => {
    const filter = new SgrReorderFilter();
    const input = bytes("ab\x1b[31m", [0xa4, 0xa4], "\x1b[m!");
    expect([...filter.process(input)]).toEqual([...input]);
  });

  it("frame 尾的半個字先扣住,下一個 frame 補齊後送出", () => {
    const filter = new SgrReorderFilter();
    expect(filter.process(bytes([0xa4])).length).toBe(0);
    expect(big5().decode(filter.process(bytes([0xa4])))).toBe("中");
  });
});

describe("SgrReorderFilter(utf-8 模式)", () => {
  it("SGR 插在 UTF-8 序列中間:搬到字後", () => {
    const filter = new SgrReorderFilter();
    filter.switchToUtf8();
    const zhong = [...new TextEncoder().encode("中")]; // e4 b8 ad
    const out = filter.process(bytes([zhong[0]], "\x1b[31m", [zhong[1]], "\x1b[1m", [zhong[2]]));
    expect(new TextDecoder().decode(out)).toBe(`${DUO_START}中\x1b[31m\x1b[1m${DUO_END}`);
  });

  it("switchToUtf8 回傳扣住的殘留 bytes,不靜默丟棄", () => {
    const filter = new SgrReorderFilter();
    filter.process(bytes([0xa4])); // 半個 big5 字扣在濾層裡
    expect([...filter.switchToUtf8()]).toEqual([0xa4]);
  });

  it("孤立的 continuation byte 原樣通過(交給 decoder 處理)", () => {
    const filter = new SgrReorderFilter();
    filter.switchToUtf8();
    expect([...filter.process(bytes([0x80, 0xbf]))]).toEqual([0x80, 0xbf]);
  });
});

describe("SgrReorderFilter(實錄 fixture)", () => {
  // 2026-07-13 以 guest 實錄的登入前畫面(telnet 濾層後),含 111 處雙色字切割
  const fixture = new Uint8Array(
    readFileSync(new URL("./fixtures/welcome-big5.bin", import.meta.url)),
  );

  const countSplits = (data: Uint8Array): number => {
    let n = 0;
    for (let i = 0; i < data.length - 1; i++) {
      const b = data[i];
      if (b >= 0x81 && b <= 0xfe) {
        if (data[i + 1] === 0x1b) n++;
        i++; // 跳過 trail byte
      }
    }
    return n;
  };
  const countFffd = (data: Uint8Array): number =>
    [...big5().decode(data)].filter((c) => c === "�").length;

  it("過濾前後基準:111 處切割 → 0;FFFD 126 → 0", () => {
    expect(countSplits(fixture)).toBe(111);
    expect(countFffd(fixture)).toBe(126);

    const filter = new SgrReorderFilter();
    const out = filter.process(fixture);
    expect(countSplits(out)).toBe(0);
    // 此畫面的 FFFD 全數來自雙色字切割,重排後可完整解碼
    expect(countFffd(out)).toBe(0);
  });

  it("逐 byte 餵入(最壞切割)與整段餵入結果一致", () => {
    const whole = new SgrReorderFilter().process(fixture);
    const filter = new SgrReorderFilter();
    const parts: number[] = [];
    for (const b of fixture) parts.push(...filter.process(Uint8Array.of(b)));
    expect(parts).toEqual([...whole]);
  });
});
