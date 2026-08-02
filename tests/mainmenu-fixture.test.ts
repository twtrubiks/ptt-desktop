// mainmenu-utf8.bin 格式守門：framing 完整、嚴格 UTF-8、個資遮蔽不回歸。
// 格式（P4 產製，P5 重放測試據此解析）：
//   magic "PTTREC1\n" + 重複 [u32 big-endian 長度][chunk bytes]
//   chunk 邊界 = 實錄當下的 WebSocket frame 邊界（跨 frame 案例的來源）
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const FIXTURE = new URL("./fixtures/mainmenu-utf8.bin", import.meta.url);
const MAGIC = "PTTREC1\n";

function parseChunks(data: Buffer): Buffer[] {
  expect(data.subarray(0, 8).toString("latin1")).toBe(MAGIC);
  const chunks: Buffer[] = [];
  let i = 8;
  while (i < data.length) {
    const len = data.readUInt32BE(i);
    i += 4;
    expect(i + len).toBeLessThanOrEqual(data.length); // 長度欄不可指出檔尾
    chunks.push(data.subarray(i, i + len));
    i += len;
  }
  expect(i).toBe(data.length); // 恰好走完，無殘餘垃圾 bytes
  return chunks;
}

// 解析放 helper、斷言在各 it 內執行（framing 壞掉時錯誤落在明確的測試裡）
let cache: Buffer[] | null = null;
function loadChunks(): Buffer[] {
  cache ??= parseChunks(readFileSync(FIXTURE));
  return cache;
}

describe("mainmenu-utf8.bin fixture", () => {
  it("framing 可完整解析且保留多個 chunk 邊界", () => {
    const chunks = loadChunks();
    expect(chunks.length).toBeGreaterThan(50);
    expect(Buffer.concat(chunks).length).toBeGreaterThan(50000);
  });

  it("整段為嚴格合法 UTF-8（串接後；單一 chunk 可切在字中間）", () => {
    const whole = Buffer.concat(loadChunks());
    const text = new TextDecoder("utf-8", { fatal: true }).decode(whole);
    expect(text).toContain("主功能表"); // 確為主功能表段
  });

  it("個資遮蔽不回歸：狀態列 ID 全為遮蔽字元、無 IP 樣態", () => {
    const text = new TextDecoder("utf-8").decode(Buffer.concat(loadChunks()));
    // 狀態列實際樣態為「我是<SGR>xxxxxxxxx」——ID 前可夾 SGR 色碼，
    // regex 必須跨過色碼才抓得到 ID；並斷言命中數防止 regex 空轉假成功
    const ids = [...text.matchAll(/我是(?:\x1b\[[0-9;]*m)*([A-Za-z0-9]+)/g)];
    expect(ids.length).toBeGreaterThanOrEqual(4);
    for (const m of ids) {
      expect(m[1]).toMatch(/^x+$/);
    }
    // IP 樣態掃兩層：原文 + 剝除 SGR 後（防 IP 被色碼切開而漏抓）
    const stripped = text.replace(/\x1b\[[0-9;]*[A-Za-z]/g, "");
    expect(text).not.toMatch(/\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}/);
    expect(stripped).not.toMatch(/\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}/);
  });
});
