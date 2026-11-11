# CLAUDE.md

## Project Overview

PTT 桌面客戶端（GPUI 全 Rust）。固定 80×24 BBS 終端畫面，經官方 WebSocket 端點 `wss://ws.ptt.cc/bbs` 連線。
目前只支援 Linux。舊的 Tauri 2 + xterm.js 版在 `tauri` 分支，不在這裡維護。

**首要設計目標：輕量、快。** 單一執行檔、無 WebView、不做多餘的事。所有變更以此為最高取捨標準。

## Tech Stack

- Rust 2021 + gpui 0.2（版本釘住，升版是獨立任務）
- 連線：tungstenite + native-tls（ws.ptt.cc 不支援 TLS 1.3，不能換 rustls）
- 測試：`cargo test`（單元測試寫在各模組內，fixture 在 `tests/fixtures/`）；E2E 為手動清單 `docs/e2e-checklist.md`

## Architecture

`src/` 每檔一模組、單一職責，串接只在 `view.rs`。模組表、資料流、協定與各行為的理由見
`docs/architecture.md`（改架構必同步此檔）。

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

使用說明：`docs/harness-guide.md`

- 任務 SSOT：`Plans.md`，標記 `cc:TODO / cc:DOING / cc:BLOCKED / cc:DONE`
- 自主執行時何時才停下來問人：`.claude/rules/confirmation-scope.md`
- 測試品質（防假成功）：`.claude/rules/test-quality.md`

## Common Commands

```bash
cargo run                       # 開發模式（讀真實設定檔並連線，實機驗證由使用者做）
cargo test                      # 單元測試
cargo fmt --check
cargo clippy --all-targets -- -D warnings
scripts/build-appimage.sh       # 本機打包 AppImage（用法見檔頭；推 v* tag 時 release.yml 也用它）
```

## DoD（與 .github/workflows/ci.yml 一致）

`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

## 專案級 Invariant（不可協商）

1. 自動登入**絕不自動重試**（防鎖帳號）；重連不繞過 30 秒冷卻
2. 帳密絕不進 log、錯誤訊息、測試 fixture、repo
3. 絕不讀取 `~/.config/ptt-desktop/`（真實帳密，settings 已 deny），也不自行啟動 app 用真實帳號連線；
   範例與測試一律用 `config.example.json` 的假資料
4. 不新增依賴，除非 reviewer 通過充分理由

## Conventions

- 註解、文件、commit message 使用繁體中文；commit 遵循 Conventional Commits
- 文件只寫現況與一兩句理由，不記日期、任務編號與修正過程；實測數字放 `docs/comparison.md`
- 有狀態模組（`docs/architecture.md` 模組表標「（有狀態）」者）改動必附 edge case 測試
- gpui 0.2 的 API 不確定時讀 `~/.cargo/registry` 裡的原始碼，不憑記憶猜
