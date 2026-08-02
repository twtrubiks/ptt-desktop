// 字型覆蓋守門（P7）：customGlyphs 停用後，框線/方塊（U+2500–259F）完全依賴捆綁
// 字型 PTT Square Mono 的字形——缺字會 fallback 到系統 monospace（寬度與造型不可
// 控，分隔線退化）；advance 若非全形（1em = 2 格）則右半格空白、虛線復發。
// 本測試直接解析 src/assets/ptt-square-mono.woff2 驗證 cmap 與 hmtx：字型重製
// （scripts/build-font.py 換來源版本重跑）若丟字形或改寬度，這裡會先紅。
// WOFF2 解析零依賴：node:zlib 的 brotli 解壓＋依 W3C WOFF2 規格走 table directory
// （CFF 字型無 transformed glyf/loca，所有表原樣串接）。
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { brotliDecompressSync } from "node:zlib";
import { describe, expect, it } from "vitest";

// WOFF2 規格的已知 table tag 表：directory entry 首 byte 低 6 bits 為索引，
// 0x3f 表示 tag 以 4 bytes 明寫跟隨
// prettier-ignore
const KNOWN_TAGS = [
  "cmap", "head", "hhea", "hmtx", "maxp", "name", "OS/2", "post",
  "cvt ", "fpgm", "glyf", "loca", "prep", "CFF ", "VORG", "EBDT",
  "EBLC", "gasp", "hdmx", "kern", "LTSH", "PCLT", "VDMX", "vhea",
  "vmtx", "BASE", "GDEF", "GPOS", "GSUB", "EBSC", "JSTF", "MATH",
  "CBDT", "CBLC", "COLR", "CPAL", "SVG ", "sbix", "acnt", "avar",
  "bdat", "bloc", "bsln", "cvar", "fdsc", "feat", "fmtx", "fvar",
  "gvar", "hsty", "just", "lcar", "mort", "morx", "opbd", "prop",
  "trak", "Zapf", "Silf", "Glat", "Gloc", "Feat", "Sill",
];

function readBase128(buf: Buffer, pos: number): { value: number; next: number } {
  let value = 0;
  for (let i = 0; i < 5; i++) {
    const b = buf[pos + i] as number;
    value = value * 128 + (b & 0x7f); // 乘法累加，避免 32-bit 位移在大值溢位
    if ((b & 0x80) === 0) return { value, next: pos + i + 1 };
  }
  throw new Error("UIntBase128 超過 5 bytes");
}

function parseWoff2Tables(path: string): Map<string, Buffer> {
  const file = readFileSync(path);
  expect(file.readUInt32BE(0)).toBe(0x774f4632); // 'wOF2'
  // 假設前提：單一 CFF 字型（OTTO）。'ttcf' 集合會在 directory 後多一段
  // collection directory、glyf/loca 變換也只存在 TrueType——都不在此解析範圍
  expect(file.readUInt32BE(4)).toBe(0x4f54544f); // 'OTTO'
  const numTables = file.readUInt16BE(12);
  const totalCompressedSize = file.readUInt32BE(20);
  let pos = 48;
  const entries: { tag: string; dataLength: number }[] = [];
  for (let i = 0; i < numTables; i++) {
    const flags = file[pos] as number;
    pos += 1;
    let tag: string;
    if ((flags & 0x3f) === 0x3f) {
      tag = file.toString("latin1", pos, pos + 4);
      pos += 4;
    } else {
      tag = KNOWN_TAGS[flags & 0x3f] as string;
    }
    const orig = readBase128(file, pos);
    pos = orig.next;
    // transform version（flags 高 2 bits）：glyf/loca 以 3 為「原樣」，其他表以 0 為「原樣」
    const version = (flags >> 6) & 0x3;
    const transformed = tag === "glyf" || tag === "loca" ? version !== 3 : version !== 0;
    let dataLength = orig.value;
    if (transformed) {
      const t = readBase128(file, pos);
      pos = t.next;
      dataLength = t.value;
    }
    entries.push({ tag, dataLength });
  }
  // 所有表資料依 directory 順序無填充串接後整段 brotli 壓縮
  const decompressed = brotliDecompressSync(file.subarray(pos, pos + totalCompressedSize));
  const tables = new Map<string, Buffer>();
  let offset = 0;
  for (const entry of entries) {
    tables.set(entry.tag, decompressed.subarray(offset, offset + entry.dataLength));
    offset += entry.dataLength;
  }
  return tables;
}

