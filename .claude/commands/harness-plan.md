# /harness-plan — 任務規劃

## 你的角色

Planner。把需求整理成 Plans.md 的任務區塊。只寫 Plans.md，不動程式碼。

## 子命令

- `/harness-plan create <需求描述>` — 新需求梳理成一或多個任務
- `/harness-plan add <一句話>` — 快速補一個小任務
- `/harness-plan sync` — 比對 git log 與 Plans.md 標記，找出漏標/錯標

## create 流程

0. **清理**：先把 Plans.md 裡 `cc:DONE` 的任務區塊刪掉（紀錄在 git 歷史，不另外封存）。
   有 `cc:DOING` / `cc:BLOCKED` 時不清，先收尾
1. 需求不明處最多問 3 個問題（一次問完）；能自行判斷的自己判斷
2. 讀相關 `src/` 模組與 `docs/architecture.md`，確認影響範圍與現況
3. 產出任務區塊（格式見下），編號接續 Plans.md 記載的下一個編號
4. 大需求拆成可獨立驗收的小任務，並標注依賴關係（`依賴：P{M}`）

## 任務區塊格式

```markdown
### P{N} — {標題}  `cc:TODO`

**目標：** 一句話說清楚做完後使用者看得到什麼

**變更範圍：**
- [ ] src/xxx.rs — 做什麼、補什麼測試

**DoD（完成定義）：**
- [ ] cargo fmt --check 通過
- [ ] cargo clippy --all-targets -- -D warnings 通過
- [ ] cargo test 通過
- [ ] 手動驗證：docs/e2e-checklist.md 第 X 項（涉及 UI/連線行為時）

**風險：** 無 / 說明（涉及有狀態模組、依賴變更時必填；有狀態模組清單見
`docs/architecture.md` 模組表標「（有狀態）」者）

---
```

## sync 流程

1. `git log --oneline -20` 與 Plans.md 標記對照
2. 已實作但仍標 TODO/DOING，且 commit 能明確對應（訊息含 P{N} 或內容吻合）
   → 直接改 `cc:DONE` 並回報對應 commit
3. 無法確定對應的 → 列出差異，請使用者裁決（不擅自改）
4. 輸出同步報告：改了什麼、待裁決什麼
