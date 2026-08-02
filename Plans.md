# Plans — ptt-desktop

## 狀態標記

| 標記 | 意義 |
|---|---|
| `cc:TODO` | 待處理 |
| `cc:DOING` | 進行中（同時最多一個） |
| `cc:BLOCKED` | 卡住，待使用者裁決（附原因） |
| `cc:DONE` | 已完成 |

規則：標記由 `/harness-work` 與 `/harness-plan sync` 更動；`cc:DOING` 同時最多存在一個。

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

> **背景（2026-08-01 彩繪重驗決定）**：2026-07-15「彩繪跑版不修」的決定經重新調查後發現前提有誤——
> v2 四層修法的第 4 層（正方形行高）在 xterm 6 上靜默失效（`lineHeight < 1` 拋例外被 try/catch 吞掉），
> A 路線改用的 `@font-face ascent-override` 在 WebKitGTK 也不支援（WebKit bug 219735），
> 所以當時實測的畫面始終帶著「縱向拉長 45%」。經使用者裁決重啟，分三階段重驗，每階段獨立驗收、隨時可停。
> 修法來源保全於本機 tags：`ansi-art-v2-backup`（ba7b92f 四層修法）、`ansi-art-a-route-stash`（28d41e1 A 路線 stash）。
> 三個 loose blob tag（`ansi-art-a-route-duotone-ts` / `-overlay-ts` / `-test-ts`）已於 2026-08-02 刪除——
> 內容經 P5–P8 五輪修改後全面覆蓋（分歧 31 / 6 / 102 行），非有效回退點；
> 原始內容備份於 `~/.claude/projects/-home-twtrubiks-work-code-run-learn-ptt-desktop/tag-backup/`（repo 外）。
> 已知硬限制：`@xterm/addon-canvas` 已在 xterm 6.0.0 被上游移除；雙色字 sub-cell 異色是 xterm cell 模型天花板，
> 只能靠 overlay（A 路線）解；PttChrome / pcman.js 渲染核心皆 GPL-2.0，不可直接抽用。

### P1 — 移植 v2 第 1、2 層：雙色字 SGR 重排 + Big5 字寬 provider  `cc:DONE`

**目標：** 彩繪亂碼歸零（U+FFFD 與雜訊字母消失，登入畫面實錄 111 處切割全數修復）、方塊/框線字元寬度正確不再壓縮成條紋；一般文字顯示不受影響。

**變更範圍：**
- [x] src/sgr-reorder.ts — 自 tag `ansi-art-v2-backup` 取回（含 `switchToUtf8()` 殘留 bytes 處理）
- [x] src/unicode-ptt.ts、src/wcwidth-data.ts — 同 tag 取回
- [x] src/main.ts — 接線 `codec.decode(reorder.process(filtered))` 與 `onSwitchUtf8` 殘留寫回
- [x] src/terminal.ts — 註冊 unicode provider + `allowProposedApi: true`（**只取這部分，不取 v2 的 fitSquareCell 與 CanvasAddon**）
- [x] tests/sgr-reorder.test.ts、tests/unicode-ptt.test.ts、tests/fixtures/welcome-big5.bin — 同 tag 取回（12 案例含跨 frame、逐 byte 最壞切割）
- [x] devDependencies：`@types/node` + tsconfig `types: ["node"]`（測試讀 fixture 需 node:fs 型別；dev-only，runtime 零新依賴，reviewer 裁決通過）
- [x] docs/architecture.md — 更新模組表；改寫「已知限制」章節（記載第 1、2 層已併入、第 4 層當年靜默失效的勘誤）
- [x] docs/e2e-checklist.md — 加「登入畫面彩繪橫幅無亂碼、無壓縮條紋」項

**注意：** 用 tag 逐檔移植，**不要 `git merge ansi-art-v2-backup`**（會帶入已死的 `@xterm/addon-canvas` 依賴與 `.npmrc`）；main 的 src/、tests/ 自分歧點後零改動，程式碼零衝突，只有 docs/architecture.md 需人工合寫。

**DoD（完成定義）：**
- [x] npm run quality 通過
- [x] npm run build 通過
- [x] npm test 通過（v2 測試全數保留，9 檔 81 測試全綠）
- [x] 手動驗證：登入畫面彩繪橫幅亂碼歸零、無條紋（2026-08-01 使用者實測通過）

**風險：** sgr-reorder 位於 telnet→codec 的有狀態 byte 流路徑，IAC/SGR 可能跨 frame——v2 既有 edge case 測試必須全數移植；unicode provider 把 EAW Ambiguous 一律報 2 格，PTT 之外來源的文字可能過寬（PTT 為唯一資料來源，影響可控，實測觀察）。

---

### P2 — 重做第 3、4 層：webgl customGlyphs 實測 + 正方形 cell 方案選型  `cc:DONE`（依賴：P1）

**目標：** 彩繪色塊無黑縫、整體比例接近正方（消除縱向拉長 45%）；或得出「WebKitGTK 上不可行」的明確結論收尾（負結論也是有效產出）。