// cmap format 4（BMP 分段）：展開所有 segment 為 cp→glyph id
function parseFormat4(buf: Buffer, base: number, map: Map<number, number>): void {
  const segCount = buf.readUInt16BE(base + 6) / 2;
  const endBase = base + 14;
  const startBase = endBase + segCount * 2 + 2; // +2 跳過 reservedPad
  const deltaBase = startBase + segCount * 2;
  const rangeBase = deltaBase + segCount * 2;
  for (let seg = 0; seg < segCount; seg++) {
    const end = buf.readUInt16BE(endBase + seg * 2);
    const start = buf.readUInt16BE(startBase + seg * 2);
    const delta = buf.readInt16BE(deltaBase + seg * 2);
    const rangeOffset = buf.readUInt16BE(rangeBase + seg * 2);
    if (start === 0xffff) continue; // 終結 segment
    for (let cp = start; cp <= end; cp++) {
      let gid: number;
      if (rangeOffset === 0) {
        gid = (cp + delta) & 0xffff;
      } else {
        // idRangeOffset 是「自本 segment 的 idRangeOffset 欄位起算」的位移
        gid = buf.readUInt16BE(rangeBase + seg * 2 + rangeOffset + (cp - start) * 2);
        if (gid !== 0) gid = (gid + delta) & 0xffff;
      }
      if (gid !== 0) map.set(cp, gid);
    }
  }
}

// cmap format 12（完整平面連續群組）
function parseFormat12(buf: Buffer, base: number, map: Map<number, number>): void {
  const numGroups = buf.readUInt32BE(base + 12);
  for (let g = 0; g < numGroups; g++) {
    const p = base + 16 + g * 12;
    const start = buf.readUInt32BE(p);
    const end = buf.readUInt32BE(p + 4);
    const startGid = buf.readUInt32BE(p + 8);
    for (let cp = start; cp <= end; cp++) map.set(cp, startGid + (cp - start));
  }
}

function parseCmap(cmap: Buffer): Map<number, number> {
  const map = new Map<number, number>();
  const numTables = cmap.readUInt16BE(2);
  for (let i = 0; i < numTables; i++) {
    const offset = cmap.readUInt32BE(4 + i * 8 + 4);
    const format = cmap.readUInt16BE(offset);
    if (format === 4) parseFormat4(cmap, offset, map);
    else if (format === 12) parseFormat12(cmap, offset, map);
  }
  return map;
}

const FONT_PATH = fileURLToPath(new URL("../src/assets/ptt-square-mono.woff2", import.meta.url));
const tables = parseWoff2Tables(FONT_PATH);
const cmap = parseCmap(tables.get("cmap") as Buffer);
const unitsPerEm = (tables.get("head") as Buffer).readUInt16BE(18);
const numberOfHMetrics = (tables.get("hhea") as Buffer).readUInt16BE(34);

function advanceOf(gid: number): number {
  // hmtx 前 numberOfHMetrics 筆為 (advance, lsb) 各 2 bytes，其後字形共用末筆 advance
  return (tables.get("hmtx") as Buffer).readUInt16BE(Math.min(gid, numberOfHMetrics - 1) * 4);
}

// 與 wcwidth-data.ts 產生法同源：枚舉 WHATWG big5 雙位元組空間，取出 PTT 送得出的
// 圖形符號（框線/方塊/幾何形狀，落在 U+2500–25FF）
function big5GraphicCps(): Set<number> {
  const decoder = new TextDecoder("big5");
  const found = new Set<number>();
  const bytes = new Uint8Array(2);
  const trailRanges: readonly (readonly [number, number])[] = [
    [0x40, 0x7e],
    [0xa1, 0xfe],
  ];
  for (let lead = 0x81; lead <= 0xfe; lead++) {
    for (const [lo, hi] of trailRanges) {
      for (let trail = lo; trail <= hi; trail++) {
        bytes[0] = lead;
        bytes[1] = trail;
        const text = decoder.decode(bytes);
        const cp = text.codePointAt(0) as number;
        if (text.length === 1 && cp >= 0x2500 && cp <= 0x25ff) found.add(cp);
      }
    }
  }
  return found;
}

// 枚舉 WHATWG big5 雙位元組空間可解出的**全部**字（不限圖形符號）。
// build-font.py 的子集化守門用：這是 PTT 送得出的字集下界，一個都不能少
function big5AllCps(): Set<number> {
  const decoder = new TextDecoder("big5");
  const found = new Set<number>();
  const bytes = new Uint8Array(2);
  for (let lead = 0x81; lead <= 0xfe; lead++) {
    for (let trail = 0x40; trail <= 0xfe; trail++) {
      bytes[0] = lead;
      bytes[1] = trail;
      const text = decoder.decode(bytes);
      if ([...text].length !== 1) continue; // 解碼失敗會得到 U+FFFD 以外的多字元結果
      const cp = text.codePointAt(0) as number;
      if (cp !== 0xfffd) found.add(cp);
    }
  }
  return found;
}

