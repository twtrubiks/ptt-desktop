import { describe, expect, it } from "vitest";
import { TelnetFilter } from "../src/telnet";

const IAC = 0xff;
const SE = 0xf0;
const SB = 0xfa;
const WILL = 0xfb;
const DO = 0xfd;

const PREAMBLE = Array.from(new TextEncoder().encode("HTTP/1.1 200 OK\r\n\r\n"));

function feed(filter: TelnetFilter, bytes: number[]): number[] {
  return Array.from(filter.filter(Uint8Array.from(bytes)));
}

// 已吃完 HTTP 前導的濾層
function fresh(): TelnetFilter {
  const filter = new TelnetFilter();
  filter.filter(Uint8Array.from(PREAMBLE));
  return filter;
}

describe("TelnetFilter：HTTP 前導", () => {
  it("丟棄前導，保留其後資料", () => {
    const filter = new TelnetFilter();
    expect(feed(filter, [...PREAMBLE, 0x41, 0x42])).toEqual([0x41, 0x42]);
  });

  it("前導跨 frame 切割", () => {
    const filter = new TelnetFilter();
    expect(feed(filter, PREAMBLE.slice(0, 10))).toEqual([]);
    expect(feed(filter, [...PREAMBLE.slice(10), 0x41])).toEqual([0x41]);
  });

  it("前導的 \\r\\n\\r\\n 被切在正中間", () => {
    const filter = new TelnetFilter();
    const cut = PREAMBLE.length - 2; // \r\n | \r\n
    expect(feed(filter, PREAMBLE.slice(0, cut))).toEqual([]);
    expect(feed(filter, [...PREAMBLE.slice(cut), 0x41])).toEqual([0x41]);
  });

  it("無前導時（首 byte 為 IAC）直接處理 telnet 資料", () => {
    const filter = new TelnetFilter();
    expect(feed(filter, [IAC, DO, 0x18, 0x41])).toEqual([0x41]);
  });

  it("無前導時（開頭為一般資料）不吞資料", () => {
    const filter = new TelnetFilter();
    expect(feed(filter, [0x41, 0x42, 0x43])).toEqual([0x41, 0x42, 0x43]);
  });

  it("開頭部分匹配 HTTP 前導後不符，回放已收位元組", () => {
    const filter = new TelnetFilter();
    // "HTT" + "X"：前 3 bytes 曾被暫扣，確認不是前導後原樣回放
    expect(feed(filter, [0x48, 0x54, 0x54, 0x58])).toEqual([0x48, 0x54, 0x54, 0x58]);
  });

  it("部分匹配跨 frame 切割後不符，回放已收位元組", () => {
    const filter = new TelnetFilter();
    expect(feed(filter, [0x48, 0x54])).toEqual([]); // "HT" 暫扣中
    expect(feed(filter, [0x41])).toEqual([0x48, 0x54, 0x41]);
  });

  it("回放的位元組含 IAC 時仍走 telnet 處理", () => {
    const filter = new TelnetFilter();
    // "H" + IAC DO 0x18 + "A"：回放後 IAC 序列照常被濾掉
    expect(feed(filter, [0x48, IAC, DO, 0x18, 0x41])).toEqual([0x48, 0x41]);
  });
});

describe("TelnetFilter：IAC 協商", () => {
  it("濾掉 DO/WILL + 選項 byte", () => {
    const filter = fresh();
    // 實測連線即收到的協商型式：IAC DO TERMINAL-TYPE、IAC WILL ECHO
    expect(feed(filter, [IAC, DO, 0x18, 0x41, IAC, WILL, 0x01, 0x42])).toEqual([0x41, 0x42]);
  });

  it("IAC 序列跨 frame 切割", () => {
    const filter = fresh();
    expect(feed(filter, [0x41, IAC])).toEqual([0x41]);
    expect(feed(filter, [DO])).toEqual([]);
    expect(feed(filter, [0x18, 0x42])).toEqual([0x42]);
  });

  it("濾掉 SB 子協商（IAC SB ... IAC SE）", () => {
    const filter = fresh();
    // IAC SB TERMINAL-TYPE SEND IAC SE
    expect(feed(filter, [IAC, SB, 0x18, 0x01, IAC, SE, 0x41])).toEqual([0x41]);
  });

  it("SB 子協商跨 frame 切割", () => {
    const filter = fresh();
    expect(feed(filter, [IAC, SB, 0x18])).toEqual([]);
    expect(feed(filter, [0x01, IAC])).toEqual([]);
    expect(feed(filter, [SE, 0x41])).toEqual([0x41]);
  });

  it("IAC IAC 還原為資料中的 0xFF", () => {
    const filter = fresh();
    expect(feed(filter, [0x41, IAC, IAC, 0x42])).toEqual([0x41, 0xff, 0x42]);
  });

  it("濾掉單 byte 指令（如 NOP）", () => {
    const filter = fresh();
    expect(feed(filter, [IAC, 0xf1, 0x41])).toEqual([0x41]);
  });

  it("reset 後重新等待 HTTP 前導", () => {
    const filter = fresh();
    feed(filter, [0x41]);
    filter.reset();
    expect(feed(filter, [...PREAMBLE, 0x42])).toEqual([0x42]);
  });
});