**變更範圍：**
- [x] 實驗 A（黑縫）：掛 `@xterm/addon-webgl`（0.19.0，xterm 6 現役且支援 customGlyphs），WebKitGTK 下實測穩定性——新依賴，理由：`@xterm/addon-canvas` 已被上游移除，v6 的 customGlyphs 只存在於 webgl addon；穩定性疑慮（v2 當年避開 webgl 的原因）以實測裁決。掛載 try/catch + onContextLoss dispose，失敗退回 DOM renderer（reviewer 裁決通過）
- [x] 實驗 B（比例）：先試 B2 `letterSpacing`（零依賴），使用者實測**否決**（字太寬、文字觀感不可接受，2026-08-01）→ 回退並定案 B1：`scripts/build-font.py`（fonttools）自系統 Noto Sans Mono CJK TC 產製垂直度量修正版（hhea/usWin 改用 sTypo 880/-120 = 1em），依 OFL 改名 PTT Square Mono，woff2 10.9 MB 捆綁進 app（styles.css @font-face + config 預設字型），授權見 src/assets/OFL.txt
- [x] src/terminal.ts、docs/architecture.md、docs/e2e-checklist.md 對應更新（另補 tests/terminal.test.ts、README.md）

**禁止路徑（已證實靜默失敗，勿重蹈）：** `term.options.lineHeight < 1`（xterm 6 拋例外）；`@font-face ascent-override`（WebKit 不支援，bug 219735）。

**DoD（完成定義）：**
- [x] npm run quality 通過
- [x] npm run build 通過
- [x] npm test 通過（10 檔 86 測試）
- [x] 手動驗證：使用者親眼比對彩繪畫面（比例、黑縫）後裁決採用（2026-08-01 使用者實測通過）

**風險：** webgl 在 WebKitGTK 穩定性未知；新依賴 1 個（webgl addon）需 reviewer 裁決；B1 安裝包大小 vs B2 文字觀感的取捨由使用者實測後定案。

---

### P3 — 復活 A 路線：雙色字 PUA 拆字 + overlay 疊繪  `cc:DONE`（依賴：P2）

**目標：** 雙色字不再降級為前半色，一個全形字左右半異色，主功能表雨字等雙色密集彩繪接近 term.ptt.cc。

**變更範圍：**
- [x] src/duotone.ts、src/duotone-overlay.ts、tests/duotone.test.ts — 自 tags `ansi-art-a-route-duotone-ts` / `-overlay-ts` / `-test-ts`（loose blob）取回
- [x] src/sgr-reorder.ts、src/unicode-ptt.ts、src/terminal.ts、src/main.ts、src/blink-overlay.ts — 套 stash `ansi-art-a-route-stash`（28d41e1）的對應改動（APC marker 包裹、PUA 報 1 格、copy handler 還原選取文字）
- [x] docs/architecture.md、docs/e2e-checklist.md 同步

**DoD（完成定義）：**
- [x] npm run quality 通過
- [x] npm run build 通過
- [x] npm test 通過（10 檔 98 測試）
- [x] 手動驗證：雙色字左右異色正確、選取複製還原為原字、閃爍彩繪不異常
      （2026-08-01 首測：主功能表整體仍跑版，經診斷歸因於第五根因「byte 網格
      覆寫」→ 移交 P4，非雙色字拆分問題；本項待 P4 修畢後複驗）
      2026-08-02 複驗收尾：雙色字 59/59、複製還原 24/24 列一字不差為機械驗證，
      閃爍彩繪與真實剪貼簿由使用者實測通過（詳見 docs/e2e-checklist.md）

**風險：** overlay 疊繪與 P2 的 renderer 選型耦合（onRender 管線在 DOM/webgl 下行為需驗證）；動到 blink-overlay 匯出面與 copy 行為；體感效能需以「輕量、快」標準把關。降級安全：APC 序列漏處理時 xterm parser 會吞掉序列，畫面退回前半色（與現況相同）。

---

> **背景（2026-08-01 第五根因診斷，P4–P6 的依據）**：P3 實裝後主功能表動態看板仍跑版
> （蟲蛀缺塊、散落 ASCII 字母、錯位色塊），診斷確認為與前四層獨立的第五根因——
> **xterm「原子寬字元 cell」模型 vs PTT「byte 網格」模型不相容**：
> ① xterm 6.0.0 本機實測：覆寫全形字任一半，另一半整格被清空（PTT 期待留下半個字形），
> 動態看板以游標定位反覆覆寫播動畫，每次覆寫半字就吃掉一整字 → 蟲蛀缺塊；
> ② 區段重繪可切在字中間 → 孤兒 trail byte（0x40–0x7E 與 ASCII 字母重疊，實測 0x75 顯示為 `u`）、
> 孤兒 lead byte 解成 U+FFFD 且在字寬表報 2 格（`src/wcwidth-data.ts:41`），一個孤兒 byte 汙染兩格；
> ③ 對照組 term.ptt.cc（robertabcd/PttChrome `src/js/term_buf.js`）：每 cell 存 byte + `isLeadByte`
> 標記、渲染時 `b2u` 配對相鄰 cell，天生支援半字與覆寫。**注意其授權 GPL-2.0：只可讀行為寫規格，
> 程式碼一行不可抄。**
> 使用者裁決（2026-08-01）採 A 案治本：byte 網格 shadow buffer + 渲染時配對，
> 拆為 P4 探勘規格 → P5 純邏輯模組 → P6 接線驗收，各自獨立驗收、隨時可停。
> 尚未定案的事實（P4 要回答）：主功能表在登入後屬 UTF-8 串流，UTF-8 模式下孤兒/覆寫的
> 實際位元組樣態尚無實錄佐證（guest 實錄被「太多 guest」擋下，僅取得登入前畫面）。

