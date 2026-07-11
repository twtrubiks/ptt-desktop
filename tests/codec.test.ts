import { describe, expect, it } from "vitest";
import { PttDecoder } from "../src/codec";

describe("PttDecoder", () => {
  it("登入前以 big5 解碼", () => {
    const codec = new PttDecoder();
    // 「中文」的 big5 編碼
    expect(codec.decode(Uint8Array.from([0xa4, 0xa4, 0xa4, 0xe5]))).toBe("中文");
  });

  it("big5 多位元組字跨 frame 切割", () => {
    const codec = new PttDecoder();
    expect(codec.decode(Uint8Array.from([0xa4]))).toBe("");
    expect(codec.decode(Uint8Array.from([0xa4]))).toBe("中");
  });

  it("switchToUtf8 後以 utf-8 解碼", () => {
    const codec = new PttDecoder();
    codec.switchToUtf8();
    expect(codec.decode(new TextEncoder().encode("中文"))).toBe("中文");
  });

  it("utf-8 多位元組字跨 frame 切割", () => {
    const codec = new PttDecoder();
    codec.switchToUtf8();
    const bytes = new TextEncoder().encode("中"); // 3 bytes
    expect(codec.decode(bytes.slice(0, 2))).toBe("");
    expect(codec.decode(bytes.slice(2))).toBe("中");
  });

  it("switchToUtf8 flush 殘留的半個 big5 字為 U+FFFD", () => {
    const codec = new PttDecoder();
    expect(codec.decode(Uint8Array.from([0xa4]))).toBe(""); // 「中」的前半
    expect(codec.switchToUtf8()).toBe("�");
    expect(codec.decode(new TextEncoder().encode("中文"))).toBe("中文");
  });

  it("switchToUtf8 無殘留時 flush 為空字串", () => {
    const codec = new PttDecoder();
    codec.decode(Uint8Array.from([0xa4, 0xa4])); // 完整的「中」
    expect(codec.switchToUtf8()).toBe("");
  });

  it("非法位元組以 U+FFFD 替代且串流不中斷", () => {
    const codec = new PttDecoder();
    codec.switchToUtf8();
    expect(codec.decode(Uint8Array.from([0xff, 0x41]))).toBe("�A");
  });

  it("reset 後回到 big5", () => {
    const codec = new PttDecoder();
    codec.switchToUtf8();
    codec.reset();
    expect(codec.decode(Uint8Array.from([0xa4, 0xa4]))).toBe("中");
  });
});
