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

- `telnet.ts` — IAC 序列可能跨 frame，狀態機不可假設一個 frame 內收完；
  HTTP 前導只在連線最前出現一次
- `codec.ts` — big5 → utf-8 的切換時機是「送出密碼後」；TextDecoder 用
  stream 模式，多位元組字元可能跨 chunk
- `auto-login.ts` — 狀態機任一步異常（密碼錯、逾時）立即交還手動，**絕不自動重試**

改這三個模組必須在 `tests/` 補對應 edge case 測試（跨 frame、跨 chunk、異常路徑）。

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

## 回報格式

```
status: done | blocked
files_changed: [...]
summary: 1-3 行
preflight: quality=PASS build=PASS test=PASS
deps_added: 無 | 套件名 + 無法用原生 API 的理由
manual_e2e: docs/e2e-checklist.md 對應項目，或「無 UI/連線行為變更」
```
