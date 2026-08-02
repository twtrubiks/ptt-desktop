# /harness-work — 實作 Plans.md 任務

## 你的角色

Lead。從 Plans.md 取出任務 → 實作 → 送審 → 提交，一次一個任務。
實作規範遵循 `.claude/harness/worker-prompt.md`（已由 CLAUDE.md import），
何時可以停下來問人遵循 `.claude/rules/confirmation-scope.md`。

## 參數

- `/harness-work` — 取 Plans.md 第一個 `cc:TODO`
- `/harness-work P14` — 指定任務
- `/harness-work all` — 依序處理所有 `cc:TODO`（每個任務完整走完 Step 2-8 才進下一個）

## Step 0 — 殘留檢查

`grep -n '^###.*cc:DOING' Plans.md`（只看任務標題行，狀態標記說明表不算）。若有殘留：

- 與本次要做的任務相同 → 視為續作，直接進 Step 3
- 不同 → 停下來問使用者（confirmation-scope 第 3 類：影響任務解釋）

## Step 1 — 選任務

讀 Plans.md 確認目標、變更範圍、DoD。
若任務描述與程式碼現況矛盾（如目標檔案已不存在）→ 停下來問（第 3 類）。

## Step 2 — 標記與 contract

1. 任務標記改 `cc:DOING`
2. 依 `.claude/harness/sprint-contract-template.json` 產生
   `.claude/harness/contracts/P{N}.sprint-contract.json`：
   填 scope（本次允許改動的檔案）、non_goals、
   manual_e2e_items（從 `docs/e2e-checklist.md` 挑出受本次變更影響的項目）

## Step 3 — 實作

依 worker-prompt.md 規範實作。
超出 contract scope 的檔案改動：先把檔案補進 contract 並留一行理由，再動手。

## Step 4 — Preflight

```bash
npm run quality && npm run build && npm test
```

全綠才能進 Step 5。失敗只能修實作或補測試，
禁止弱化測試（`.claude/rules/test-quality.md`）。

## Step 5 — Review 迴圈

用 Agent tool spawn 一個**唯讀** reviewer subagent，prompt 內容給它：

1. `.claude/harness/reviewer-prompt.md` 全文
2. 本次 sprint contract 內容
3. `git diff HEAD` 全文（含未 staged）

verdict 處理：

- `APPROVE` → Step 6
- `REQUEST_CHANGES` → 修正後**重新 spawn 新的 reviewer**（不沿用舊 context）再審
- 連續 3 次 `REQUEST_CHANGES` → 任務標 `cc:BLOCKED`（附爭點摘要），整理後問使用者

## Step 6 — 手動驗證清單

輸出 contract 的 manual_e2e_items：「本次需手動驗證」清單
（對應 `docs/e2e-checklist.md` 項目）。無 UI/連線行為變更則明說「無需手動驗證」。

## Step 7 — 提交

Conventional Commits + 繁體中文描述
（例：`fix: 修正 telnet IAC 序列跨 frame 時漏濾的問題`）。
只 commit，**不 push**——push 屬外部送信（confirmation-scope 第 1 類）。

## Step 8 — 收尾

1. Plans.md 該任務改 `cc:DONE`，變更範圍與 DoD checkbox 勾選
2. 行為變更同步 README.md / `docs/architecture.md` 相關段落
3. 回報：完成摘要 + preflight 結果 + 手動驗證清單（worker-prompt.md 的回報格式）
