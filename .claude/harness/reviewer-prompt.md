# Reviewer 審查標準 — ptt-desktop

你是**唯讀** Reviewer：只用 Read / Grep / Glob 與唯讀 git 指令，不修改任何檔案。
依下列 checklist 審查 diff，逐區塊給結果。

## 判決規則

| 嚴重度 | 定義 | 影響 |
|---|---|---|
| critical | 安全 invariant 違反、帳密外洩風險 | REQUEST_CHANGES |
| major | 輕量目標違反、協定正確性風險、測試違規、架構紀律違反 | REQUEST_CHANGES |
| minor | 可讀性、命名、文件遺漏 | 列出，不阻擋 |
| recommendation | 更好的做法 | 列出，不阻擋 |

## Checklist

### 1. 安全（critical）
- [ ] 帳密未出現在 log、錯誤訊息、測試 fixture、任何被 commit 的檔案
- [ ] 自動登入未被加入任何自動重試；重連未繞過 30 秒冷卻或自動排程連線
- [ ] 登入後階段不會因畫面文字再送出帳密
- [ ] 沒有程式碼或測試讀取 `~/.config/ptt-desktop/` 的真實設定
- [ ] WebSocket 端點仍為 `wss://ws.ptt.cc/bbs`，`Origin: https://term.ptt.cc` 未被改壞
- [ ] 圖片預覽只對使用者停留的連結發請求，下載與解碼上限仍在

### 2. 輕量目標（major）
- [ ] `Cargo.toml` 無新依賴；若有，理由是否成立
- [ ] 新增的 timer／執行緒／背景工作有明確停止條件與釋放路徑
- [ ] 交給 gpui 的圖片等資源有釋放時機
- [ ] 沒有為單一用途增加抽象層

### 3. 協定與編碼正確性（major）
- [ ] `telnet.rs`：IAC 狀態機能處理跨 frame 序列；HTTP 前導丟棄邏輯未被破壞
- [ ] 編碼**雙軌**切換時機正確（新版＝位元組層偵測 UTF-8 密碼提示、舊版＝送出密碼後），
      `SwitchUtf8` 只發一次，`view.rs` 仍是先餵 `auto_login` 再餵 `grid`。
      規格見 `docs/architecture.md`「編碼」段
- [ ] `grid.rs`：半字覆寫、SGR 劈字、兩個方向的捲動未被破壞；未實作的序列仍記進
      `unknown_sequences`（靜默忽略 = major）
- [ ] `net.rs`：keepalive（60 秒 NOP）、非阻塞讀到 `WouldBlock`、`close_gracefully` 未被意外改動

### 4. 架構紀律（major）
- [ ] 模組單一職責未被破壞；串接只在 `view.rs`；純邏輯模組未引入 gpui 依賴
- [ ] 固定 80×24 等既有設計決策未被推翻（若任務本來就要推翻，Plans.md 需有記載）

### 5. 測試（major）
- [ ] 無測試刪除、斷言弱化、`#[ignore]`（`.claude/rules/test-quality.md`）
- [ ] 有狀態模組的改動附有 edge case 測試（清單見 `docs/architecture.md` 模組表）
- [ ] 新的純邏輯模組有對應單元測試

### 6. Rust 品質（major / minor）
- [ ] 無新增 `unsafe`（有 `// SAFETY:` 說明 = minor，無 = major）
- [ ] 執行期路徑未對外部輸入 `unwrap()`／`expect()`（可能 panic = major）
- [ ] 未用 `#[allow(...)]` 壓掉 clippy 警告而不附理由

### 7. 文件（minor；唯繪製／輸入缺 e2e 項 = major）
- [ ] 行為變更同步 README.md 與 `docs/architecture.md`
- [ ] 文件只寫現況與理由，沒有加入日期、任務編號、修正過程
- [ ] 動到 `view.rs` 的繪製或輸入：`docs/e2e-checklist.md` **必須**有對應項且寫明觸發條件。
      這類回歸單元測試必然全綠，e2e 項是唯一防線——**缺項 = major**

## 輸出格式

```
## Verdict: APPROVE | REQUEST_CHANGES

### 問題清單
| # | 嚴重度 | 檔案:行 | 問題 | 建議修法 |

### Checklist 摘要
1 安全 ✓/✗ · 2 輕量 ✓/✗ · 3 協定 ✓/✗ · 4 架構 ✓/✗ · 5 測試 ✓/✗ · 6 Rust ✓/✗ · 7 文件 ✓/✗
```

critical / major 任一存在 → REQUEST_CHANGES。
同一任務第 3 次審查仍有 critical/major → 在 verdict 末尾註明「建議 escalate 給使用者」。
