// 實錄回歸驗證：把 dev 錄檔（recordings/*.jsonl）走「真管線」重播，並以 xterm
// 語意的 80×24 模擬器重建使用者實際看到的畫面，檢查 shadow 有沒有跟畫面脫節。
//
//   npm run replay:check -- recordings/rec-*.jsonl [--dump]
//
// 改 screen-grid / sgr-reorder / codec / duotone 後跑一次。判定（任一不過即 exit 1）：
//   1. ScreenGrid 未實作的終端序列 = 0（2026-08-04 兩次殘字事故的共同機制）
//   2. 模擬器未實作的序列 = 0（模擬器自己漏掉時，它會與 shadow「一致地錯」，
//      單看兩模型是否一致會得到假綠燈——這條就是防這個）
//   3. 同一列連續分歧 < 10 幀。單幀不一致是補寫遞延的正常暫態（兩份實錄實測
//      最長 2 幀），錯列時則長期不收斂（停用 RI 重跑實測 92 幀）。
//      「最終畫面比對」不能當主判準——全螢幕重繪會讓兩模型收斂成假綠燈
//   4. 孤兒補寫沒有蓋到畫面上的既有內容
//
// 錄檔含帳號等個資，不進 repo（recordings/ 已在 .gitignore）；本工具進 repo。
import { readFileSync } from "node:fs";

const SRC = new URL("../src/", import.meta.url).pathname;
const { ScreenGrid } = await import(SRC + "screen-grid.ts");
const { SgrReorderFilter } = await import(SRC + "sgr-reorder.ts");
const { PttDecoder } = await import(SRC + "codec.ts");
const { DuotoneSplitFilter, buildOrphanRepair } = await import(SRC + "duotone.ts");
const { PttUnicodeProvider } = await import(SRC + "unicode-ptt.ts");

const ROWS = 24;
const COLS = 80;
const LEFT_BASE = 0xf0000;
const RIGHT_BASE = 0x100000;
const prov = new PttUnicodeProvider();

const cell = (attr = 0) => ({ ch: " ", w: 1, attr });
const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));

// xterm 語意的螢幕模型：寬字元原子（覆寫一半 → 另一半整格清空）、pending wrap、
// scrollback:0 的上捲／下捲。只實作 PTT 會送的子集，其餘記進 unknown 讓判定亮紅燈。
class Sim {
  constructor() {
    this.cells = Array.from({ length: ROWS }, () => Array.from({ length: COLS }, () => cell()));
    this.row = 0;
    this.col = 0;
    this.attr = 0;
    this.saved = null;
    this.st = "text";
    this.params = "";
    this.strEsc = false;
    this.unknown = new Set();
    this.scrolls = 0;
  }

  write(text) {
    for (const ch of text) this.step(ch);
  }

  step(ch) {
    const cp = ch.codePointAt(0);
    if (this.st === "str") {
      // APC/OSC/DCS 字串序列（duotone 的 marker 走這條）：吞到 ST
      if (this.strEsc) {
        this.strEsc = false;
        if (ch === "\\") this.st = "text";
        return;
      }
      if (ch === "\x1b") this.strEsc = true;
      if (cp === 7) this.st = "text";
      return;
    }
    if (this.st === "esc") {
      this.st = "text";
      if (ch === "[") {
        this.st = "csi";
        this.params = "";
      } else if (ch === "_" || ch === "]" || ch === "P" || ch === "^") {
        this.st = "str";
        this.strEsc = false;
      } else if (ch === "7") {
        this.saved = { row: this.row, col: this.col, attr: this.attr };
      } else if (ch === "8") {
        if (this.saved) {
          this.row = this.saved.row;
          this.col = Math.min(this.saved.col, COLS - 1); // restoreCursor 會夾掉 pending-wrap
          this.attr = this.saved.attr;
        }
      } else if (ch === "M") {
        this.ri();
      } else {
        this.unknown.add("ESC " + ch);
      }
      return;
    }
    if (this.st === "csi") {
      if ((cp >= 0x30 && cp <= 0x3f) || (cp >= 0x20 && cp <= 0x2f)) {
        this.params += ch;
        return;
      }
      this.st = "text";
      this.csi(ch);
      return;
    }
    if (ch === "\x1b") {
      this.st = "esc";
      return;
    }
    if (ch === "\r") {
      this.col = 0;
      return;
    }
    if (ch === "\n") {
      this.lf();
      return;
    }
    if (ch === "\b") {
      this.col = Math.max(0, Math.min(this.col, COLS - 1) - 1);
      return;
    }
    if (cp === 0 || cp === 7 || cp === 0x7f) return; // NUL/BEL/DEL：不影響畫面
    if (cp < 32) {
      this.unknown.add(`C0 0x${cp.toString(16).padStart(2, "0")}`);
      return;
    }
    this.put(ch, prov.wcwidth(cp));
  }

