# Harness 使用說明

本專案的 AI 開發工作流（harness）：讓 Claude Code 依「規劃 → 實作 → 審查」的固定流程工作，
你只需要下需求和做最終判斷。架構參考 [claude-code-harness](https://github.com/Chachamaru127/claude-code-harness)
的概念，針對本專案（Tauri 2 + xterm.js、輕量優先）客製。

## 日常流程

```
你：/harness-plan create <需求>     →  Claude 調查程式碼、把需求拆成 Plans.md 任務
你：看一眼 Plans.md，不滿意就叫它改
你：/harness-work                   →  Claude 實作 → 跑 preflight → spawn 獨立 reviewer 審查
                                        → 修到 APPROVE → commit → 列出手動驗證清單
你：照清單手動驗證（docs/e2e-checklist.md），滿意後自己 git push
```

## 啟動方式

| 情境 | 指令 |
|---|---|
| 日常互動開發 | `claude`（常用 git/npm 指令已在 allowlist，不太會被問） |
| 掛機無人值守（/goal） | `claude --permission-mode auto`（分類器幫忙擋清單外的危險操作） |

不要用 `--dangerously-skip-permissions`：deny/ask/hooks 雖仍有效，
但清單外的危險操作會全部無條件放行，沒有任何兜底。

## 三個指令

### `/harness-plan` — 規劃

| 用法 | 說明 |
|---|---|
| `/harness-plan create 加上巷口留言板功能` | 需求梳理成任務（最多問你 3 個問題） |
| `/harness-plan add 修正 F11 全螢幕在 KDE 下的閃爍` | 快速補一個小任務 |
| `/harness-plan sync` | 比對 git log 與 Plans.md，找出漏標/錯標 |

### `/harness-work` — 實作

| 用法 | 說明 |
|---|---|
| `/harness-work` | 做 Plans.md 第一個 `cc:TODO` |
| `/harness-work P3` | 做指定任務 |
| `/harness-work all` | 依序做完所有 TODO（每個都走完整流程） |

每個任務的固定流程：標記 `cc:DOING` → 產生 sprint contract（圈定可改的檔案）→ 實作
→ preflight（`npm run quality && npm run build && npm test` 全綠）→ 唯讀 reviewer 審查
→ REQUEST_CHANGES 就修（最多 3 輪，仍不過就標 `cc:BLOCKED` 問你）→ commit（不 push）
→ 標 `cc:DONE` + 列手動驗證清單。

### `/harness-review` — 獨立審查

| 用法 | 說明 |
|---|---|
| `/harness-review` | 唯讀審查（自動選：未提交變更 > main..HEAD > 最近 commit） |
| `/harness-review HEAD~3` | 審指定範圍 |
| `/harness-review --fix` | 審完自動修 critical/major |

審查標準見 `.claude/harness/reviewer-prompt.md`：安全（帳密、自動登入不重試）、
輕量目標（新依賴必審）、telnet/編碼協定正確性、架構紀律、測試品質、TS 品質、文件同步。

## Plans.md 狀態標記

| 標記 | 意義 | 誰改它 |
|---|---|---|
| `cc:TODO` | 待處理 | /harness-plan 建立 |
| `cc:DOING` | 進行中（同時最多一個） | /harness-work 開工時 |
| `cc:BLOCKED` | 卡住待你裁決（附原因） | review 3 輪不過或需求矛盾時 |
| `cc:DONE` | 已完成 | /harness-work 收尾時 |

看到 `cc:BLOCKED`：讀附註的爭點，直接用一句話告訴 Claude 你的裁決即可繼續。

## 自動防護（你不用做任何事）

| 防護 | 行為 |
|---|---|
| 真實帳密 | `~/.config/ptt-desktop/` 讀寫全 deny，密碼不會進到 AI 的對話 context |
| 危險寫入 | hook 擋 `.env`、`.git/`、`node_modules/`、建置產物、疑似 secret 的內容 |
| 不可逆操作 | `git push` / `reset` / `rebase` 會先問你；force push 直接 deny |
| CI 保護 | `.github/workflows/` 對 AI 唯讀 |
| 自我保護 | AI 不能改 `.claude/settings*`（不能解開綁自己的鎖） |
| 測試品質 | 加測試自由；刪測試/弱化斷言 = reviewer 直接退件 |
| 收尾提醒 | session 結束或 context 壓縮前，若有 `cc:DOING` 殘留會警告 |

## 搭配 /goal 無人值守

```
/goal Plans.md 的 P3 到 P5 全部 cc:DONE，且 npm run quality、npm run build、npm test 全綠，or stop after 40 turns
```

搭配 `/harness-work all` 使用。條件要寫「可機器驗證」的（測試全綠），
不要寫「功能做好」這種模糊條件。何時會停下來問你，由
`.claude/rules/confirmation-scope.md` 規範：只有外部送信、安全風險、需求達不成三種情況。

## 常見問題

**Q：它一直問我可不可以跑指令？**
常用唯讀 git、npm quality/build/test 已在 allowlist。若還有高頻誤問，把該指令加進
`.claude/settings.json` 的 `allow`（這個檔 AI 不能改，要你手動編輯）。

**Q：想跳過 review 快速改個小東西？**
直接用一般對話叫 Claude 改即可，harness 指令不是強制的。但 commit 前跑一次
`/harness-review` 花不了多少時間。

**Q：session 結束時跳出 cc:DOING 警告？**
上次任務沒收尾。開新 session 跑 `/harness-work`（會偵測殘留並續作），
或手動把該任務改回 `cc:TODO`。

**Q：reviewer 擋我加依賴，但我真的想加？**
在對話裡明說「我裁決：加這個依賴」即可——人的決定優先於 reviewer。
CLAUDE.md 的 invariant 是給 AI 的紀律，不是給你的。

## 檔案結構

```
CLAUDE.md                        # AI 的專案說明書（invariant、DoD、慣例）
Plans.md                         # 任務 SSOT
docs/harness-guide.md            # 本文件（給人看的使用說明）
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

- `Read` deny 擋不住 Bash `cat`（理論繞道）；靠 CLAUDE.md invariant 多層防禦，非絕對保證
- `settings.json` 改動要新開 session 才生效
- E2E 是手動的：桌面 app 無法用瀏覽器自動化，`/harness-work` 只負責列出待驗證清單
