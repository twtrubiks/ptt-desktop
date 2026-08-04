# Plans — ptt-desktop

最後封存：2026-08-03（P1–P11 → [`docs/plans-archive.md`](docs/plans-archive.md)）

## 狀態標記

| 標記 | 意義 |
|---|---|
| `cc:TODO` | 待處理 |
| `cc:DOING` | 進行中（同時最多一個） |
| `cc:BLOCKED` | 卡住，待使用者裁決（附原因） |
| `cc:DONE` | 已完成 |

規則：標記由 `/harness-work` 與 `/harness-plan sync` 更動；`cc:DOING` 同時最多存在一個。

## 已封存任務索引

P1–P11 全文在 [`docs/plans-archive.md`](docs/plans-archive.md)——下表只留定案結論，
要實測數據、被否決的方案與其否決理由，去翻全文。
commit 一律是 `a1f424a`（11 個原始 commit 已 squash，舊 hash 不在 main 歷史上，說明見封存檔開頭）。

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

P1–P11 的手動驗證項全數收尾，無未結案項目（最後一項 P11 字型子集化於 2026-08-03 由使用者實測通過）。

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
- [ ] 手動驗證：docs/e2e-checklist.md 第 X 項（涉及 UI/連線行為時）

**風險：** 無 / 說明（涉及 telnet/codec/auto-login 有狀態邏輯、依賴變更時必填）

---
-->

### P12 — 第九根因：shadow 未模擬末列捲動，捲頁後補寫憑空殘字  `cc:DONE`

**目標：** 文章往下捲頁後回列表，畫面不再出現與內容無關的半個漢字。

**根因（實錄 `recordings/rec-2026-08-04T01-16-52-915Z.jsonl` 走真管線重播定位）：**
PTT 往下捲頁的文法是「`ESC[24;1H` + 連續 LF」（一次捲 5–6 列）。xterm 的
`lineFeed()` 在 scrollBottom 會呼叫 `bufferService.scroll()`（`scrollback: 0` →
捲出即丟），`screen-grid.ts` 卻是「夾住不捲」（P5 建模時的假設，本錄檔推翻）。
一次捲動後兩模型整段錯列，其後孤兒判定全落在舊畫面上 → 把已捲走的半字補寫到
新畫面。案發截圖：文章列表首欄憑空出現「但」的左半，壓在文章編號 778800 上。

**變更範圍：**
- [x] src/screen-grid.ts — LF／自動換行在末列改走 `lineFeed()` 捲動；dirty 與
      掛起的 big5 lead 隨之平移，捲出畫面者作廢
- [x] tests/screen-grid.test.ts — 5 項 edge case（末列 LF 上捲＋BCE、行尾自動換行
      捲動、捲動後孤兒列號平移、遞延孤兒隨捲動平移、捲出畫面的孤兒作廢）
- [x] docs/architecture.md — byte 網格語意規格新增第 6 條「末列捲動」
- [x] docs/e2e-checklist.md — 新增「文章捲頁後畫面無憑空殘字」項

**驗證：** 案發實錄重播，修法前重現截圖殘字、修法後消失；shadow 與 xterm 語意
畫面逐列比對 24/24 一致（修前 2 列不符），孤兒補寫 74 → 69 格。

**DoD：** quality ✓ / build ✓ / test ✓（166 項）；真人複驗見 e2e-checklist 對應項。

---

### P13 — 捲動的另一半：`ESC M`（RI）未模擬，上捲頁後同樣補出殘字  `cc:DONE`

**目標：** 文章**往上**捲頁後回列表，首欄同樣不再出現半個漢字（P12 只修了往下）。

**根因（實錄 `rec-2026-08-04T03-11-28-006Z.jsonl` 重播定位）：**
P12 的盤點只看第一份錄檔，該份使用者全程往下翻，非 CSI 的 ESC 序列為 0，
於是漏掉 PTT 往上捲頁的文法「`ESC[24;1H` `ESC[J` `ESC[H` `ESC M`」。
`ESC M`（RI）在頂列會讓 xterm 整體下捲一列，`screen-grid.ts` 卻把非 CSI 的
ESC 序列整個吃掉 → 與 P12 完全同構的錯列。第二份錄檔 `ESC M` 出現 37 次。

