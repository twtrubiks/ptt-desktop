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

## 等上游（我們這邊沒得改）

- **GPU 用量**：等 gpui 在 crates.io 發新版（目前 0.2.2），發版後升級並重量一次。
  真的等不及時，最省事的過渡做法是只套上游 #45369 那個 patch，但這要由使用者決定。
  細節見 [`docs/comparison.md`](docs/comparison.md#gpu-問題的上游狀態)。

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
