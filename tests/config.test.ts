import { describe, expect, it } from "vitest";
import { sanitizeLineHeight } from "../src/config";

// xterm 對 lineHeight < 1 會拋例外（OptionsService.ts:177），非法值若原樣傳進
// createTerminal，終端整個建不起來 → 這裡守的是「手改設定檔不會讓 app 開不了」
describe("sanitizeLineHeight", () => {
  it("合法倍率原樣通過(含邊界 1 與上限 2)", () => {
    expect(sanitizeLineHeight(1)).toBe(1);
    expect(sanitizeLineHeight(1.08)).toBe(1.08);
    expect(sanitizeLineHeight(1.15)).toBe(1.15);
    expect(sanitizeLineHeight(2)).toBe(2);
  });

  it("小於 1 夾到下限 1 而非退回預設:填了數字就尊重意圖,只擋 xterm 會拋例外的範圍", () => {
    expect(sanitizeLineHeight(0.9)).toBe(1);
    expect(sanitizeLineHeight(0)).toBe(1);
    expect(sanitizeLineHeight(-3)).toBe(1);
  });

  it("超過上限夾到 2:再大只是整片空白,無實用意義", () => {
    expect(sanitizeLineHeight(5)).toBe(2);
    expect(sanitizeLineHeight(1000)).toBe(2);
  });

  // 以下兩個 case 目前期望值都是 1（預設值恰與下限重合），仍分開測:兩條是不同
  // 語意路徑(垃圾值退預設 vs 有限數夾邊界),預設值日後若右移就會分岔
  it("非有限數回退預設 1:垃圾值當成沒填,不是夾到邊界", () => {
    expect(sanitizeLineHeight(undefined)).toBe(1);
    expect(sanitizeLineHeight("abc")).toBe(1);
    expect(sanitizeLineHeight(Number.NaN)).toBe(1);
    expect(sanitizeLineHeight({})).toBe(1);
    expect(sanitizeLineHeight(Number.POSITIVE_INFINITY)).toBe(1);
  });

  it("null 轉數字為 0,走 clamp 得下限 1", () => {
    expect(sanitizeLineHeight(null)).toBe(1);
  });

  it("JSON 寫成字串數字仍可用:設定檔手寫易誤植引號", () => {
    expect(sanitizeLineHeight("1.2")).toBe(1.2);
  });
});
