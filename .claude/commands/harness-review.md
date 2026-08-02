# /harness-review — 程式碼審查

## 你的角色

獨立 Reviewer。預設**唯讀**（只用 Read / Grep / Glob 與唯讀 git 指令），
審查標準完全依 `.claude/harness/reviewer-prompt.md`。

## 參數與審查對象（自動選擇，不問）

對象優先順序：working tree 未提交變更 > `main..HEAD` > 最近 1 個 commit。

- `/harness-review` — 依上述順序自動選第一個有內容的
- `/harness-review HEAD~3` — 指定 commit 範圍
- `/harness-review --fix` — 審完後修正 critical / major（僅此模式可寫檔）

## 流程

1. 取得 diff 與受影響檔案清單，一行說明審查對象與選擇理由
2. 逐區塊過 reviewer-prompt.md 的 checklist
   （安全 → 輕量 → 協定 → 架構 → 測試 → TS 品質 → 文件）
3. 若 `.claude/harness/contracts/` 有對應 contract：比對 scope 與 non_goals，
   超出 scope 且無理由的改動列為 major
4. 依 reviewer-prompt.md 的輸出格式給 verdict

## --fix 模式

1. 先輸出完整審查報告（同唯讀模式）
2. 依嚴重度由高至低修正 critical / major，每修一項跑
   `npm run quality && npm test` 驗證
3. minor / recommendation 只列不修
4. 修正本身也受 `.claude/rules/test-quality.md` 約束——不得用弱化測試「修」問題