function missingOrNarrow(cps: Iterable<number>): { missing: string[]; narrow: string[] } {
  const missing: string[] = [];
  const narrow: string[] = [];
  for (const cp of cps) {
    const gid = cmap.get(cp);
    const label = `U+${cp.toString(16).toUpperCase()}`;
    if (gid === undefined) missing.push(label);
    else if (advanceOf(gid) !== unitsPerEm) narrow.push(label);
  }
  return { missing, narrow };
}

describe("font-coverage：PTT Square Mono 字形守門", () => {
  it("WOFF2 解析出必要表，度量與 build-font.py 一致", () => {
    for (const tag of ["cmap", "head", "hhea", "hmtx", "maxp"]) {
      expect(tables.has(tag), `缺 ${tag} 表`).toBe(true);
    }
    expect(unitsPerEm).toBe(1000);
    expect(cmap.size).toBeGreaterThan(1000);
  });

  it("解析自我驗證：全形「中」佔 1em、半形 A 佔 0.5em", () => {
    // 已知樣本交叉驗證 cmap/hmtx 解析正確，排除「解析壞掉導致全數誤判通過」
    expect(advanceOf(cmap.get(0x4e2d) as number)).toBe(unitsPerEm);
    expect(advanceOf(cmap.get(0x41) as number)).toBe(unitsPerEm / 2);
  });

  it("U+2500–259F 框線與方塊全數有字形且為全形（customGlyphs 停用的前提）", () => {
    const range: number[] = [];
    for (let cp = 0x2500; cp <= 0x259f; cp++) range.push(cp);
    expect(missingOrNarrow(range)).toEqual({ missing: [], narrow: [] });
  });

  it("Big5 圖形符號域（PTT 送得出的 U+25xx，含 ◢◣◤◥ 三角）全數有字形且為全形", () => {
    const cps = big5GraphicCps();
    // 枚舉自我驗證：Big5 圖形區必含分隔線與實心方塊，防止列舉空轉讓斷言形同虛設
    expect(cps.has(0x2500)).toBe(true);
    expect(cps.has(0x2588)).toBe(true);
    expect(cps.has(0x25e2)).toBe(true);
    expect(missingOrNarrow(cps)).toEqual({ missing: [], narrow: [] });
  });

  // 以下兩項守 build-font.py 的子集化（砍韓文諺文）：字型是二進位產物，
  // 腳本改壞不會有任何編譯期訊號，只能靠這裡擋
  it("子集化未誤砍：Big5 可解出的字全數保留（來源字型本就沒有的除外）", () => {
    const cps = big5AllCps();
    // 枚舉自我驗證：Big5 至少有一萬多字，防止枚舉空轉讓斷言形同虛設
    expect(cps.size).toBeGreaterThan(13000);
    // 兩類排除項都**不是**子集化造成的——以子集化前的字型逐一比對過，
    // 前後缺的是同一批 6220 個，子集化誤砍數為 0：
    // 1. Big5 造字區映射到 BMP PUA，標準字型不含這些字形（6217 個）
    // 2. 來源 Noto Sans Mono CJK TC 本身就沒有的三個字
    const KNOWN_ABSENT = new Set([0x7f, 0x02cd, 0x2252]);
    const missing = [...cps].filter(
      (cp) => !cmap.has(cp) && !(cp >= 0xe000 && cp <= 0xf8ff) && !KNOWN_ABSENT.has(cp),
    );
    expect(missing).toEqual([]);
    // 排除範圍不得無限擴張：漢字主體（U+4E00–9FFF）一個都不能少
    const han = [...cps].filter((cp) => cp >= 0x4e00 && cp <= 0x9fff);
    expect(han.length).toBeGreaterThan(13000);
    expect(han.filter((cp) => !cmap.has(cp))).toEqual([]);
  });

  it("子集化已生效：韓文諺文不在 cmap 內", () => {
    // 諺文不在 Big5／UAO 映射域，PTT 送不出；砍掉省 1.5 MB（build-font.py）。
    // 這條同時防止「腳本被改回不子集化」而悄悄長回 10.9 MB
    const hangul: number[] = [];
    for (let cp = 0xac00; cp <= 0xd7af; cp++) if (cmap.has(cp)) hangul.push(cp);
    expect(hangul).toEqual([]);
  });
});
