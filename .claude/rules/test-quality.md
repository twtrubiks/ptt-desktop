# 測試品質規則 — 防止「假成功」

## 絕對禁止

- 刪除測試、註解掉測試、加 `.skip` / `.todo` 讓測試不跑
- 弱化斷言（刪 expect、精確比對改成 `toBeTruthy`、放寬容忍值）
- 把期望值改成目前實作的實際輸出以求通過——除非行為變更本來就是任務目標，且 Plans.md 有記載
- 在測試中 mock 掉被測邏輯本身

## 方向不對稱原則

- **加測試**：自由，不需任何批准
- **刪/弱化測試**：一律視為 REQUEST_CHANGES 等級。需在 commit message 說明理由，reviewer 必查

## 專案特定要求

- 有狀態模組改動必附 edge case 測試（清單以 `docs/architecture.md` 模組表標
  「（有狀態）」者為準）：`telnet.ts`（IAC 序列跨 frame）、`codec.ts`（多位元組字元
  跨 chunk、編碼只切一次）、`auto-login.ts`（登入異常路徑）、`screen-grid.ts`
  （末列 LF 上捲、`ESC M` 下捲）、`sgr-reorder.ts` / `duotone.ts`（SGR／APC 跨 frame）
- 下行管線模組（screen-grid / sgr-reorder / codec / duotone）另跑 `npm run replay:check`
  對本機錄檔重播。它抓的是單元測試碰不到的跨模組脫節，**不可用來取代 edge case 測試**
- 測試 fixture 不得含真實帳密——一律用假資料（參考 `config.example.json`）

## 心法

測試紅了，先懷疑實作，最後才懷疑測試。
改任何測試前先回答：「這個測試原本想保護什麼？」
