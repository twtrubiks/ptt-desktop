# /harness-plan — 任務規劃

## 你的角色

Planner。把需求整理成 Plans.md 的任務區塊。只寫 Plans.md，不動程式碼。

## 子命令

- `/harness-plan create <需求描述>` — 新需求梳理成一或多個任務
- `/harness-plan add <一句話>` — 快速補一個小任務
- `/harness-plan sync` — 比對 git log 與 Plans.md 標記，找出漏標/錯標
- `/harness-plan archive` — 已完成任務搬進 `docs/plans-archive.md`，Plans.md 只留索引

## create 流程

0. **封存檢查**：Plans.md 超過 200 行、或 `cc:DONE` 累積 5 個以上時，先跑 archive 流程
   再開新任務（Plans.md 每次 `/harness-work`、`/harness-plan` 都會被完整讀進 context，
   已完成任務的全文讀了也用不到）。有 `cc:DOING` / `cc:BLOCKED` 時不封存，先收尾
1. 需求不明處最多問 3 個問題（一次問完）；能自行判斷的自己判斷
2. 讀相關 `src/` 模組與 `docs/architecture.md`，確認影響範圍與現況
3. 產出任務區塊（格式見下），編號接續現有最大 P{N}
4. 大需求拆成可獨立驗收的小任務，並標注依賴關係（`依賴：P{M}`）

## 任務區塊格式

```markdown
### P{N} — {標題}  `cc:TODO`

**目標：** 一句話說清楚做完後使用者看得到什麼

**變更範圍：**
- [ ] src/xxx.ts — 做什麼
- [ ] tests/xxx.test.ts — 補什麼測試

**DoD（完成定義）：**
- [ ] npm run quality 通過
- [ ] npm run build 通過
- [ ] npm test 通過
- [ ] npm run replay:check 通過（改 screen-grid / sgr-reorder / codec / duotone 時）
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

## archive 流程

只搬 `cc:DONE`。`cc:TODO` / `cc:DOING` / `cc:BLOCKED` 的任務區塊一律不動。

1. 確認無 `cc:DOING` / `cc:BLOCKED`——有就停下來，先收尾再封存
2. `cc:DONE` 任務區塊**原文**剪到 `docs/plans-archive.md` 末尾（含其上方的背景說明
   blockquote）。不改寫、不摘要：決策理由與實測數據靠全文留存，摘要留不住
3. 封存檔開頭更新封存日期、來源 commit、收錄任務清單。commit hash 先用
   `git merge-base --is-ancestor <hash> main` 確認還在 main 歷史上——被 squash 掉的舊
   hash 會被 gc 回收，引用等於留死連結
4. Plans.md「已封存任務索引」加列：`# | 標題 | 定案結論`（一句話）
5. Plans.md 開頭「最後封存」那行更新日期
6. 檢查被封存任務的 DoD 有沒有未打勾的 `- [ ]`（多半是待真人做的手動驗證）——
   有就在索引表下方明列，不可隨封存埋掉。**同時掃一次 `docs/e2e-checklist.md` 的
   未打勾項**：沒開任務直接修掉的 hotfix 只會記在那裡，任務 DoD 掃不到
7. 單獨 commit（`docs: 封存 Plans.md 已完成任務 P{N}–P{M}`），不與程式碼變更混在一起
   ——封存本身就是靠這個 commit 可回退，不另外留 `.bak` 檔