### P4 — 第五根因探勘：dev 錄製模式 + 主功能表實錄 + byte 網格語意規格  `cc:DONE`（依賴：P3）

**目標：** 拿到主功能表動態看板的實錄串流與定量損傷分析（孤兒 byte、覆寫半字各佔多少），把「byte 網格語意規格」寫進 architecture.md，產出清洗後的 fixture——P5 據以實作與測試。

**變更範圍：**
- [x] src/main.ts（或獨立 debug 模組）— dev-only 下行錄製：telnet 濾層後的 bytes 落地。約束：零新依賴；只存在於 `import.meta.env.DEV` 路徑，release build 驗證不含；錄檔絕不自動寫進 repo
      （2026-08-01 完成：src/dev-recorder.ts JSONL POST → vite.config.ts `/__ptt-rec` dev middleware → recordings/（gitignore）；dist grep 無錄製路徑字樣；reviewer APPROVE）
- [x] 使用者實錄（需使用者操作）：`npm run tauri dev` 登入 → 停在主功能表看完動態看板整輪 → 匯出錄檔；手動步驟寫進任務回報
      （2026-08-01 完成：兩份錄檔留本機 recordings/，主檔 97 行 / 75319 bytes 下行、
      含 connect / utf8-switch 標記，seq 無缺漏）
- [x] 分析：定量統計孤兒 lead/trail、覆寫半字事件、FFFD 汙染格數；確認 UTF-8 模式下的位元組樣態（登入後串流為 UTF-8，規格必須涵蓋 big5/UTF-8 兩模式）
      （2026-08-01 完成：**utf-8 模式串流層無孤兒 byte**（無效 UTF-8 = 0、FFFD = 0），
      損傷全來自半字覆寫語意分歧——主功能表段可見洞 249 件（~4.4/s），90.8% 為動畫
      「空格×2 BS×2 寬字」文法錯相劈 pair，寬字錯相 12、窄字蓋半字 11、CUP 落 trail 31；
      段末兩模型收斂 diff=0 → 修復式可行。big5 段對照：SGR 劈 pair 158 處屬根因 1）
- [x] 行為規格：對照 PttChrome `term_buf.js` 的配對與 orphan byte 渲染語意，寫成 architecture.md 規格節（GPL-2.0——只讀行為，不抄碼）
      （2026-08-01 完成：「byte 網格語意規格」節，引用釘 robertabcd/PttChrome@7f2d6686dc63）
- [x] tests/fixtures/mainmenu-*.bin — 清洗後 fixture：狀態列使用者代號等個資必遮蔽（reviewer 必查；原始錄檔留本機不 commit）
      （2026-08-01 完成：mainmenu-utf8.bin，76 chunks / 65043 bytes，PTTREC1 framing；
      狀態列 ID 4 處等長遮蔽、排除歡迎/登出頁（IP 與上站資訊所在）；
      格式與遮蔽由 tests/mainmenu-fixture.test.ts 守門）
- [x] docs/architecture.md — 「PTT 彩繪」章節增補第五根因（修正現有「四個根因已全數修正」的過時敘述）

**DoD（完成定義）：**
- [x] npm run quality 通過
- [x] npm run build 通過
- [x] npm test 通過
- [x] 實錄檔取得 + 定量報告產出（損傷來源佔比）；語意規格進 architecture.md
- [x] 手動驗證：錄製模式 dev 可用（2026-08-01 使用者實錄成功）；release build（`npm run build` 產物）不含錄製路徑（grep dist 無 `__ptt-rec` 字樣）

**風險：** 錄檔含使用者個資（狀態列代號等）——原始錄檔留本機、進 repo 的 fixture 必經清洗，安全 invariant 2 適用；下行串流不含密碼（PTT 不回顯），reviewer 仍必查 fixture。

---

### P5 — byte 網格 shadow 模組（純邏輯，不接線）  `cc:DONE`（依賴：P4）

**目標：** 新模組以 P4 實錄 fixture 重放能正確重建 80×24 網格並輸出修復內容，測試全綠即驗收；main.ts 零改動。

**變更範圍：**
- [x] src/screen-grid.ts — 80×24 網格 + 極小 VT 子集（CUP/CUU/CUD/CUF/CUB/CR/LF/BS/ED/EL；SGR 屬性含 fg/bg/bold/blink/inverse）；big5 與 UTF-8 兩模式；依 P4 規格做相鄰配對與孤兒判定
      （2026-08-01 完成：feed() 回傳批末孤兒清單含各半自己的 SGR；寬度與 xterm
      provider 共用同一張 wcwidth-data 表；SGR 劈字容忍（duotone 左右半異色））