  lf() {
    if (this.row < ROWS - 1) {
      this.row++;
      return;
    }
    this.cells.shift();
    this.cells.push(Array.from({ length: COLS }, () => cell(this.attr)));
    this.scrolls++;
  }

  ri() {
    if (this.row > 0) {
      this.row--;
      return;
    }
    this.cells.pop();
    this.cells.unshift(Array.from({ length: COLS }, () => cell(this.attr)));
    this.scrolls++;
  }

  put(ch, w) {
    if (w === 0) return;
    if (this.col + w > COLS) {
      this.col = 0;
      this.lf();
    }
    const r = this.cells[this.row];
    this.breakPair(r, this.col);
    if (w === 2) this.breakPair(r, this.col + 1);
    r[this.col] = { ch, w, attr: this.attr };
    if (w === 2) r[this.col + 1] = { ch, w: 0, attr: this.attr };
    this.col = Math.min(this.col + w, COLS); // col===COLS 即 pending wrap
  }

  // 寫進寬字元的任一半 → xterm 把整格清空
  breakPair(r, c) {
    if (c >= COLS) return;
    if (r[c].w === 2 && c + 1 < COLS && r[c + 1].w === 0) r[c + 1] = cell(this.attr);
    if (r[c].w === 0 && c > 0 && r[c - 1].w === 2) r[c - 1] = cell(this.attr);
  }

  csi(final) {
    const raw = this.params;
    const p = (raw.startsWith("?") ? raw.slice(1) : raw).split(";").map((s) => parseInt(s, 10));
    const p0 = Number.isNaN(p[0]) ? 0 : p[0];
    const cur = Math.min(this.col, COLS - 1);
    switch (final) {
      case "H":
      case "f": {
        const c = (p.length > 1 && !Number.isNaN(p[1]) ? p[1] : 0) || 1;
        this.row = clamp((p0 || 1) - 1, 0, ROWS - 1);
        this.col = clamp(c - 1, 0, COLS - 1);
        return;
      }
      case "A":
        this.row = Math.max(0, this.row - (p0 || 1));
        return;
      case "B":
        this.row = Math.min(ROWS - 1, this.row + (p0 || 1));
        return;
      case "C":
        this.col = Math.min(COLS - 1, cur + (p0 || 1));
        return;
      case "D":
        this.col = Math.max(0, cur - (p0 || 1));
        return;
      case "J":
        if (p0 === 2 || p0 === 3) this.erase(0, 0, ROWS - 1, COLS - 1);
        else if (p0 === 1) this.erase(0, 0, this.row, cur);
        else this.erase(this.row, cur, ROWS - 1, COLS - 1);
        return;
      case "K":
        if (p0 === 2) this.erase(this.row, 0, this.row, COLS - 1);
        else if (p0 === 1) this.erase(this.row, 0, this.row, cur);
        else this.erase(this.row, cur, this.row, COLS - 1);
        return;
      case "m":
        this.sgr(raw);
        return;
      case "h":
      case "l":
      case "n":
      case "c":
        return; // 模式設定與查詢：不影響 buffer 內容
      default:
        this.unknown.add("CSI " + final);
        return;
    }
  }

