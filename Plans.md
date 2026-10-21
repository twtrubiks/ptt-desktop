# Plans — ptt-desktop

最後封存：2026-08-05（P12–P14 → [`docs/plans-archive.md`](docs/plans-archive.md)；前批 P1–P11 於 2026-08-03）

## 狀態標記

| 標記 | 意義 |
|---|---|
| `cc:TODO` | 待處理 |
| `cc:DOING` | 進行中（同時最多一個） |
| `cc:BLOCKED` | 卡住，待使用者裁決（附原因） |
| `cc:DONE` | 已完成 |

規則：標記由 `/harness-work` 與 `/harness-plan sync` 更動；`cc:DOING` 同時最多存在一個。

## 已封存任務索引

P1–P14 全文在 [`docs/plans-archive.md`](docs/plans-archive.md)——下表只留定案結論，
要實測數據、被否決的方案與其否決理由，去翻全文。
P1–P11 的 commit 一律是 `a1f424a`（11 個原始 commit 已 squash，舊 hash 不在 main 歷史上，
說明見封存檔開頭）；P12–P14 未經 squash，各自的 hash 可直接引用。

| # | 標題 | 定案結論 |
|---|---|---|
| P1 | 移植 v2 第 1、2 層：SGR 重排 + Big5 字寬 provider | 根因 1、2 修畢：亂碼歸零、方塊框線不再壓成條紋 |
| P2 | 重做第 3、4 層：webgl customGlyphs + 正方形 cell 選型 | 定案捆綁 PTT Square Mono（垂直度量改 1em）；`letterSpacing` 方案經使用者實測否決 |
| P3 | 復活 A 路線：雙色字 PUA 拆字 + overlay 疊繪 | 雙色字左右異色成立，是不換 renderer 下的唯一出路 |
| P4 | 第五根因探勘：dev 錄製 + 主功能表實錄 + byte 網格規格 | UTF-8 串流層無孤兒 byte，損傷全來自半字覆寫語意分歧；規格寫進 architecture.md |
| P5 | byte 網格 shadow 模組（純邏輯，不接線） | 定案「修復式」而非重寫式：批內自癒率 99.8%，補寫流量趨近零 |
| P6 | shadow 接線與主功能表驗收 | 蟲蛀破洞歸零；BS@80 定案前置正規化為絕對 CUP；sgr-reorder 保留雙層 |
| P7 | 第六根因：webgl customGlyphs 寬字只畫半格 | 定案方案 A `customGlyphs: false`，改走捆綁字型路徑 |
| P8 | 第七根因：未設 theme 的 xterm 預設 Tango 色盤 | 改用 PTT 傳統 VGA 色盤；theme 必須在建構子給定（熱更新會殘留舊字形圖集） |
| P9 | `lineHeight` 可調行高（預設維持 1） | 行距與彩繪無縫嚴格互斥，只能給旋鈕；預設維持 1，推薦值 1.08 寫進 README |
| P10 | 第八根因：合成粗體撐破字形圖集造成磁磚縫 | 定案 `fontWeightBold: "normal"`；SGR 1 的亮色由 `drawBoldTextInBrightColors` 負責，與字重無關 |
| P11 | 捆綁字型子集化：砍韓文諺文 | 10.89 → 9.35 MB，誤砍數 0；只砍諺文，不再擴大範圍 |
| P12 | 第九根因：shadow 未模擬末列捲動（`b953a32`） | PTT 往下捲頁＝「CUP 第 24 列 + 連續 LF」，shadow 必須跟著上捲，否則錯列後把捲走的半字補到新畫面 |
| P13 | 捲動的另一半：`ESC M`（RI）未模擬（`9faf430`） | 往上捲頁走「`ESC[24;1H ESC[J ESC[H ESC M`」，兩個方向都補齊後首欄誤補歸零；單一錄檔的序列盤點不能當協定規格 |
| P14 | 防同類再犯：未實作序列偵測 + 實錄回歸工具（`f28920f`） | `ScreenGrid.unknownSequences` + `npm run replay:check`；判準用「同一列連續分歧幀數」，最終畫面比對與兩模型一致都會假綠燈 |

P1–P14 **任務內**的手動驗證項已收尾（最後一項 P12／P13 的捲頁殘字複驗於 2026-08-04
由使用者實測通過）。

**待真人驗收的項目一律以 `docs/e2e-checklist.md` 的未打勾項為準**，Plans.md 不重複
記帳——沒開任務直接修掉的 hotfix（如 `8c2e382` 新版 PTT 編碼、`aa56257` 字型 race）
只會出現在該清單裡。

**待裁決事項（封存後仍有效）：** 孤兒補寫的失敗模式保守化——不確定時留白而非憑空
補字。現行失敗模式是「憑空多半個字」最刺眼，代價是真實破洞修復率下降，需重新評估
補寫規則。全文見封存檔第 2 批 P14。

## 任務列表

<!-- /harness-plan 產生的任務由此往下 append。格式範例：

### P1 — 範例任務標題  `cc:TODO`

**目標：** 一句話說明完成後使用者看得到什麼

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
-->

目前無進行中任務。新任務編號從 P15 接續。