- [x] 設計裁決（依 P4 數據定，worker 留一行理由）：**修復式**（觀察原串流、每批後只補寫受損格——現有管線零改動、可獨立回退，預設傾向）vs **重寫式**（正規化重放髒行——語意最乾淨但取代面大）
      （定案修復式：P4 實錄段末兩模型收斂 diff=0；P5 重放再證批內自癒率 99.8%、
      批末僅 8 件/57s 需補寫——補寫流量趨近零，重寫式的取代面不值得）
- [x] tests/screen-grid.test.ts — P4 fixture 重放 snapshot、跨 chunk/跨 frame 序列、孤兒與覆寫 edge case、兩編碼模式、效能 sanity（每 chunk 更新須微秒級）
      （25 測試；fixture 重放期望值經獨立 Python 對照實作逐項交叉驗證後鎖定）
- [x] docs/architecture.md — 模組表與資料流（標注「未接線，P6 啟用」）

**DoD（完成定義）：**
- [x] npm run quality 通過
- [x] npm run build 通過
- [x] npm test 通過（fixture 重放結果與 P4 語意規格一致，以斷言呈現；26 測試、
      期望值經獨立 Python 對照實作交叉驗證，reviewer 亦第三方驗算一致）
- [x] main.ts 零改動（本任務不接線，無 UI/連線行為變更；git diff 驗證）

**風險：** 有狀態模組、序列可能跨 frame——edge case 測試比照 telnet/codec 標準；不可抄 GPL 程式碼（依 P4 規格重新實作）。

---

### P6 — shadow 接線與主功能表驗收  `cc:DONE`（依賴：P5；手動複驗已於 2026-08-02 收尾）

**目標：** 主功能表動態看板不再蟲蛀、無散落字母與錯位色塊，接近 term.ptt.cc；P3 的三個手動驗證項一併複驗收尾。

**變更範圍：**
- [x] src/main.ts — 接線（修復式：與 autoLogin.feedBytes 同點觀察 + 每批後補寫；若 P5 定案重寫式則調整下行組裝）
      （2026-08-01 完成：grid.feed 觀察 telnet 濾層後 bytes、下游改吃正規化後
      output、批末孤兒以 buildOrphanRepair 補寫；switchToUtf8／reset／密碼錯
      撥回與 codec 同時機。補寫遞延：批末停在行尾虛位（xterm restoreCursor
      經 _restrictCursor 夾掉 x=80 丟 pending-wrap）或 ESC/CSI 序列中途（注入
      劈壞跨批序列）時 dirty 跨批累積、待可安全注入再回報——fixture 8 件孤兒
      因此 3 件（23,50全／6,29◎／18,50進）在遞延期間被下一批動畫蓋掉自癒
      （xterm 端同批修復、無需補寫），逐批溯源驗證非漏報，餘 5 件批次不變）
- [x] BS@80 錯位損傷（P5 審查發現）：實錄 31 次「寫滿行尾後 BS」，xterm pending-wrap 語意（夾 79 再減→78）與 PTT byte 網格意圖（純減一→79）差一欄，行尾寬字整段左移＋殘留；孤兒回報表達不了，需裁決「行尾 BS 前置正規化為絕對 CUP」vs「shadow 雙游標 diff 補寫」，先量測實際畫面影響再定（規格見 architecture.md byte 網格語意規格第 6 點）
      （2026-08-01 量測（fixture 重放雙游標模擬）：31 次事件、每次錯位寫入
      恰為一個寬字（共 62 格、遍及 23 列、row 不分歧、無 erase 交纏），全屬
      動畫文法「BS×2 後寫寬字」。定案**前置正規化**：feed() 把行尾虛位 BS
      替換為同列 80 欄絕對 CUP，錯位在源頭消除、兩游標其後恆一致；
      「雙游標 diff 補寫」需完整模擬 xterm 網格且補寫前先閃錯字，不採）
- [x] 孤兒半字顯示 — 復用 P3 的 PUA 半字 + duotone-overlay（視 P4 規格擴充 orphan 側繪製）
      （2026-08-01 完成：overlay 掃 buffer 逐格獨立繪製，天生支援落單半字、
      零擴充；補寫序列由 duotone.buildOrphanRepair 產生（CUP+SGR+conceal+PUA
      逐格、ESC 7/8 包裹），glyph 超 BMP 或 U+FFFD 退帶色空格）
- [x] 評估簡化：shadow 若已涵蓋 sgr-reorder 職責，遷移必守 test-quality（測試等價遷移、不刪斷言、commit message 說明；不確定就保留雙層）
      （2026-08-01 裁決保留雙層：sgr-reorder 修 decode 層正確性——每個 SGR
      劈字都會出 U+FFFD 上畫面（實錄百餘處/畫面）；shadow 補寫只處理批末
      孤兒（5 件/57 秒），職責不重疊、無等價遷移可言）
- [x] cursor 位置一致性（PTT 輸入欄位）、自動登入 feedBytes 不受影響、選取複製還原不退化
      （feedBytes 與 dev-recorder 仍收正規化前原始 bytes；補寫序列 ESC 7/8
      保存還原游標與 SGR，真 xterm 端對端測試驗證；孤兒 PUA 半字走
      restoreDuotoneText 既有還原路徑）