  erase(r1, c1, r2, c2) {
    for (let r = r1; r <= r2; r++) {
      const from = r === r1 ? c1 : 0;
      const to = r === r2 ? c2 : COLS - 1;
      if (from > 0) this.breakPair(this.cells[r], from);
      if (to < COLS - 1) this.breakPair(this.cells[r], to);
      for (let c = from; c <= to; c++) this.cells[r][c] = cell(this.attr);
    }
  }

  sgr(raw) {
    for (const s of (raw === "" ? [""] : raw.split(";"))) {
      const n = s === "" ? 0 : parseInt(s, 10);
      if (Number.isNaN(n)) continue;
      if (n === 0) this.attr = 0;
      else if (n === 1) this.attr |= 1;
      else if (n === 22) this.attr &= ~1;
      else if (n === 5) this.attr |= 2;
      else if (n === 25) this.attr &= ~2;
      else if (n === 7) this.attr |= 4;
      else if (n === 27) this.attr &= ~4;
      else if (n === 8) this.attr |= 8;
      else if (n === 28) this.attr &= ~8;
      else if (n >= 30 && n <= 37) this.attr = (this.attr & ~(0xf << 4)) | ((n - 30 + 1) << 4);
      else if (n === 39) this.attr &= ~(0xf << 4);
      else if (n >= 40 && n <= 47) this.attr = (this.attr & ~(0xf << 8)) | ((n - 40 + 1) << 8);
      else if (n === 49) this.attr &= ~(0xf << 8);
    }
  }

  // 逐欄 token；PUA 半字還原成原字，相鄰「左半+同字右半」＝雙色字的正常樣態，
  // 併回一個寬字才能與 shadow 的 byte 網格比較
  tokens(r) {
    const out = [];
    for (let c = 0; c < COLS; c++) {
      const x = this.cells[r][c];
      if (x.w === 0) continue;
      const cp = x.ch.codePointAt(0);
      if (cp >= RIGHT_BASE) {
        const base = String.fromCodePoint(cp - RIGHT_BASE);
        if (out.length && out[out.length - 1] === `${c - 1}:${base}左`) {
          out[out.length - 1] = `${c - 1}:${base}`;
          continue;
        }
        out.push(`${c}:${base}右`);
      } else if (cp >= LEFT_BASE) {
        out.push(`${c}:${String.fromCodePoint(cp - LEFT_BASE)}左`);
      } else {
        out.push(`${c}:${x.ch}`);
      }
    }
    return out;
  }

  rowText(r) {
    return this.cells[r]
      .map((c) => {
        if (c.w === 0) return "";
        const cp = c.ch.codePointAt(0);
        if (cp >= RIGHT_BASE) return `⟨${String.fromCodePoint(cp - RIGHT_BASE)}右⟩`;
        if (cp >= LEFT_BASE) return `⟨${String.fromCodePoint(cp - LEFT_BASE)}左⟩`;
        return c.ch;
      })
      .join("");
  }
}

function gridTokens(grid, r) {
  const out = [];
  for (let c = 0; c < COLS; c++) {
    const x = grid.cellAt(r, c);
    if (x.half === 2 && x.paired) continue; // 右半，左半已輸出
    if (x.half === 1) out.push(`${c}:${x.glyph}${x.paired ? "" : "左"}`);
    else if (x.half === 2) out.push(`${c}:${x.glyph}右`);
    else if (x.byte >= 0) out.push(`${c}:b${x.byte.toString(16)}`);
    else out.push(`${c}:${x.glyph}`);
  }
  return out;
}

