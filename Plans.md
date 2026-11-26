# Plans — ptt-desktop

## 狀態標記

| 標記 | 意義 |
|---|---|
| `cc:TODO` | 待處理 |
| `cc:DOING` | 進行中（同時最多一個） |
| `cc:BLOCKED` | 卡住，待使用者裁決（附原因） |
| `cc:DONE` | 已完成 |

規則：標記由 `/harness-work` 與 `/harness-plan sync` 更動；`cc:DOING` 同時最多存在一個。
已完成的任務在下次 `/harness-plan create` 時刪除，紀錄留在 git 歷史。

**待真人驗收的項目一律以 `docs/e2e-checklist.md` 的未打勾項為準**，Plans.md 不重複記帳。

## 等上游

- **gpui 升版**：GPU 的兩個問題（輸入後每幀重畫、Wayland 閒置空 commit）已在 `vendor/gpui`
  修正。gpui 在 crates.io 何時發新版沒有時程，而且升版是一次移植
  （平台層已拆成多個 crate），不是換版本號；升版後要重跑 `docs/e2e-checklist.md` 的 Wayland
  閒置與出幀兩項。細節見 [`docs/comparison.md`](docs/comparison.md#gpu-問題的上游狀態)。

## 任務列表

<!-- /harness-plan 產生的任務由此往下 append。格式範例：

### P1 — 範例任務標題  `cc:TODO`

**目標：** 一句話說明完成後使用者看得到什麼

**變更範圍：**
- [ ] src/xxx.rs — 做什麼、補什麼測試

**DoD（完成定義）：**
- [ ] cargo fmt --check 通過
- [ ] cargo clippy --all-targets -- -D warnings 通過
- [ ] cargo test 通過
- [ ] 手動驗證：docs/e2e-checklist.md 第 X 項（涉及 UI/連線行為時）

**風險：** 無 / 說明（涉及有狀態模組、依賴變更時必填）

---
-->

目前無進行中任務。新任務編號從 P1 開始。