- [x] docs/architecture.md、README.md、docs/e2e-checklist.md 同步（e2e 清單新增動態看板與游標兩項、P3 三項註記請複驗）

**DoD（完成定義）：**
- [x] npm run quality 通過
- [x] npm run build 通過
- [x] npm test 通過（146 測試；fixture 重放孤兒 8→5 件為遞延語意的正當變更，逐批溯源＋reviewer 獨立全網格重放交叉驗證，重放結束存活孤兒 0）
- [x] 手動驗證：e2e 清單「主功能表動態看板」「游標與輸入」新項 + P3 三項複驗
      2026-08-02 收尾：看板蟲蛀（破洞 0）、補寫不動游標（57 批次位移 0）為機械驗證；
      真實敲鍵的游標落點與卡頓感由使用者實測通過

**風險：** 動到下行主管線，爆炸半徑為 P1 以來最大——分小 commit、每步可回退；動畫期間每批補寫的效能體感以「輕量、快」實測把關；補寫與 blink/duotone overlay 的重繪互動需驗證。

---

> **背景（2026-08-02 第六根因診斷）**：使用者比對 ChromePTT 截圖發現主功能表分隔線變虛線（─ 空 ─ 空）。
> 證據鏈：PTT 以「空格×2 BS×2 + 寬字」文法送 U+2500，每個 ─ 佔 2 格（tests/fixtures/mainmenu-utf8.bin）；
> 我們的 provider 正確報 2 格（src/wcwidth-data.ts WIDE_RANGES 含 0x24EB–0x254B）；但 xterm webgl 的
> customGlyphs 自繪一律只畫單格寬——`node_modules/@xterm/addon-webgl/src/TextureAtlas.ts:517` 只傳
> `deviceCellWidth`，`CustomGlyphs.ts` 的 box/block/pattern 定義全無寬字處理 → **寬字右半格永遠空白**。
> 影響：主功能表 fixture 統計框線（U+2500–257F）250 個、方塊（U+2580–259F）700 個——分隔線虛線化、
> 前景方塊彩繪梳齒狀碎裂。**三角 ◢◣◤◥ 不受影響**（不在 customGlyphs 定義表，走字型路徑，
> 光柵化允許 2 格寬，TextureAtlas.ts:450）。duotone-overlay 只重畫 duotone PUA 半字，救不了此題。
> 另：使用者 08-01 18:02 的比對截圖早於 P3（18:19）與 P4–P6（19:06–20:22），
> P6 遺留的手動複驗必須用含 3bcc8c5 的新 build 重測。

### P7 — 第六根因：webgl customGlyphs 寬字只畫半格  `cc:DONE`（依賴：P6；手動複驗已於 2026-08-02 收尾。P2 黑縫回歸複驗時**驗出真的回歸**＝第八根因，另開 P10 修畢）

**目標：** 主功能表分隔線恢復連續實線、前景方塊彩繪無梳齒碎裂，與 term.ptt.cc 目視一致；一併完成 P6/P3 遺留的手動複驗。

**變更範圍：**
- [x] 設計裁決（worker 實測後擇一，留一行理由）：
      **A** `customGlyphs: false`——全靠捆綁字型 PTT Square Mono 走字型路徑（光柵化允許 2 格寬）；
      需先驗證字型含 U+2500–259F 全形字形（duotone-overlay.ts:6 註解載明 Noto CJK 缺 U+2581 等
      block elements——缺字會 fallback 到系統 monospace，寬度與造型不可控），且不回歸 P2 黑縫；
      **B** 推廣 P3 PUA 拆字機制——單色寬字方塊/框線也拆 PUA 半字交 duotone-overlay 幾何自繪
      （drawGeometry 需擴充 U+2500–257F 框線；overlay 每幀掃描量上升需實測）；
      **C** 報上游 xterm + 本地最小 workaround。取捨標準：「輕量、快」、不回歸 P2 黑縫、零新依賴
      （2026-08-02 定案 **A**：fonttools 實測捆綁字型 U+2500–259F **零缺字**、advance 全為
      1em（= 2 格）、方塊墨水恰滿行高箱（█ bbox = (0,-120)–(1000,880)，與 build-font.py
      手術後度量對齊，縱橫無縫平鋪）、─ 橫貫 x 0–1000 可跨格相連——
      duotone-overlay.ts:6「Noto CJK 缺 U+2581」的舊註解與實測不符（當時未捆綁字型），
      已修正該註解。一行設定、零依賴、零每幀成本，與漢字同一條已日常驗證的字型路徑；
      B 需 stream 層拆字＋overlay 每幀多掃 700+ 格，過重不採；C 不必要。
      殘餘黑縫風險（分數 dpr 的 1px 縫）交手動驗證裁決）
- [x] src/terminal.ts — `customGlyphs: false` + webgl 選用理由註解改寫；
      src/duotone-overlay.ts — 僅修正過時註解（幾何自繪路徑不動）