**變更範圍：**
- [x] src/screen-grid.ts — `ESC M` 進狀態機；頂列時整體下捲，`clearRow` 與
      `shiftPending` 由上捲／下捲共用
- [x] tests/screen-grid.test.ts — 5 項 RI edge case（頂列下捲＋BCE、非頂列只移
      游標、上捲頁後孤兒列號、遞延孤兒隨 RI 平移、擠出畫面底的孤兒作廢）；
      全部經「暫停 RI 分支」實測會失敗，非假成功
- [x] docs/architecture.md — 語意規格第 6 條改寫為「捲動（上捲／下捲）」，
      VT 子集盤點補註「只覆蓋當時錄到的操作路徑」

**驗證（兩份錄檔 × 三版 grid，走真管線重播 + xterm 語意畫面重建）：**

| grid 版本 | 錄檔1 repair／首欄誤補 | 錄檔2 repair／首欄誤補 |
|---|---|---|
| 修法前 | 82 ／ 4 | 48 ／ 14 |
| P12（+LF 上捲） | 74 ／ 0 | 46 ／ 15 |
| P13（+RI 下捲） | 74 ／ 0 | **28 ／ 0** |

兩份錄檔的 shadow 與畫面最終逐列比對皆 24/24 一致。

**DoD：** quality ✓ / build ✓ / test ✓（171 項）；真人複驗見 e2e-checklist。

---

### P14 — 防同類再犯：未實作序列偵測 + 實錄回歸工具  `cc:DONE`

**目標：** 下次 PTT 送來我們沒實作的終端序列時，在 dev console 與回歸工具立刻
現形，而不是等使用者截圖回報（P12／P13 是同一機制被咬兩次）。

**變更範圍：**
- [x] src/screen-grid.ts — `unknownSequences`（未實作的 ESC／CSI／C0 樣態，
      上限 16 筆）；CSI 只記 final byte 避免隨參數爆炸
- [x] src/main.ts — dev 模式下新樣態出現即 `console.warn`
- [x] scripts/replay-check.mjs + scripts/ts-loader.mjs — 錄檔走真管線重播 +
      xterm 語意畫面重建，四項判定：shadow 未實作序列、模擬器未實作序列、
      同一列連續分歧 ≥10 幀、補寫蓋掉既有內容
- [x] package.json — `npm run replay:check -- recordings/*.jsonl [--dump]`
- [x] tests/screen-grid.test.ts — 偵測面 3 項（已實作序列不入列、
      ESC/CSI/C0 各自入列、樣態數有上限）
- [x] docs/architecture.md、CLAUDE.md — 工具用法與判定依據

**為何模擬器也要偵測未實作序列：** P13 定位時模擬器自己也漏了 `ESC M`，於是
與 shadow「一致地錯」，兩模型比對顯示 0 分歧卻仍有殘字。只比對兩模型會得到
假綠燈，這條判定就是防它。

**閾值依據（實測）：** 同一列連續分歧幀數，兩份錄檔正常時最長 2 幀（補寫遞延
的暫態）；把 RI 分支停用重跑則達 92 幀。取 10 為界。
「最終畫面比對」不能當主判準——全螢幕重繪會讓兩模型收斂（停用 RI 版最終比對
仍是 0 列不一致）。

**驗證：** 停用 RI 分支重跑 `replay:check` → 正確 FAIL 並指出 `ESC M`；
還原後兩份錄檔皆 PASS。3 項新測試。

**DoD：** quality ✓ / build ✓ / test ✓（174 項）。錄檔含個資不進 repo，
`replay:check` 不列入 CI 的 DoD。

**未做（保留給下次裁決）：** 孤兒補寫的失敗模式保守化——不確定時留白而非憑空
補字。目前失敗模式是「憑空多半個字」，最刺眼；代價是真實破洞的修復率下降，
要重新評估補寫規則，故暫不動。

---

目前無進行中任務。新任務編號從 P15 接續。