function replay(path) {
  const grid = new ScreenGrid();
  const reorder = new SgrReorderFilter();
  const codec = new PttDecoder();
  const duo = new DuotoneSplitFilter();
  const sim = new Sim();
  let repairCells = 0;
  const overwrites = []; // 補寫蓋到既有內容（畫面上該格不是空白）
  // 逐幀分歧：單幀不一致可能只是補寫遞延的暫態，「同一列連續多少幀不一致」才是
  // 使用者看得到的錯誤停留時間。最終畫面比對太弱——全螢幕重繪會讓兩模型收斂
  const streak = new Array(ROWS).fill(0);
  let maxStreak = 0;
  let worstRow = -1;

  for (const line of readFileSync(path, "utf8").trim().split("\n")) {
    const o = JSON.parse(line);
    if (o.ev) {
      if (o.ev === "utf8-switch") {
        grid.switchToUtf8();
        const residue = reorder.switchToUtf8();
        if (residue.length) sim.write(duo.process(codec.decode(residue)));
        const tail = codec.switchToUtf8();
        if (tail) sim.write(duo.process(tail));
      }
      continue;
    }
    const { output, repairs } = grid.feed(new Uint8Array(Buffer.from(o.d, "base64")));
    sim.write(duo.process(codec.decode(reorder.process(output))));
    if (repairs.length > 0) {
      repairCells += repairs.length;
      for (const r of repairs) {
        const c = sim.cells[r.row][r.col];
        if (c.ch !== " " || c.w !== 1) {
          overwrites.push(`seq=${o.seq} (r${r.row},c${r.col}) 補「${r.glyph}」蓋掉 ${c.ch}`);
        }
      }
      sim.write(buildOrphanRepair(repairs));
    }
    for (let r = 0; r < ROWS; r++) {
      if (gridTokens(grid, r).join(" ") !== sim.tokens(r).join(" ")) {
        if (++streak[r] > maxStreak) {
          maxStreak = streak[r];
          worstRow = r;
        }
      } else streak[r] = 0;
    }
  }

  const mismatched = [];
  for (let r = 0; r < ROWS; r++) {
    if (gridTokens(grid, r).join(" ") !== sim.tokens(r).join(" ")) mismatched.push(r);
  }
  return { grid, sim, repairCells, overwrites, mismatched, maxStreak, worstRow };
}

const SUSTAINED_LIMIT = 10; // 同一列連續分歧幀數上限（實測：正常 2、錯列 92）

const args = process.argv.slice(2);
const dump = args.includes("--dump");
const files = args.filter((a) => !a.startsWith("--"));
if (files.length === 0) {
  console.error("用法：npm run replay:check -- recordings/rec-*.jsonl [--dump]");
  process.exit(2);
}

let failed = false;
for (const file of files) {
  const { grid, sim, repairCells, overwrites, mismatched, maxStreak, worstRow } = replay(file);
  const unknownGrid = [...grid.unknownSequences];
  const unknownSim = [...sim.unknown];
  const bad =
    unknownGrid.length + unknownSim.length + overwrites.length + mismatched.length +
    (maxStreak >= SUSTAINED_LIMIT ? 1 : 0);
  failed ||= bad > 0;

  console.log(`\n=== ${file} ===`);
  console.log(`  捲動 ${sim.scrolls} 次 · 孤兒補寫 ${repairCells} 格`);
  console.log(`  shadow 未實作序列：${unknownGrid.join(" ") || "無"}`);
  console.log(`  模擬器未實作序列：${unknownSim.join(" ") || "無"}`);
  console.log(`  shadow ↔ 畫面最終不一致列數：${mismatched.length}${mismatched.length ? ` (${mismatched.join(",")})` : ""}`);
  console.log(
    `  同一列連續分歧最長：${maxStreak} 幀${worstRow >= 0 ? `（第 ${worstRow} 列）` : ""}` +
      `${maxStreak >= SUSTAINED_LIMIT ? ` ← 超過 ${SUSTAINED_LIMIT}，shadow 與畫面錯列` : ""}`,
  );
  console.log(`  補寫蓋掉既有內容：${overwrites.length}`);
  for (const o of overwrites.slice(0, 10)) console.log(`    ${o}`);
  if (dump) {
    console.log("  --- 最終畫面 ---");
    for (let r = 0; r < ROWS; r++) console.log(`  ${String(r).padStart(2)} |${sim.rowText(r)}|`);
  }
  console.log(bad ? "  ✗ FAIL" : "  ✓ PASS");
}
process.exit(failed ? 1 : 0);