- [x] tests/font-coverage.test.ts — 字型字形覆蓋驗證：零依賴解析 woff2（node:zlib brotli +
      WOFF2 table directory），驗證 U+2500–259F 與 Big5 圖形符號域（TextDecoder("big5") 枚舉，
      與 wcwidth-data 產生法同源）全數有字形且 advance 為全形；含「中」= 1em、「A」= 0.5em
      的解析自我驗證；fixture 重放既有測試不回歸（150 測試全綠）
- [x] docs/architecture.md —「PTT 彩繪」章節增補第六根因（含根因 3 黑縫後記與 webgl 選用
      理由改寫）；docs/e2e-checklist.md 加「主功能表分隔線連續、方塊無梳齒」項（含 P2 黑縫
      同場複驗）；README.md 已知限制段同步

**DoD（完成定義）：**
- [x] npm run quality 通過
- [x] npm run build 通過
- [x] npm test 通過（150 測試；新增 font-coverage 4 測試，reviewer 以獨立重現腳本
      對真實字型交叉驗證解析正確性）
- [x] 手動驗證：新 build 比對 term.ptt.cc——分隔線連續、方塊無梳齒；同場複驗 P6 遺留五項
      （動態看板不蟲蛀、游標/輸入正常 + P3 三項）與 P2 黑縫不回歸（customGlyphs 停用
      改走字型路徑，分數 dpr 有 1px 縫理論風險），通過後把 P3/P6 的手動項收尾
      2026-08-02 收尾：分隔線空隙 0px（對照組切回 customGlyphs 出現 50% 虛線，反證有效）。
      **P2 黑縫的理論風險確實成真**——但成因不是分數 dpr，是合成粗體撐破字形圖集（第八根因），
      已另開 P10 修畢

**風險：** 動 renderer 視覺主路徑，全畫面受影響——需以登入畫面與主功能表兩場景實測把關；
方案 A 有字型缺字與黑縫回歸風險、方案 B 有每幀效能風險，皆以「輕量、快」標準裁決；零新依賴。

---

### P8 — 第七根因：未設 theme 的 xterm 預設 Tango 色盤  `cc:DONE`（手動複驗已於 2026-08-02 收尾）

**目標：** 首頁登入橫幅等黑底彩繪不再浮出灰黑拼布，全畫面色感與 term.ptt.cc 一致。

**診斷（2026-08-02，使用者提供首頁實錄 recordings/rec-2026-08-02T03-42-35-096Z.jsonl）：**
- 實錄離線餵真實管線（ScreenGrid → sgr-reorder → codec → duotone）：byte 語意層
  零損傷（零 U+FFFD、零孤兒半字、零補寫、零 BS@80）→ 剩餘差距必在渲染層
- headless Chrome 重播（真實 createTerminal + 捆綁字型 + webgl）截圖，對 ScreenGrid
  語意網格逐格比對背景色：`new Terminal({...})` 從未設 `theme`，xterm 預設 Tango
  色盤的「黑」#2e3436 ≠ 預設背景 #000000，首頁實錄 659/1920 格帶明確黑底（SGR 40）
  → 灰黑拼布；16 色全數偏離 BBS 慣例（紅 #cc0000 vs #800000 等）

**變更範圍：**
- [x] src/blink-overlay.ts — 色盤單一出處改為 PTT 傳統 VGA 色盤（色值來源 PttChrome
      src/styles/color.scss $color-0…15，term.ptt.cc 同源）；DEFAULT_FOREGROUND
      #ffffff → #c0c0c0（= color 7，BBS 慣例）；ANSI_16 改為匯出供 theme 共用
- [x] src/terminal.ts — theme 於**建構子**給定（A/B 實測：webgl 下 options.theme
      熱更新會殘留舊色字形圖集）
- [x] tests/blink-overlay.test.ts — 三個既有期望值換新色盤值（#3465a4→#000080、
      #c4a000→#808000、#ffffff→#c0c0c0；行為變更即本任務目標，於此記載）；
      新增 VGA 色盤守門三測試（16 色與 PttChrome 一致、黑底=預設背景、預設前景=color 7）
- [x] docs/architecture.md 第七根因與畫面呈現段、README.md 修正列表、
      docs/e2e-checklist.md 新增色盤複驗項

**DoD（完成定義）：**
- [x] npm run quality 通過
- [x] npm run build 通過
- [x] npm test 通過
- [x] 手動驗證：e2e 清單「色盤與 term.ptt.cc 一致」新項 + P7 遺留七項同場複驗
      （須用含本修正的新 build）
      2026-08-02 收尾：六個畫面 5000+ 空白格逐格比對 VGA 色值，0 個不符（兩引擎）

---

### P9 — `lineHeight` 可調行高（預設維持 1）  `cc:DONE`（旋鈕生效已於 2026-08-02 機械驗證：縫寬 1→0px、1.08→1px、1.2→4px）

**起因（2026-08-02 使用者實測回報）：** 捆綁字型 PTT Square Mono 讀長文偏擠，
改回系統 Noto Sans Mono CJK TC 觀感舒服但首頁彩繪跑版——兩者字面完全相同，
差別只在 hhea 行高 1.0em vs 1.448em。

