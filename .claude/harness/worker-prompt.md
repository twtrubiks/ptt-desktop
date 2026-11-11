# Worker 實作規範 — ptt-desktop

實作任務時遵循本文件。專案首要目標是「**輕量、快**」，所有取捨以此為準。

## 模組紀律

- 每個模組單一職責（模組表見 `docs/architecture.md`）
- 模組間的串接只發生在 `view.rs`；純邏輯模組（telnet / grid / auto_login / reconnect / hotkeys /
  urls / config）不依賴 gpui，才能直接單元測試
- 新增／刪除模組時同步 `docs/architecture.md` 的模組表

## 輕量原則

- 不新增 cargo 依賴。真的需要時：在回報的 `deps_added` 欄寫明「為何無法用現有依賴或 std 實作」，
  交 reviewer 裁決
- timer、執行緒、背景工作必須有明確停止條件與釋放路徑（參考 `view.rs` 的 500 ms tick：
  視窗失焦且畫面無閃爍字時停止）
- 交給 gpui 的資源要有釋放時機：圖片不走 asset 快取，用完以 `window.drop_image` 移除
- 不做推測性抽象；三行重複好過一個只用一次的 helper

## 有狀態模組（改動必附測試）

清單以 `docs/architecture.md` 模組表標「（有狀態）」者為準：

- `telnet.rs` — IAC 序列可能跨 frame；HTTP 前導只在連線最前出現一次
- `grid.rs` — 唯一的畫面模型。半字覆寫、SGR 劈字、捲動兩個方向（末列 LF 上捲、`ESC M` 下捲）
  都要對；多位元組字可能跨 chunk；沒實作的序列一律記進 `unknown_sequences`，不要靜默忽略
- `auto_login.rs` — 任一步異常立即交還手動，**絕不自動重試**；`SwitchUtf8` 只發一次；
  登入後不再回應代號／密碼提示
- `reconnect.rs` — 冷卻與二次確認的判斷
- `net.rs` — 非阻塞讀寫要讀到 `WouldBlock`；關閉一律走 `close_gracefully`，關好才回報 `Closed`

改這些模組必須補對應 edge case 測試（跨 frame、跨 chunk、捲動、異常路徑、逾時）。

**協定規格不能從單一錄檔歸納**：盤點序列時至少跨兩份錄檔，並想清楚反方向／對稱操作走的
是不是同一條路（往下捲頁與往上捲頁就是兩條文法）。

### 編碼切換時機（新舊 PTT 雙軌）

規格在 `docs/architecture.md`「編碼」段，改 `auto_login.rs` / `grid.rs` 前先讀：

- **新版**：收到「id,」立即切 UTF-8，密碼提示已是 UTF-8 → 位元組層偵測到就切
- **舊版**：密碼提示仍是 Big5 → 送出密碼後才切
- `view.rs` 必須先餵 `auto_login`、處理完切換事件，再餵 `grid`
- 「密碼不對」只在 Big5 位元組命中（舊版）時撥回 Big5

## 繪製與輸入（單元測試碰不到的一類）

字形位置、格子量測、IME、滑鼠、視窗關閉、GPU 資源這些只有實機看得出來，`cargo test` 一定全綠。
動到 `view.rs` 的繪製或輸入時，**測試綠燈不構成證據**：在 `docs/e2e-checklist.md` 加一項並寫明
觸發條件，再於回報的 `manual_e2e` 欄指出來。實機驗證一律由使用者做，不要自己用真實帳號啟動 app。

## 安全 Invariant（不可協商）

1. 自動登入**絕不自動重試**；重連不繞過 30 秒冷卻
2. 帳號密碼絕不出現在：log、錯誤訊息、測試 fixture、repo 任何檔案
3. 絕不讀取 `~/.config/ptt-desktop/`。範例與測試一律用 `config.example.json` 的假資料
4. `PTT_DUMP` 錄檔含帳號回顯，只能放 gitignore 的 `recordings/`

## Rust

- 不新增 `unsafe`（`net.rs` 的 `libc::poll` 是既有例外；不得不用時附 `// SAFETY:` 說明）
- 執行期路徑不 `unwrap()`／`expect()` 外部輸入（網路資料、設定檔、圖片）；測試內不限
- gpui 0.2 的 API 用法不確定時讀 `~/.cargo/registry` 的原始碼，不憑記憶猜

## Preflight（自我點檢，全綠才算完成）

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

失敗只能修實作或補測試，禁止弱化測試（`.claude/rules/test-quality.md`）。

## 回報格式

```
status: done | blocked
files_changed: [...]
summary: 1-3 行
preflight: fmt=PASS clippy=PASS test=PASS
deps_added: 無 | 套件名 + 無法用現有依賴的理由
manual_e2e: docs/e2e-checklist.md 對應項目，或「無 UI/連線行為變更」
```
