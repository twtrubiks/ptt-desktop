# CLAUDE.md

## Project Overview

PTT 桌面客戶端（Tauri 2 + xterm.js）。固定 80×24 BBS 終端畫面，經官方 WebSocket 端點 `wss://ws.ptt.cc/bbs` 連線。

**首要設計目標：輕量、快。** 系統 WebView、純 TypeScript 無前端框架、不做多餘的事。所有變更以此為最高取捨標準。

## Tech Stack

- **Frontend（約 98% 程式碼，所有業務邏輯）**: TypeScript (strict) + xterm.js 6 + Vite 8
- **Rust 側**: Tauri 2 樣板 + plugin 註冊，不含業務邏輯
- **測試**: Vitest（單元測試）；E2E 為手動清單 `docs/e2e-checklist.md`
- **品質**: oxlint / prettier / stylelint / cargo fmt / clippy（`npm run quality` 一次跑完）

## Architecture

src/ 每檔一模組、單一職責，main.ts 負責組裝。模組表、資料流、telnet/編碼協定細節見 `docs/architecture.md`（改架構必同步此檔）。

實作規範：
@.claude/harness/worker-prompt.md
審查標準：
@.claude/harness/reviewer-prompt.md

## Harness Workflow

| 指令 | 用途 |
|---|---|
| `/harness-plan` | 需求 → Plans.md 任務（create / add / sync） |
| `/harness-work` | 實作 Plans.md 任務（含 review 迴圈與提交） |
| `/harness-review` | 獨立審查（預設唯讀，`--fix` 可修） |

使用說明（給人看的完整版）：`docs/harness-guide.md`

- 任務 SSOT：`Plans.md`，標記 `cc:TODO / cc:DOING / cc:BLOCKED / cc:DONE`
- 自主執行時何時才停下來問人：`.claude/rules/confirmation-scope.md`
- 測試品質（防假成功）：`.claude/rules/test-quality.md`

## Common Commands

```bash
npm run dev            # Vite dev server（純前端）
npm run tauri dev      # 完整桌面 app
npm run quality        # format:check + oxlint + stylelint + cargo fmt check + clippy
npm run build          # tsc + vite build
npm test               # vitest run
npm run replay:check -- recordings/rec-*.jsonl   # 對本機錄檔重播驗畫面（改 screen-grid 等模組後跑）
```

## DoD（與 .github/workflows/ci.yml 完全一致）

`npm run quality && npm run build && npm test`

## 專案級 Invariant（不可協商）

1. 自動登入**絕不自動重試**（防鎖帳號）
2. 帳密絕不進 log、錯誤訊息、測試 fixture、repo
3. 絕不讀取 `~/.config/ptt-desktop/`（真實帳密，settings 已 deny）；範例與測試一律用 `config.example.json` 的假資料
4. 不新增依賴、不引入前端框架，除非 reviewer 通過充分理由

## Conventions

- 註解、文件、commit message 使用繁體中文；commit 遵循 Conventional Commits
- 有狀態模組（`docs/architecture.md` 模組表標「（有狀態）」者：telnet / codec / auto-login / screen-grid / sgr-reorder / duotone）改動必附 edge case 測試；改下行管線那幾個另跑 `npm run replay:check`
- Tauri 2 / xterm.js 6 版本敏感 API 不確定時查官方文件，不憑記憶猜