**診斷（fonttools 實測 NotoSansCJK-Regular.ttc）：**
- 漢字「中」ink bounds (96,-79)–(902,840)：寬 806、高 919（/1000 em）。
  1.0em cell 下上下留白僅 8.1%（左右 19.4%）→ 字上下貼邊，即「滿板」觀感
- 方塊 U+2588 ink bounds (0,-120)–(1000,880)，**恰為 1em×1em** → cell 只要
  高過 1em，方塊列間必然露出等比例橫縫。Noto 原版 1.448em 即 44.8% 黑縫，
  這才是「首頁跑板」的真相（不只是縱向拉長，是整張圖被撕開）
- 結論：**行距與彩繪無縫嚴格互斥**，無免費午餐，只能給旋鈕讓使用者選點

**決策（2026-08-02，使用者定案）：預設維持 `1`，P2「正方形 cell」未被推翻。**
本任務只提供旋鈕，不移動預設點——彩繪保真是專案既有的取捨方向，行距屬個人
偏好，交由使用者自行開啟較妥；預設 1 亦等同 xterm 原生預設，對現有使用者
零行為變更。使用者實測 1.0 / 1.08 / 1.15 後認定 **1.08 為推薦值**（行距改善
有感、彩繪縫約 8% 格高難以察覺），此建議寫進 README 設定表與「已知限制」段，
供覺得字擠的人自行調整。

**變更範圍：**
- [x] src/config.ts — `AppConfig.lineHeight`（預設 1）＋ `sanitizeLineHeight()`
      夾到 1–2：xterm 對 < 1 直接拋例外（OptionsService.ts:177），手改設定檔
      填非法值會讓終端整個建不起來。小於 1 的有限數夾到下限 1（尊重填數字的
      意圖），非有限數才退回預設——兩條路徑期望值目前重合，測試仍分開守
- [x] src/terminal.ts — `createTerminal` 第 4 參數傳入 xterm options
- [x] src/main.ts — 傳 `config.lineHeight`
- [x] tests/config.test.ts（新）— 合法值（含 1.08、1.15）、< 1 夾下限、超上限、
      非有限數退預設、null 走 clamp、字串數字六類
- [x] config.example.json / README.md（設定表 + 已知限制）/ docs/architecture.md
      （畫面呈現段）/ docs/e2e-checklist.md

**未動（已查證，故不需改）：** duotone / blink / link-underline 三個 overlay 皆以
`deviceH / term.rows` 動態算格高，且 duotone 基線 `(cellH-(asc+desc))/2+asc` 與
webgl 的 `char.top = round((cell.height-char.height)/2)` 同為垂直置中 → 自動對齊。
`fitFontSize` 一般視窗下為寬度受限，字級不隨 lineHeight 改變。

**DoD（完成定義）：**
- [x] npm run quality 通過
- [x] npm run build 通過
- [x] npm test 通過（159 passed）
- [x] 手動驗證：e2e 清單「行高旋鈕」新項——預設（不設或設 1）畫面與 P8 完全
      相同、設 1.08 行距變寬且橫縫仍不明顯、設 1.2 縫明顯可見（兩端都變化
      才代表旋鈕真的生效），同場確認三個 overlay 對位不偏移
      2026-08-02 機械驗證通過：縫寬 1→0px、1.08→1px（4.0% 格高）、1.2→4px（14.3%），
      恰為 `cellH − fontSize`；雙色字三種行高皆 59/59、閃爍蓋色 40/40

---

### P10 — 第八根因：合成粗體撐破字形圖集造成方塊磁磚縫  `cc:DONE`（觀感複驗已於 2026-08-02 由使用者實測通過，定案採用 `fontWeightBold: "normal"`）

**發現方式（2026-08-02）：** 為了把 e2e 清單的目視項變成客觀判定，建了「實錄重播 →
截圖 → 逐格機械比對」的驗證 harness（保存於 repo 外
`~/.claude/projects/-home-twtrubiks-work-code-run-learn-ptt-desktop/verify-harness/`，
含 README 與踩坑紀錄）。跑 P2「色塊拼接無黑縫」回歸複驗時抓到本根因。

**症狀與定量：** 亮色（SGR 1）方塊彩繪被 1px 暗線切成磁磚拼貼。實錄
rec-2026-08-02T04-59-24-251Z seq 127（青色方塊看板）：垂直相鄰滿格方塊 **128 對
全部有縫**，接縫像素 #00ffff → (0,151,151)（亮度掉到 59%）。

**根因：** 捆綁字型 PTT Square Mono 只有 Regular，SGR 1 讓引擎合成粗體（stroke
加粗）把字形撐出 em 方框，超出部分在字形圖集格邊界被裁掉 → 方塊底緣與右緣各露
1px。這是 P7 停用 customGlyphs 的代價：改走字型路徑才會碰到合成粗體。

**變更範圍：**
- [x] src/terminal.ts — 建構子加 `fontWeightBold: "normal"`
- [x] src/duotone-overlay.ts — `ctx.font` 不再套 `bold`；連帶移除已無用的
      `DuotoneCell.bold` 欄位與其填值（避免同一個字因「是不是雙色字」粗細不同）
