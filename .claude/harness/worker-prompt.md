# Worker 實作規範 — ptt-desktop

實作任務時遵循本文件。專案首要目標是「**輕量、快**」，所有取捨以此為準。

## 模組紀律

- 業務邏輯只寫在 `src/*.ts`；每個模組單一職責（現有模組表見 `docs/architecture.md`）
- `src-tauri/` 只有 Tauri 樣板 + plugin 註冊。**禁止在 Rust 側加業務邏輯**；
  若功能必須進 Rust（如系統 API），先停下來說明理由——這改變架構前提，
  屬 `.claude/rules/confirmation-scope.md` 第 3 類
- 模組間的組裝只發生在 `main.ts`；模組彼此不直接依賴（型別 import 除外）
- 新增/刪除模組時同步 `docs/architecture.md` 的模組表

## 輕量原則

- 禁止引入前端框架（React / Vue / Svelte / jQuery 皆否）
- 不新增 npm / cargo 依賴。真的需要時：在回報的 `deps_added` 欄寫明
  「為何無法用現有依賴或原生 API 實作」，交 reviewer 裁決
- timer / requestAnimationFrame / 事件監聽必須有明確停止條件與釋放路徑
  （參考 `blink-overlay.ts`：畫面無閃爍字或視窗不可見時 timer 完全靜止）
- 不做推測性抽象；三行重複好過一個只用一次的 helper

## 有狀態模組（改動必附測試）

清單以 `docs/architecture.md` 模組表標「（有狀態）」者為準，目前是：

- `telnet.ts` — IAC 序列可能跨 frame，狀態機不可假設一個 frame 內收完；
  HTTP 前導只在連線最前出現一次
- `codec.ts` — big5 → utf-8 **只切一次**，切換點由 auto-login 在位元組層偵測驅動
  （新舊 PTT 時機不同，見下節）；TextDecoder 用 stream 模式，多位元組字元可能跨 chunk
- `auto-login.ts` — 狀態機任一步異常（密碼錯、逾時）立即交還手動，**絕不自動重試**
- `screen-grid.ts` — shadow 必須跟上 xterm 的捲動語意（末列 LF 上捲、`ESC M` 下捲），
  錯列就會把捲走的半字補到新畫面（P12／P13 兩次殘字事故）；沒實作的序列一律記進
  `unknownSequences` 讓它亮紅燈，不要靜默忽略
- `sgr-reorder.ts` / `duotone.ts` — 同屬下行 byte 流管線，SGR／APC 區段可能跨 frame

改這些模組必須在 `tests/` 補對應 edge case 測試（跨 frame、跨 chunk、捲動、異常路徑），
並跑 `npm run replay:check`（見 Preflight）。

**協定規格不能從單一錄檔歸納**：P13 的教訓是往下捲頁（末列 LF）修好後，往上捲頁走的是
另一條文法（`ESC[24;1H ESC[J ESC[H ESC M`），第二份錄檔立刻踩同一個坑。盤點序列時
至少跨兩份錄檔，並想清楚反方向／對稱操作走的是不是同一條路。

### 編碼切換時機（新舊 PTT 雙軌，2026-08 起）

規格出處是 `docs/architecture.md`「編碼」段，改 `codec.ts` / `auto-login.ts` 前先讀：

- **新版**（2026-08-09 實錄）：伺服器收到「id,」**立即**切 UTF-8，密碼提示已是 UTF-8
  → auto-login 在位元組層偵測 UTF-8「請輸入您的密碼」，搶在該 chunk 解碼前切
- **舊版**：密碼提示仍是 Big5，送出密碼後的輸出才是 UTF-8 → 送出密碼後才切
- 兩條路共用 `utf8Switched` 旗標**保證只切一次**——重切會 flush 掉跨 chunk 的半個字
- `main.ts` 必須先 `feedBytes` 再解碼，callback 才來得及在亂碼上畫面前把 codec 撥正
- 「密碼不對」撥回 big5 只在 **Big5 位元組**命中（舊版）時觸發；新版以 UTF-8 文字
  命中，維持 utf-8 不撥回

## 渲染與啟動時序（單元測試碰不到的一類）

xterm 的格寬量測與字形圖集是**一次性快取**：`CharSizeService` 只監聽 `fontFamily` /
`fontSize`，圖集只在字型或字級變動時重建。凡是「事後才改變量測結果或圖集狀態」的
東西——字型晚一步載入、renderer 換掉、`init()` 順序調動、長時間使用累積——
`npm test` 一定全綠、實機才壞（`aa56257` 字型 race、`11ef6d8` 圖集損壞都是）。

∴ 動 `terminal.ts` / 字型 / renderer / `main.ts` 的 `init()` 順序時，**測試綠燈不構成
證據**：必須在 `docs/e2e-checklist.md` 加一項並寫明**觸發條件**（冷啟動？連續用多久？
什麼字級？），再於回報的 `manual_e2e` 欄指出來。想不出怎麼觸發，就是還沒懂根因。

## 安全 Invariant（不可協商）

1. 自動登入**絕不自動重試**——任何形式的 retry 都可能鎖帳號
2. 帳號密碼絕不出現在：log、錯誤訊息、測試 fixture、repo 任何檔案
3. 絕不讀取 `~/.config/ptt-desktop/`（真實帳密）。範例與測試一律用
   `config.example.json` 的假資料

## TypeScript

- strict 模式；不新增 `any`（不得不用時附一行理由註解）
- Tauri 2 / xterm.js 6 的 API 用法不確定時查官方文件，不憑記憶猜

## Preflight（自我點檢，全綠才算完成）

```bash
npm run quality   # format:check + oxlint + stylelint + cargo fmt check + clippy
npm run build     # tsc + vite build
npm test          # vitest run
```

失敗只能修實作或補測試，禁止弱化測試（`.claude/rules/test-quality.md`）。

**改到下行管線的有狀態模組**（`screen-grid.ts` / `sgr-reorder.ts` / `codec.ts` /
`duotone.ts`）時追加：

```bash
npm run replay:check -- recordings/rec-*.jsonl   # 走真管線重播本機錄檔
```

判準（四條，含兩個會假綠燈的指標）寫在 `scripts/replay-check.mjs` 檔頭，直接讀那裡，
本文件不重述。錄檔含帳號等個資、不進 repo（`recordings/` 已 gitignore），CI 也跑不到
這步——本機沒有錄檔就在回報寫明「無錄檔可驗」，不要當作已驗。

## 回報格式

```
status: done | blocked
files_changed: [...]
summary: 1-3 行
preflight: quality=PASS build=PASS test=PASS
           replay=PASS（錄檔數）| 無錄檔可驗 | N/A（未改下行管線模組）
deps_added: 無 | 套件名 + 無法用原生 API 的理由
manual_e2e: docs/e2e-checklist.md 對應項目，或「無 UI/連線行為變更」
```
