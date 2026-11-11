# Harness 使用說明

本專案的 AI 開發工作流：讓 Claude Code 依「規劃 → 實作 → 審查」的固定流程工作，
你只需要下需求和做最終判斷。概念參考 [claude-code-harness](https://github.com/Chachamaru127/claude-code-harness)。

## 日常流程

```
你：/harness-plan create <需求>     →  Claude 調查程式碼、把需求拆成 Plans.md 任務
你：看一眼 Plans.md，不滿意就叫它改
你：/harness-work                   →  實作 → preflight → 獨立 reviewer 審查 → 修到 APPROVE
                                        → commit（不 push）→ 列出手動驗證清單
你：照清單實機驗證（docs/e2e-checklist.md），滿意後自己 git push
```

`docs/e2e-checklist.md` 不進 git，它的未打勾項就是「還沒被真人驗過」的唯一真相。
實機驗證要用真實帳號，一律由你做，AI 不會自己啟動 app 連線。

## 三個指令

| 指令 | 說明 |
|---|---|
| `/harness-plan create <需求>` | 需求梳理成任務（最多問 3 個問題）；順手刪掉已完成的任務 |
| `/harness-plan add <一句話>` | 快速補一個小任務 |
| `/harness-plan sync` | 比對 git log 與 Plans.md，找出漏標／錯標 |
| `/harness-work [P3｜all]` | 做第一個 `cc:TODO`、指定任務，或全部依序做完 |
| `/harness-review [範圍] [--fix]` | 唯讀審查（自動選：未提交變更 > `main..HEAD` > 最近 commit）；`--fix` 會修 critical／major |

`/harness-work` 每個任務的流程：標 `cc:DOING` → 產生 sprint contract（圈定可改的檔案）→ 實作
→ preflight（`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`）
→ 唯讀 reviewer 審查，退件就修（最多 3 輪，仍不過標 `cc:BLOCKED` 問你）→ commit → 標 `cc:DONE`。

審查標準在 `.claude/harness/reviewer-prompt.md`：安全、輕量、協定與編碼、架構紀律、測試品質、
Rust 品質、文件同步。動到繪製或輸入卻沒有 e2e 項算 major，因為那類回歸單元測試必然全綠。

## Plans.md 狀態標記

| 標記 | 意義 | 誰改它 |
|---|---|---|
| `cc:TODO` | 待處理 | /harness-plan 建立 |
| `cc:DOING` | 進行中（同時最多一個） | /harness-work 開工時 |
| `cc:BLOCKED` | 卡住待你裁決（附原因） | review 3 輪不過或需求矛盾時 |
| `cc:DONE` | 已完成 | /harness-work 收尾時 |

看到 `cc:BLOCKED`：讀附註的爭點，用一句話告訴 Claude 你的裁決即可繼續。

## 自動防護

| 防護 | 行為 |
|---|---|
| 真實帳密 | `~/.config/ptt-desktop/` 讀寫全 deny |
| 危險寫入 | hook 擋 `.env`、`.git/`、建置產物、疑似 secret 的內容 |
| 不可逆操作 | `git push` / `reset` / `rebase` 會先問你；force push 直接 deny |
| CI 與自我保護 | `.github/workflows/`、`.claude/settings*`、hooks 對 AI 唯讀，要改請自己動手 |
| 測試品質 | 加測試自由；刪測試／弱化斷言 = reviewer 直接退件 |
| 收尾提醒 | session 結束或 context 壓縮前，若有 `cc:DOING` 殘留會警告 |

何時會停下來問你由 `.claude/rules/confirmation-scope.md` 規範：只有外部送信、安全風險、需求達不成三種情況。
人的決定優先於 reviewer：在對話裡明說「我裁決：…」即可。

## 檔案結構

```
CLAUDE.md                        # AI 的專案說明書（invariant、DoD、慣例）
Plans.md                         # 任務 SSOT
docs/harness-guide.md            # 本文件
.claude/
├── settings.json                # 權限 allow/ask/deny + hooks 配線（AI 不可改）
├── commands/harness-{plan,work,review}.md
├── rules/{confirmation-scope,test-quality}.md
└── harness/
    ├── {worker,reviewer}-prompt.md      # 角色規範與審查標準
    ├── sprint-contract-template.json
    ├── contracts/                       # 逐任務 contract（gitignore）
    └── hooks/{pre-write-check,check-doing}.sh
```

## 已知限制

- `Read` deny 擋不住 Bash `cat`；靠 CLAUDE.md invariant 多層防禦，非絕對保證
- `settings.json` 改動要新開 session 才生效
- E2E 是手動的，`/harness-work` 只負責列出待驗證清單