- [x] docs/architecture.md 第八根因與畫面呈現段、README.md 已知限制段、
      docs/e2e-checklist.md 複驗項

**為何可直接關字重（勿重議）：** BBS 的 SGR 1 語意是「選亮色」而非加粗，亮色映射
由 xterm 的 `drawBoldTextInBrightColors`（預設 true，typings 已查證）負責，與
`fontWeightBold` 互不相干，改字重不影響任何顏色。

**驗證（機械判定，非目視）：**
- WebKitGTK（＝ Tauri on Linux 的引擎，Xvfb + MiniBrowser）：水平縫 47→0、
  垂直縫 128→0，**全數歸零**
- 修正後全項複驗：色盤 5000+ 空白格 0 不符、分隔線 0 空隙、雙色字 59/59、
  蟲蛀破洞 0（三份長實錄 411/183/176 幀，含 36/16/8 次孤兒補寫）

**殘留（已知、未修，非本任務範圍）：** Blink 引擎下相鄰全形方塊間仍有 1px 色邊，
接縫像素三通道不等（如 (0,39,103)）＝次像素抗鋸齒特徵，與字級、customGlyphs
開關皆無關；WebKitGTK 無此現象故 Linux 版不受影響，Windows 的 WebView2 同為
Blink 可能可見（本機無法驗證）。

**DoD（完成定義）：**
- [x] npm run quality 通過
- [x] npm run build 通過
- [x] npm test 通過（159 passed）
- [x] 手動驗證：亮色方塊彩繪無磁磚縫；同場確認粗體字改為正常字重後整體觀感
      可接受（SGR 1 文字仍以亮色區分，只是不再加粗）
      2026-08-02 收尾：磁磚縫為機械驗證（WebKitGTK A/B——bold 垂直 128/128、水平 47/48
      全有縫，normal 皆 0）；觀感由使用者實測通過，定案採用

---

### P11 — 捆綁字型子集化：砍韓文諺文省 1.5 MB  `cc:DONE`

**起因（2026-08-02）：** 盤點 v0.1.3 以來 19 個 commit 的效能影響時發現，
`scripts/build-font.py` 只改垂直度量、**完全沒有子集化**，整包 Noto Sans Mono
CJK TC 原封輸出。實測 10.89 MB，佔 dist 產物的 **95.6%**（字型 11,414,724 B
vs JS 513,206 B + CSS 4,304 B）——與專案「輕量」首要目標直接衝突。

**定量（fonttools 實測）：** cmap 44,810 個 codepoint，其中韓文諺文 11,244、
CJK ExtA 6,582、ExtB+ 2,365。PTT 是 Big5 站台，畫面內容源自 Big5 編碼
（UTF-8 模式也只是把同一份 Big5 內容轉碼送出），諺文不在 Big5／UAO 映射域。

**只砍諺文、不砍更多的理由（勿再擴大）：** 缺字時 WebView 會 fallback 到系統
字型，而系統字型垂直度量不是 1em → 該列跑版，等於局部重現第四根因。試算
「Big5 標準 + 彩繪框線 + 注音假名」可壓到 5.23 MB（−52%），但那要砍掉 UAO
擴充字，風險與收益不成比例，**不做**。

**變更範圍：**
- [x] scripts/build-font.py — 加 `drop_hangul()`（subset 在改度量與改名之前，
      因為 subset 會重算 OS/2 欄位）；保留設定 name_IDs/name_legacy/
      notdef_outline/layout_features/desubroutinize 各有理由，見函式 docstring
- [x] src/assets/ptt-square-mono.woff2 — 重新產製，10.89 → **9.35 MB（−14.1%）**
- [x] tests/font-coverage.test.ts — 加兩項守門（字型是二進位產物，腳本改壞
      不會有編譯期訊號）：Big5 可解出的字全數保留、諺文必須不在 cmap 內
- [x] docs/architecture.md — 正方形 cell 段與第四根因段的體積數字與理由

**誤砍驗證（子集化前後逐一比對）：** Big5 可解出 19,806 個 codepoint，
子集化前後**缺的是同一批 6,220 個，誤砍數為 0**。那 6,220 個中 6,217 個是
Big5 造字區映射的 BMP PUA（U+E000–F8FF，標準字型本就不含），另 3 個
（U+007F、U+02CD、U+2252）是來源 Noto 字型自己就沒有。守門測試據此排除，
但保留「漢字主體 U+4E00–9FFF 一個都不能少」的斷言防止排除範圍無限擴張。

**DoD：**
- [x] npm run quality 通過
- [x] npm run build 通過（dist 11.93 → 10.32 MB）
- [x] npm test 通過（15 檔 161 測試，新增 2）
- [ ] 手動驗證：docs/e2e-checklist.md 字型子集化項（真人確認彩繪與中文字無缺字）

**風險：** 低。子集化誤砍數經機械比對為 0；殘餘風險是 Node 的 `TextDecoder("big5")`
與 WebView（WebKitGTK／WebView2）的 big5 索引若有差異，可能有 Node 解不出、
但 WebView 解得出的字落在被砍範圍——諺文不在任何 big5 索引的值域內，此風險理論上為零，
但未在真實 WebView 上驗證過。
