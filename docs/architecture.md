# ptt-desktop 技術文件

PTT 桌面客戶端，Tauri 2 + xterm.js。本文件描述目前的架構與各模組的運作方式，
給想了解實作細節或參與開發的人。

選 Tauri 2 的理由：使用系統 WebView（Linux 為 WebKitGTK），Linux 主程式
執行檔實測約 14 MB，且約 98% 的程式碼是前端 TypeScript，開發速度快。Rust 側僅為 create-tauri-app
樣板加 plugin 註冊，不含業務邏輯。

## 模組架構

```
ptt-desktop/
├── src/                     # 前端 TypeScript（所有業務邏輯）
│   ├── main.ts              # 進入點：組裝各模組、連線／重連流程
│   ├── terminal.ts          # xterm.js 初始化（固定 80×24、字寬 provider、DOM renderer）、縮放調字型
│   ├── assets/              # 捆綁字型 PTT Square Mono（產製：scripts/build-font.py）＋ OFL 授權
│   ├── connection.ts        # tauri-plugin-websocket 封裝＋keepalive
│   ├── dev-recorder.ts      # dev-only 下行錄製（P4 探勘；release build 經 tree-shake 不含）
│   ├── telnet.ts            # 濾 IAC 協商與 HTTP 前導（有狀態，序列可能跨 frame）
│   ├── sgr-reorder.ts       # 雙色字重排：把插在字中間的 SGR 搬到字後、包 APC 區段（有狀態）
│   ├── codec.ts             # 兩階段解碼：big5 → utf-8（TextDecoder stream 模式）
│   ├── duotone.ts           # 雙色字拆分：APC 區段改寫為左右 PUA 半字（有狀態）
│   ├── duotone-overlay.ts   # 雙色字疊繪：掃 buffer 把 PUA 半字繪回異色字形（overlay canvas）
│   ├── screen-grid.ts       # byte 網格 shadow：半字語意 80×24 網格、孤兒回報、BS@80 正規化（有狀態）
│   ├── unicode-ptt.ts       # xterm 字寬 provider：Big5 語意字元一律 2 格
│   ├── wcwidth-data.ts      # 字寬資料表（產生檔，產生方式見檔頭）
│   ├── auto-login.ts        # 自動登入狀態機
│   ├── hotkeys.ts           # 快捷鍵攔截層
│   ├── blink-overlay.ts     # 補 xterm 未實作的 SGR 5 閃爍動畫（overlay canvas）
│   ├── link-underline.ts    # 網址常駐底線（overlay canvas）
│   ├── image-preview.ts     # 圖片連結 hover 浮動預覽
│   └── config.ts            # 讀取 ~/.config/ptt-desktop/config.json
├── src-tauri/               # Rust 側：Tauri 樣板 + plugin 註冊，不寫業務邏輯
└── tests/                   # vitest 單元測試
```

## 資料流

下行（PTT → 畫面）：

```
wss binary frame → tauri-plugin-websocket (Rust, 帶 Origin header)
  → telnet.ts 濾掉 IAC 與 HTTP 前導
  → screen-grid.ts byte 網格 shadow 觀察＋BS@80 正規化（見「byte 網格語意規格」；
    自動登入的位元組層「密碼不對」比對與 dev 錄製都收正規化前的原始 bytes）
  → sgr-reorder.ts 把插在字中間的 SGR 搬到字後、包 APC 區段（雙色字，見「彩繪修正層」）
  → codec.ts 解碼（登入前 big5、送出密碼後 utf-8）
  → duotone.ts 把 APC 區段改寫成「左半 PUA + SGR + 右半 PUA」（保留雙色）
  → xterm.write()（自動登入狀態機的 pattern matching 餵解碼後文字）
  → 批末若有仍存活的孤兒半字，接著寫入補寫序列（duotone.buildOrphanRepair）
```

上行（按鍵 → PTT）：

```
xterm keydown → hotkeys.ts（命中快捷鍵展開為指令序列，否則放行）
  → xterm.onData → TextEncoder 編成 UTF-8 → binary 送出
```

## PTT WebSocket 端點

- 官方端點 `wss://ws.ptt.cc/bbs`，
  連線需帶 `Origin: https://term.ptt.cc` header，
  由 tauri-plugin-websocket 的 `connect()` 自訂 headers 帶入（`src/connection.ts`）
- 串流內容是 telnet-over-WebSocket：連線即收到 IAC 協商
  （DO TERMINAL-TYPE、SB TERMINAL-TYPE SEND、DO NAWS、
  WILL ECHO/SGA/BINARY、DO BINARY）。實測不回應協商登入流程仍可進行，
  但這些 bytes 必須從畫面資料中濾掉
- 串流最前面有一段 `HTTP/1.1 200 OK\r\n\r\n` 前導（PTT 閘道行為），需丟棄

## telnet 濾層（`src/telnet.ts`）

濾層必須有狀態：IAC 序列可能被切在兩個 frame 之間。以 byte 為單位的狀態機處理：

- 開頭比對 `"HTTP/"`：符合則丟棄直到 `\r\n\r\n` 結束；不符則把已收位元組
  回放進一般處理，不吞資料
- IAC 協商只濾不回應（實測可行）：`IAC WILL/WONT/DO/DONT <option>` 吃掉
  三個 byte，`IAC SB ... IAC SE` 整段子協商丟棄，其餘單 byte 指令丟棄
- `IAC IAC` 還原為資料中的 `0xFF`

## 編碼（`src/codec.ts`）

PTT 登入前的畫面是 Big5(-UAO)；登入帳號後面加逗號（`id,`）可切換為 UTF-8 輸出,
實測切換點在串流中間：送出帳號後的「請輸入您的密碼:」仍是 Big5，
其後的輸出即為合法 UTF-8。因此：

- `PttDecoder` 先用 `TextDecoder('big5', {stream: true})`，
  自動登入狀態機送出密碼後呼叫 `switchToUtf8()` 換成
  `TextDecoder('utf-8', {stream: true})`
- 切換時 flush 舊 decoder，殘留的半個 big5 字以 U+FFFD 呈現並寫入畫面，
  避免位元組被靜默丟棄
- 多位元組字跨 frame 切割由 TextDecoder 的 stream 模式原生處理；
  非法位元組以 U+FFFD 替代（non-fatal 預設行為），串流不中斷
- 上行全程用 `TextEncoder`（UTF-8）：登入前的上行只有 ASCII（帳號、密碼），
  不受編碼影響；登入後 PTT 已在 UTF-8 模式
- 彩繪的半字元上色（SGR 插在全形字的兩個 byte 中間）原會讓 Big5 字被劈開、
  解出 U+FFFD——這是位元組被切割、不是查表查不到，標準 big5 表的覆蓋率已足夠，
  換更完整的對照表（如 Big5-UAO）解不出更多字。現由 decode 前的
  `sgr-reorder.ts` 重排修復（2026-07-13 實錄登入畫面過重排後 FFFD 歸零），
  見下方「彩繪修正層」

## 彩繪修正層（`src/sgr-reorder.ts`、`src/unicode-ptt.ts`、`src/duotone.ts`、`src/duotone-overlay.ts`）

彩繪跑版根因中的解碼與雙色字部分（詳見下方「PTT 彩繪」），第 1、2 層 2026-08-01
自 `ansi-art-v2-backup` 移植併入，雙色字拆分／疊繪自 A 路線 tags 復活：

1. **雙色字重排**（`src/sgr-reorder.ts`）：彩繪慣用「把 SGR 色碼插在一個全形字的
   兩個 byte 中間」讓左右半字異色（實錄登入畫面 4347 bytes 有 111 處）。
   TextDecoder 遇到被劈開的字吐 U+FFFD、孤兒 byte 變雜訊字母，甚至級聯錯位。
   `SgrReorderFilter` 在 decode 前把插在字中間的 SGR 序列搬到整個字後面
   （Big5 pair 與 UTF-8 序列皆處理），讓 decoder 正確配對。只搬 SGR，
   游標移動等其他 ESC 序列維持原樣。SGR 可能與字元 byte 一起跨 frame，
   濾層有狀態（同 TelnetFilter）；codec 切 utf-8 時須先呼叫其 `switchToUtf8()`
   取回 big5 殘留 bytes 先行解碼。重排後的「字 + SGR」整段包進 APC marker
   （純 ASCII，對 decoder 透明），交給解碼後的雙色字拆分層。

2. **字元寬度**（`src/unicode-ptt.ts`）：方塊、框線等彩繪主力字元屬 East Asian
   Ambiguous，xterm 內建 wcwidth 算 1 格，但 PTT 版面以 Big5 語意（2 格）編排，
   行內被壓縮、游標定位又拉回正確欄位 → 條紋與縫隙。`PttUnicodeProvider` 把
   「WHATWG big5 index 映射像 ∪ EAW W/F/A」一律回報 2 格（資料表
   `wcwidth-data.ts` 為產生檔，與 codec 用同一張 big5 對照表，天生一致）。
   需要 `allowProposedApi: true`（`term.unicode` 是 proposed API）。
   EAW Ambiguous 一律報 2 格的取捨：PTT 為唯一資料來源，影響可控。
   另把 Plane 15/16（雙色字拆出的 PUA 半字）一律報 1 格。

3. **雙色字拆分與疊繪**（`src/duotone.ts`、`src/duotone-overlay.ts`）：
   xterm 的 cell 模型一格只有一組前景色，無法表達 sub-cell 異色，重排後的雙色字
   原本整字降級為前半色。`DuotoneSplitFilter` 在解碼後的字串層把 APC 區段改寫成
   「左半 U+F0000+cp + SGR + 右半 U+100000+cp」（Plane 15/16 PUA，加減法可逆），
   xterm buffer 以「兩個窄字元、各自帶色」保存；半字前後包 SGR 8/28（conceal）
   讓 xterm 只畫背景不畫 .notdef 字形。`DuotoneOverlay` 再掃 buffer 把原字形的
   左右半各以該格顏色疊繪回去（方塊／三角以幾何自繪，其餘用字型 + clip 半格；
   左右半共用同一繪製原點，接縫零誤差）。
   選取複製時 capture 階段的 copy handler 以 `restoreDuotoneText()` 還原原字。
   安全降級：APC 區段若漏處理，xterm parser 會吞掉序列，畫面退回前半色。
   已知限制：PTT 串流若真送出 Plane 15/16 PUA 會被誤認半字（Big5-UAO 造字區
   在 BMP PUA，不受影響）；原串流本身開著 SGR 8 時結尾的 SGR 28 會提前解除。

## 自動登入狀態機（`src/auto-login.ts`）

只在連線建立後運作，對解碼後的下行文字做 pattern matching：

| 畫面出現 | 動作 |
|---|---|
| 「請輸入代號」 | 送出「帳號 + `,`」+ Enter（逗號啟用 UTF-8 模式） |
| 「請輸入您的密碼」 | 送出密碼 + Enter，**同時通知 codec 切換 utf-8 解碼** |
| 「您想刪除其他重複登入的連線嗎」 | 送出設定值（預設 `n`） |
| 「按任意鍵繼續」 | 送出空白鍵 |
| 「【主功能表】」 | 狀態機結束，之後全部直通手動 |
| 「密碼不對」、同一提示重複出現、或任一步卡超過 10 秒 | 中止並交還手動，**絕不自動重試**（避免鎖帳號） |

例外：「密碼不對」不能用解碼後文字比對——密碼錯時 PTT 從未切到 UTF-8、
仍以 Big5 重送畫面，但 codec 已切 utf-8，字面永遠比對不到。因此這條 pattern 在 telnet 濾層後的
**原始位元組層**比對 Big5 序列（`feedBytes()`），且 main.ts 在收到此中止原因時
會把 codec 撥回 big5，讓交還手動後的畫面可讀。

實作細節：比對 buffer 只保留最後 4096 字元；每次送出後清空已比對內容
（避免同一提示重複觸發）並重新起算 10 秒逾時。
設定檔沒有帳密時整個狀態機不啟動，全手動操作。

## 保持連線（keepalive，`src/connection.ts`）

- 每 60 秒送 telnet NOP（`FF F1`），不進入畫面，重置 PTT 的閒置計時。
  備選為 NUL（`0x00`），改 `KEEPALIVE_BYTES` 即可
- WebSocket 傳輸層的 ping/pong 由 plugin 底層處理，只維持傳輸層不斷線，
  與 BBS 應用層的閒置計時是兩回事，兩層都要顧
- keepalive 失效被踢時，以「斷線 → 按 Enter 重連 → 自動登入」兜底

## 斷線與重連（`src/main.ts`）

- 斷線 → 畫面顯示「已斷線，按 Enter 重連」，重連後重跑自動登入。
  不做無人值守自動重連（避免與 PTT 重複登入機制打架）
- 連線流程進行中以 `connecting` 旗標擋掉重入：連按 Ctrl+R / Enter
  不會開出第二條連線（否則舊連線變 ghost 掛在 PTT 上，
  兩個自動登入狀態機也會互相干擾）
- 重連前 reset 終端機、telnet 濾層、codec（回到 big5）

## 畫面呈現（`src/terminal.ts`）

- xterm 固定 80×24 字元格、`scrollback: 0`（BBS 為全螢幕應用，
  畫面由 PTT 自行管理）
- 視窗縮放只調字型大小（8–72），不改行列數；縮放比例對字型大小非嚴格線性，
  迭代兩次收斂。字型載入完成後重新量測一次（格寬會變）
- **DOM renderer**（xterm 內建，不掛任何 renderer addon）：曾於 P2 掛
  `@xterm/addon-webgl` 消除實心方塊拼接處的黑縫（webgl 以 glyph quad 整數格
  對齊平鋪無縫），但 0.1.4 實測**瀏覽一段時間後字形圖集（texture atlas）
  快取會損壞**：畫面字形碎裂疊影，而 buffer 內容正常（縮放視窗改字型大小
  觸發圖集重建即復原）。同一 renderer 在 VS Code 終端有同症狀
  （多頁圖集合併路徑 xtermjs/xterm.js#4061；vscode#174399、#287584、#69665），
  未能分辨是 addon 0.19.0 自身或 WebKitGTK／GPU 驅動層。取捨定為
  **正確性優先於無縫**：移除 addon 回到 DOM renderer，逐 `<span>` 排版，
  彩繪拼接處有細縫但永不亂碼（黑縫回歸記於「PTT 彩繪」根因 3）。
  `customGlyphs: false` 保留（DOM renderer 不支援此選項故為 no-op），
  作為防禦：日後若再掛 GPU renderer，第六根因不會靜默回歸
- **不套合成粗體**（`fontWeightBold: "normal"`，第八根因，見「PTT 彩繪」）：
  捆綁字型只有 Regular，合成粗體會把字形撐出 em 方框、在字形圖集邊界被
  裁掉，滿格方塊底緣與右緣各露 1px 暗線。BBS 的 SGR 1 語意是選亮色，
  亮色由 `drawBoldTextInBrightColors`（預設 true）負責，與字重無關；
  `duotone-overlay.ts` 的 `ctx.font` 同步不套 bold，否則同一個字會因
  「是不是雙色字」而粗細不同
- **VGA 色盤**（`theme`，第七根因，見「PTT 彩繪」）：色盤單一出處在
  `blink-overlay.ts`（匯出供 theme 與 overlay 疊繪共用），值為 PTT 傳統
  VGA 色盤（同 term.ptt.cc／PttChrome）；xterm 預設 Tango 的「黑」#2e3436
  與預設背景不同色，SGR 40 黑底彩繪會浮出灰黑拼布。theme 必須在建構子
  給定——webgl 下熱更新 `options.theme` 會殘留舊色字形圖集
- **正方形 cell**（捆綁字型 PTT Square Mono）：Noto Sans Mono CJK TC 的 hhea
  行高 1160/-288（1.448em）遠大於全形字寬（1em），xterm 以 fontBoundingBox
  當每列高度，畫面縱向拉長 45%。修法是捆綁一個只改垂直度量的衍生字型：
  hhea 與 OS/2 win 度量改用原字型自己的 sTypo 值 880/-120（加總恰為 1em），
  字面設計不動，xterm 量到的每列高度就等於全形字寬。產製腳本
  `scripts/build-font.py`（fonttools），輸出 `src/assets/ptt-square-mono.woff2`
  （9.3 MB，woff2 壓縮；同一支腳本順帶砍掉韓文諺文的 11,675 個 codepoint
  省 1.5 MB——PTT 是 Big5 站台，諺文不在 Big5／UAO 映射域，協定上送不出來。
  只砍「不可能出現」的字集是有原因的：缺字時 WebView 會 fallback 到系統字型，
  而系統字型的垂直度量不是 1em，該列就跑版＝局部重現本節要修的問題。
  `tests/font-coverage.test.ts` 兩項守門：Big5 可解出的字一個都不能少、
  諺文必須不在 cmap 內），`styles.css` 以 `@font-face` 載入，
  `config.ts` 預設 `fontFamily` 指向它；OFL 授權與衍生說明見
  `src/assets/OFL.txt`（"Noto" 為 Reserved Font Name，衍生版必須改名）。
  使用者自訂其他字型時彩繪會回到縱向拉長，屬可接受的自選取捨
- **`lineHeight` 旋鈕**（`config.ts`，預設 1）：正方 cell 讓漢字上下留白只剩
  8.1%（「中」字面 ink 高 919/1000，實測 fonttools），長文閱讀偏擠。此欄位
  讓使用者自行加行距，但**行距與彩繪無縫本質互斥**——方塊字形 U+2588 的
  ink bounds 為 (0,-120)–(1000,880)，恰為 1em×1em，cell 高過 1em 時列間必然
  露縫（Noto 原版 1.448 的 45% 黑縫即此成因）。xterm 對 < 1 拋例外，
  故 `sanitizeLineHeight` 夾到 1–2。> 1 時 webgl 把字形垂直置中於 cell
  （`char.top`），三個 overlay 皆以 `deviceH / rows` 動態算格高故自動對齊。
  預設維持 1（**P2 的正方 cell 決策未被推翻**，此欄位只是把選擇權交出去）：
  彩繪保真是本專案既有的取捨方向，行距改善屬個人偏好，故由使用者自行開啟。
  使用者實測（2026-08-02）1.08 為推薦值——行距改善有感、彩繪縫約 8% 格高
  難以察覺，已寫進 README 供有需要者參考。額外性質——`fitFontSize` 在一般視窗下是寬度受限
  （畫面比 80:24×lineHeight 恆比視窗扁），字級只由視窗寬決定，故加大
  lineHeight 不會縮小字級，等於把原本閒置的上下黑邊換成行距

### 渲染補強（overlay canvas）

四個功能都以「透明 canvas 疊在 xterm 文字層上、`onRender` 時重掃可視畫面」
的方式實作，不動 xterm 內部：

- **閃爍字**（`src/blink-overlay.ts`）：xterm.js 的 parser 有解析 SGR 5（blink）
  屬性，但 renderer 未實作動畫（上游自標 FIXME）。掃出帶 blink 屬性的格子，
  off 相位用該格的「有效背景色」蓋住文字，on 相位清空，各 500ms。
  只在畫面上有閃爍字且視窗可見時跑 timer；反白格的蓋色為其前景色
- **雙色字疊繪**（`src/duotone-overlay.ts`，見「彩繪修正層」）：內容隨畫面靜止，
  不需 timer；`onWriteParsed` + `onRender` + ResizeObserver 驅動重畫。
  z-index 與 blink-overlay 同層但**必須先掛載**（後掛載的 sibling 疊在上面），
  blink 的 off 相位才蓋得住會閃的雙色字
- **網址常駐底線**（`src/link-underline.ts`）：xterm 的連結底線是 hover 裝飾，
  滑過才顯示。這裡掃描可視畫面上的網址、常駐畫底線。regex 與
  `@xterm/addon-web-links` 的預設值逐字相同（套件未匯出，只能複製），
  保證「畫了底線」＝「可點擊」。全形字佔 2 格，字串索引到欄位需另建對照
- **圖片預覽**（`src/image-preview.ts`）：滑鼠停在圖片連結上 250ms 後浮動預覽，
  離開即移除。只處理直連圖檔（jpg/png/gif/webp）與 imgur 單圖頁面，
  不自動掃描畫面載圖（opt-in 才載）。`referrerPolicy: no-referrer`
  避開圖床對外站 Referer 的 403

網址點擊以 `@tauri-apps/plugin-opener` 開系統預設瀏覽器
（WebLinksAddon 的預設 handler 是 `window.open`，在 Tauri WebView 內無效）。

已知限制：PTT 是 server 端 80 欄硬換行，跨行網址無 `isWrapped` 標記，
底線與點擊都只涵蓋單行內的網址。

## PTT 彩繪（ANSI art）修正歷程

PTT 的 Big5 手工彩繪（登入畫面橫幅、主功能表的雨字、彩繪文章、簽名檔）
曾在 xterm.js 上與 term.ptt.cc 有明顯差距。前四個根因已修正（2026-08-01，
Plans.md P1–P3）；P3 驗證時發現第五根因（byte 網格覆寫，P4–P6 已修）；
主功能表複驗比對截圖時發現第六根因（webgl customGlyphs 寬字只畫半格，
P7 已修）；首頁彩繪實錄重播定量比對發現第七根因（未設 theme 的
Tango 色盤，P8 已修）；P8 修法的機械複驗（實錄重播＋逐格像素比對）
發現第八根因（合成粗體撐破字形圖集，P10 已修）。

### 根因與現況（已調查，不必重查）

八個各自獨立、疊加的根因：

1. **雙色字切割亂碼** — ✅ 已修（`src/sgr-reorder.ts`，見「彩繪修正層」）。
   雙色字降級為前半色的殘餘限制也已由 PUA 拆字 + overlay 疊繪解決
   （`src/duotone.ts`、`src/duotone-overlay.ts`）：xterm cell 模型一格一色的
   天花板，繞道「拆成兩個窄字元各自帶色、字形由 overlay 繪回」。
2. **字元寬度壓縮條紋** — ✅ 已修（`src/unicode-ptt.ts`，見「彩繪修正層」）。
3. **方塊黑縫** — ⚠️ **已知限制（0.1.4 後回歸，刻意取捨）**。實心方塊拼接處
   露出黑底細縫。非字型問題——真因是 xterm 預設的 DOM renderer 逐 `<span>`
   排版，相鄰格亞像素取整讓實心塊拼接處露底；xterm 內建的 `customGlyphs`
   （U+2500–259F 當背景色塊滿格自繪）**DOM renderer 不支援**
   （`xterm.d.ts` 明載），只在 canvas/webgl 生效。`@xterm/addon-canvas` 已在
   xterm 6 被上游移除，僅剩 `@xterm/addon-webgl`。
   （沿革：P2 曾掛 webgl renderer 修掉此縫，customGlyphs 於 P7 因第六根因停用、
   無縫改由「glyph quad 整數格平鋪＋捆綁字型墨水滿行高箱」保證；但 0.1.4
   實測 webgl 字形圖集用久了會損壞導致畫面亂碼，權衡後**移除 addon 回到
   DOM renderer**——黑縫是可預期的靜態瑕疵，亂碼是不可預期的功能故障，
   取捨標準為正確性優先，詳見「畫面呈現」的 DOM renderer 條目。）
4. **縱向拉長約 45%** — ✅ 已修（捆綁修正垂直度量的衍生字型 PTT Square Mono，
   見「畫面呈現」）。Noto Sans Mono CJK 的 hhea 行高 1.448em（ascender 1.16 +
   |descender| 0.288）遠大於全形字寬 1em，而 xterm 用 `measureText("W")` 的
   `fontBoundingBox` 當每列高度 → 每格是「寬 1em × 高 1.448em」的直立長方形。
   （文字拉長 45% 人眼不敏感，但幾何圖案的比例失真一眼可見。）
   **勘誤（2026-08-01）**：v2 時期的修法（動態量測 metrics 把 `lineHeight`
   壓到 <1）在 xterm 6 會拋例外、被 try/catch 吞掉而**靜默失效**——當年
   「四層修完仍跑版、決定不採用」的實測畫面其實始終帶著縱向拉長 45%，
   該結論的前提有誤，經使用者裁決於 2026-08-01 重啟分三階段重驗（Plans.md
   P1–P3）。走過的路：`lineHeight < 1` 不可行（xterm 6 拋例外）、
   `@font-face ascent-override` 不可行（WebKitGTK 不支援，WebKit bug 219735）、
   `letterSpacing` 加寬 cell 可行但文字變疏，使用者實測後否決（2026-08-01），
   最終定案捆綁字型（代價是安裝包 +9.3 MB，已砍諺文子集；未子集化時為 10.9 MB）。
5. **byte 網格覆寫（第五根因）** — ✅ 已修（Plans.md P4–P6，2026-08-01 接線；
   fixture 重放與單元測試驗證，真帳號手動驗收待複驗）。
   xterm 的「原子寬字元 cell」模型與 PTT 的「byte 網格」模型不相容：
   ① 覆寫全形字任一半，另一半整格被 xterm 清空（xterm 6 本機實測），而 PTT
   動態看板以游標定位反覆覆寫半字播動畫，每次覆寫就吃掉一整字 → 蟲蛀缺塊；
   ② 區段重繪可切在字中間 → 孤兒 trail byte（0x40–0x7E 與 ASCII 重疊，實測
   0x75 顯示為 `u`）、孤兒 lead byte 解成 U+FFFD 且字寬表報 2 格
   （`src/wcwidth-data.ts`），一個孤兒 byte 汙染兩格。
   對照組 term.ptt.cc（PttChrome `term_buf.js`）每 cell 存 byte + isLeadByte
   標記、渲染時 b2u 配對相鄰 cell，天生支援半字與覆寫——其授權 GPL-2.0，
   **只讀行為寫規格，程式碼一行不可抄**。
   治本方向（使用者裁決 2026-08-01）：byte 網格 shadow buffer + 渲染時配對，
   拆為 P4（探勘與規格）→ P5（純邏輯模組）→ P6（接線驗收）。
   P4 的 dev 錄製模式：`src/dev-recorder.ts` 把 telnet 濾層後的下行 bytes
   以 JSONL POST 給 vite.config.ts 的 `/__ptt-rec` dev middleware，落地到
   `recordings/`（已 gitignore，原始錄檔含個資絕不進 repo），供定量損傷分析
   與清洗後 fixture 產製。release build 驗證不含錄製路徑：main.ts 以
   `import.meta.env.DEV` 三元建構，build 時摺疊為 false 整個模組被
   tree-shake（可 grep dist 產物無 `__ptt-rec` 字樣佐證）。
6. **webgl customGlyphs 寬字只畫半格（第六根因）** — ✅ 已修（P7，2026-08-02：
   `customGlyphs: false`，`src/terminal.ts`）。主功能表分隔線顯示為虛線
   （─ 空 ─ 空）、前景方塊彩繪梳齒碎裂。PTT 的框線/方塊是 Big5 全形字、
   字寬 provider 正確報 2 格，但 `@xterm/addon-webgl` 的 customGlyphs 自繪
   一律只畫單格寬（`TextureAtlas.ts` 只把單格 `deviceCellWidth` 傳給
   `tryDrawCustomChar`，box/block/pattern 定義全無寬字處理）→ 右半格永遠
   空白。三角 ◢◣◤◥ 不受影響（不在 customGlyphs 定義表，走字型路徑；
   字型光柵化允許 2 格寬）。停用後框線/方塊與漢字同走字型路徑：
   捆綁字型 PTT Square Mono 實測完整含 U+2500–259F（fonttools 驗證
   0 缺字）、advance 全為 1em（= 2 格）、方塊墨水恰滿行高箱
   （█ 的 bbox = (0,-120)–(1000,880)，與 build-font.py 的垂直度量手術
   對齊，縱橫皆可無縫平鋪）；`tests/font-coverage.test.ts` 解析 woff2 的
   cmap/hmtx 守門，字型重製丟字形或改寬度會先紅。已知取捨：使用者自訂
   其他字型時若缺字/半寬，框線會退化——與「自訂字型回到縱向拉長」同屬
   自選取捨。另一殘餘不一致（已查證，不必重查）：字寬 provider 對
   U+254C–254F、2575–257F、2590–2591、2596–259F 報 1 格但字型 advance
   為 2 格，字形會溢出右鄰格——這些字元不在 WHATWG big5 映射域內
   （已獨立枚舉驗證可達的 U+25xx 全在 WIDE_RANGES），PTT 串流送不出來，
   實務不可達；若未來真的出現，先想起這段再查。
   （後記：0.1.4 移除 webgl addon 後 `customGlyphs` 已成 no-op，仍保留該設定
   作為防禦，見「畫面呈現」的 DOM renderer 條目。）
7. **未設 theme 的 Tango 色盤（第七根因）** — ✅ 已修（P8，2026-08-02：
   `src/terminal.ts` theme + `src/blink-overlay.ts` 色盤改值）。
   `new Terminal({...})` 從未設定 `theme`，一直用 xterm.js 預設的 Tango
   調色盤：「黑」是 #2e3436 暗灰、與預設背景 #000000 **不同色**，且 16 色
   全部偏離 BBS 慣例（紅 #cc0000 vs #800000、亮紅 #ef2929 vs #ff0000…）。
   可見損傷：① SGR 40 黑底彩繪浮出灰黑拼布——方塊字墨水以外與黑底空白
   畫 #2e3436，和預設背景的純黑拼成色塊（首頁賀歲彩繪實錄定量：
   659/1920 格帶明確黑底）；② 全畫面色感偏離 term.ptt.cc。
   修法：theme 設為 PTT 傳統 VGA 色盤（色值來源 PttChrome
   `src/styles/color.scss` $color-0…15，term.ptt.cc 同源）、預設前景
   #c0c0c0（= color 7，BBS 慣例「未標色即 SGR 37」）、預設背景 #000000
   （= color 0，明確黑底與預設背景恆同色）。色盤單一出處在
   `blink-overlay.ts`，theme 與兩個 overlay 疊繪取同一張表
   （`tests/blink-overlay.test.ts` 守門）。
   診斷方法（可重用）：dev 錄製實錄離線餵真實管線證得 byte 語意層
   零損傷，再以 headless Chrome 重播截圖對 ScreenGrid 語意網格逐格
   比對背景色，鎖定渲染層。
   注意（實測）：webgl 下以 `term.options.theme` 熱更新會殘留舊色
   字形圖集，theme 必須在建構子給定（0.1.4 已移除 webgl，此約束無害保留）。

8. **合成粗體撐破字形圖集（第八根因）** — ✅ 已修（P10，2026-08-02：
   `src/terminal.ts` 加 `fontWeightBold: "normal"`，`src/duotone-overlay.ts`
   的 `ctx.font` 同步不套 bold）。捆綁字型 PTT Square Mono 只有 Regular
   一個字重，SGR 1 讓引擎「合成粗體」（stroke 加粗）把字形撐出原本的
   em 方框，超出部分在字形圖集的格邊界被裁掉：**滿格方塊的底緣與右緣
   各露一條 1px 暗線**，大面積彩繪整片變成磁磚拼貼。
   這是第六根因修法（P7 停用 customGlyphs）的代價——改走字型路徑後才
   會碰到合成粗體；P7 之前方塊由 customGlyphs 以 fillRect 整格填滿，
   不經字型故無此問題（但寬字只畫半格，那是更嚴重的病）。
   定量（實錄 rec-2026-08-02T04-59-24-251Z seq 127，青色 SGR 1 方塊看板）：
   垂直相鄰滿格方塊 128 對**全部**有縫，接縫像素亮度掉到 59%
   （#00ffff → (0,151,151)）；WebKitGTK 與 Blink 兩引擎實測相同。
   修法有效性：WebKitGTK（＝ Tauri on Linux 的引擎）上水平 47→0、
   垂直 128→0，全數歸零。
   為何可以直接關掉字重：BBS 的 SGR 1 語意是「選亮色」而非加粗，亮色
   映射由 xterm 的 `drawBoldTextInBrightColors`（預設 true）負責，與
   `fontWeightBold` 互不相干，改字重不影響任何顏色。
   殘留（已知、未修）：Blink 引擎下相鄰全形方塊之間仍有 1px 色邊，
   接縫像素三通道不等（如 (0,39,103)）＝次像素抗鋸齒的色邊特徵，與
   字級、`customGlyphs` 開關皆無關；WebKitGTK 無此現象，故 Linux 版
   不受影響，Windows 的 WebView2 同為 Blink 可能可見（無法在本機驗證）。

### byte 網格語意規格（第五根因；P5 `screen-grid.ts` 的實作依據）

依 2026-08-01 主功能表實錄（`tests/fixtures/mainmenu-utf8.bin`，60 秒
utf-8 段 70930 bytes；原始錄檔留本機）與 PttChrome 行為對照寫成。
PttChrome 引用釘在 robertabcd/PttChrome@7f2d6686dc63（dev 分支，GPL-2.0，
**只讀行為寫規格，程式碼一行不可抄**）。

**三種模型的差異（問題的本質）：**

| | PTT server / PttChrome | xterm 6 |
|---|---|---|
| 儲存單位 | 一欄一 byte（utf-8 模式：整字＋空欄）| 寬字元為原子 2 欄 cell |
| 覆寫 pair 的一半 | 只改被寫的欄，另一半保留 | 另一半整格清空（本機實測）|
| 配對時機 | 每批寫入後全列重掃、由左而右貪婪配對 | 寫入當下即固定 |

PttChrome 行為出處：一欄一 char、覆寫不動 partner（`term_buf.js`
`puts()` L336–357，utf-8 模式全形字補寫下一欄空字串）；每批後
`updateCharAttr()` 重掃配對，「>ASCII 一律視為 lead」為其已知 FIXME
（L362–392，isLeadByte 設於 L377）；選取複製 snap 到 pair 邊界
（`getRowText()` L889/L893）；渲染配對失敗（半字被蓋）畫 `?`、trail
為空白則保留空白，左右異色 pair 走 `TwoColorWord`
（`components/Row/ColorSegmentBuilder.js` `readChar()`）；`isDBCSLead`
為 byte 範圍判定 0x81–0xFE（`string_util.js` L110–113），故 `?` 降級
只發生在 big5 byte 模式。

**實錄定量（主功能表段，76 chunks / 64731 bytes / 57 秒）：**

- VT 子集實測只有 `CSI m/H/K/J` 與 `BS/CR/LF`（全錄檔盤點：SGR 2688、
  CUP 2142、EL 162、ED 3、BS 3676；無 CUU/CUD/CUF/CUB）
- **utf-8 模式串流層無孤兒 byte**：無效 UTF-8 = 0、U+FFFD = 0。孤兒只存在
  於「欄」的層次（pair 被拆），不在位元組層——這回答了 P4 前尚未定案的
  「utf-8 模式孤兒樣態」問題
- 損傷全部來自半字覆寫語意分歧：可見洞 249 件（約 4.4 件/秒），其中
  90.8% 是動畫慣用的「空格清半格」——PTT 動態看板的標準文法是
  `CUP` → `空格×2 BS×2 寬字`（先清兩欄再寫寬字），對齊時無害，錯相時
  空格劈到隔壁 pair、xterm 就把另一半整格吃掉；其餘為寬字錯相疊寫
  12 件、窄字蓋半字 11 件；`CUP` 直落 trail 欄 31 次
- 動畫循環結束後兩模型收斂（段末 diff = 0 格）→ 損傷是暫態、會被後續
  重繪覆蓋，**修復式（每批後只補寫受損格）足以治本**，不必重寫整條管線
- big5 模式對照（登入前段）：SGR 劈開 pair 158 處屬根因 1（sgr-reorder
  已修）；孤兒 byte 原樣送出（先前實錄 trail byte 0x75 顯示為 `u`，
  見 Plans.md 第五根因診斷）

**shadow 網格語意（P5 實作規格）：**

1. 網格 80×24 欄；每欄獨立持有「窄字元」或「寬字元的左半／右半」
   （含該欄自己的 SGR 屬性）。寬字元寫入佔兩欄，窄字元佔一欄
2. 覆寫只改被覆蓋的欄；partner 欄降為**孤兒半字**（保留原字、左右半
   記號與原色）。EL/ED 清範圍內的欄，切在 pair 中間時同樣只清範圍內半邊
3. 編碼兩模式：big5 模式一欄一 byte（lead 0x81–0xFE + trail），utf-8
   模式一欄為整字之半；切換時機與 codec 相同（送出密碼後）
4. 渲染判定：相鄰同字的左半+右半 → 完整字（xterm 原生呈現即正確）；
   孤兒半字 → xterm 已把該格清空或呈現錯誤，需補寫。孤兒呈現採 P3 的
   PUA 半字＋duotone-overlay 繪回半個字形（**優於 PttChrome 的 `?`
   降級**，忠實還原 byte 網格終端機的半字視覺；此為有意的行為偏離）
5. 接線（P6 完成，修復式）：與 `autoLogin.feedBytes` 同點觀察 telnet 濾層後
   bytes；每批後將 shadow 與 xterm 的分歧格（被原子清空的 partner、
   錯相覆蓋）補寫回去。P5 重放實測：批內 orphan 事件 5253 件、
   **自癒率 99.8%**（動畫掃過去又自己寫回），批末仍存活、真正需補寫的
   僅個位數/57 秒——xterm 一次 `write()` 吃整個 chunk、批內暫態不上畫面，
   故補寫流量趨近於零。補寫序列（`duotone.buildOrphanRepair`）＝
   「絕對 CUP + 該半的 SGR + conceal + PUA 半字」逐格，整段以 ESC 7／ESC 8
   包裹保存還原游標與 SGR，對串流狀態透明。**補寫遞延**：批末游標停在
   行尾虛位（col=80，xterm `restoreCursor` 經 `_restrictCursor` 會把 x=80
   夾回 79、丟失 pending-wrap）或 ESC/CSI 序列中途（實錄確有跨 frame 序列，
   注入會劈壞它）時，該批不回報、dirty 跨批累積，待可安全注入的批末一併
   回報；遞延期間被動畫蓋掉的孤兒（xterm 端同批修復）自然免補。已知取捨：
   遞延後若長時間無下行資料，破洞會停留到下一批才補（fixture 76 批全部在
   下一批排空；PTT 閒置時游標停在輸入欄位，不會停在行尾虛位）
6. **末列捲動**（2026-08-04 修正，實錄 `rec-2026-08-04T01-16-52`）：LF 或自動
   換行發生在第 24 列時，網格整體上捲一列、末列以當前屬性清空（xterm
   `lineFeed()` 在 scrollBottom 呼叫 `bufferService.scroll()`，`scrollback: 0`
   → 捲出的列直接丟棄）。PTT 往下捲頁的文法正是「CUP 到第 24 列 + 連續 LF」
   （文章翻頁一次捲 5–6 列），shadow 早期假設「全螢幕不捲動、夾住即可」是
   **錯的**：一次捲動後兩模型整段錯列，其後所有孤兒判定都建立在舊畫面上，
   於是把已捲走的半字補寫到新畫面（實錄：文章列表首欄憑空出現「但」的左半，
   持續到該列被整列重繪才消失）。待補 dirty 與掛起的 big5 lead 隨捲動平移，
   捲出畫面者作廢
7. 行尾虛位（col=80）的 BS 裁決（P6 定案：**前置正規化為絕對 CUP**）：
   shadow 採 byte 網格意圖「純減一」（80→79），與 PttChrome 相同；xterm
   實際是 pending-wrap 語意「先夾 79 再減」（→78，`InputHandler.backspace`
   先 `_restrictCursor` 再減），實錄「寫滿行尾後 BS」共 31 次，xterm 端
   行尾寫入因此**整段左移一欄＋殘留**——此分歧不是孤兒、孤兒回報表達
   不了。P6 量測（fixture 重放雙游標模擬）：31 次事件每次錯位寫入恰為
   一個寬字（共 62 格、遍及 23 列、row 不分歧、無 erase 交纏），全部是
   動畫文法「BS×2 後寫寬字」。故 `feed()` 在轉發串流中把行尾虛位的 BS
   替換為「同列 80 欄的絕對 CUP」（≒純減一的落點），錯位在源頭消除、
   兩游標語意其後恆一致；對照方案「shadow 追蹤 xterm 游標做 diff 補寫」
   需完整模擬 xterm 端網格且補寫前會先閃錯字，不採。BS 劈在多位元組字
   中間（實錄不存在）時不替換，退回 xterm 原生行為

規格的實作為 `src/screen-grid.ts`（P5 建模、P6 接線）：`feed(bytes)` 逐批
更新網格，回傳「轉發用串流（BS@80 已正規化）」與「受影響且仍為孤兒、
此刻可安全補寫」的半格（含字形、左右半、該半自己的 fg/bg/bold/blink/
inverse），`switchToUtf8()` 與 codec 同時機切換、`reset()` 同重連時機。
fixture 重放的期望值（孤兒逐項、畫面重建、游標落點）經獨立 Python 對照
實作交叉驗證後鎖進 `tests/screen-grid.test.ts`（P5 批末孤兒 8 件，P6 補寫
遞延後其中 3 件在遞延期間被動畫自癒、無需補寫，餘 5 件回報批次不變）。

fixture 格式（`tests/fixtures/mainmenu-utf8.bin`，守門測試
`tests/mainmenu-fixture.test.ts`）：magic `PTTREC1\n` + 重複
`[u32 big-endian 長度][chunk bytes]`；chunk 邊界即實錄 WebSocket frame
邊界。已遮蔽狀態列使用者代號（等長 `x` 替換）、不含登入歡迎頁與登出頁
（IP 與上站資訊在該兩段）；動態看板廣告內的看板名與板主代號為
站方對所有使用者廣播的公開內容，比照 `welcome-big5.bin` 保留。

## 快捷鍵（`src/hotkeys.ts`）

掛在 xterm 的 `attachCustomKeyEventHandler`，keydown 階段處理：

1. 保留本地鍵優先：`Ctrl+R`＝手動重連、`F11`＝全螢幕切換（不送給 PTT）
2. 帶 Ctrl/Alt/Meta 修飾鍵的組合一律放行給 xterm；Shift+特殊鍵（如 Shift+F3）
   視為不同組合也放行（可列印字元的 shift 已反映在 `event.key`，不在此限）
3. 無修飾鍵時查設定檔的 `hotkeys` 表（`KeyboardEvent.key` → 送出序列），
   命中即送出序列並攔截；否則放行

## 設定檔（`src/config.ts`）

`~/.config/ptt-desktop/config.json`（macOS / Windows 亦讀取家目錄下同一路徑），
經 `@tauri-apps/plugin-fs` 讀取，與預設值合併；檔案不存在則全用預設值
（跳過自動登入，全手動）。檔案存在但 JSON 解析失敗時同樣改用預設值，
但會在終端機顯示錯誤並暫停於按 Enter 連線，不靜默吞掉。

密碼為明文儲存，是明確的取捨（同 `~/.netrc` 等級，使用者需自行 `chmod 600`）。
未來升級路徑：Rust 側加 keyring command 走 GNOME Keyring 等系統金鑰圈。

## 測試

`npm test`（vitest）單元測試涵蓋：

- `telnet.test.ts`：IAC 序列跨 frame 切割、HTTP 前導丟棄與非前導回放
- `sgr-reorder.test.ts`：雙色字 SGR 重排與 APC 包裹（Big5/UTF-8、跨 frame、
  逐 byte 最壞切割、切換 utf-8 殘留取回、防呆上限），含實錄登入畫面 fixture 全量驗證
- `duotone.test.ts`：PUA 半字進駐 xterm buffer 的閘門測試（各佔 1 格、各自帶色）、
  APC 區段拆分（跨 chunk、損壞區段、非 BMP 降級）、複製還原、
  孤兒補寫序列（逐 byte 精確比對、FFFD/超 BMP 降級空格、真 xterm 端對端
  驗證半字補回與游標/SGR 還原），含實錄 fixture 走完整下行管線的端對端驗證
- `unicode-ptt.test.ts`：字寬 provider（Ambiguous/W/F 報 2 格、組合字 0 格、
  與 xterm 內部寬度編碼一致）
- `codec.test.ts`：big5 → utf-8 切換時機、多位元組字跨 frame、切換點 flush
- `dev-recorder.test.ts`：錄製行格式（base64 可逆、seq 遞增、事件標記）、
  POST 序列化順序、失敗永久停錄不重試
- `mainmenu-fixture.test.ts`：mainmenu-utf8.bin 格式守門（framing 完整、
  嚴格 UTF-8、狀態列 ID 遮蔽與無 IP 樣態不回歸）
- `font-coverage.test.ts`：捆綁字型守門（customGlyphs 停用的前提）——
  零依賴解析 woff2（node:zlib brotli + WOFF2 table directory），驗證
  U+2500–259F 與 Big5 圖形符號域全數有字形且 advance 為全形 1em
- `screen-grid.test.ts`：byte 網格 shadow——半字覆寫孤兒回報（蓋左半/右半、
  錯相疊寫、動畫文法、EL 切 pair、行尾邊界）、SGR 劈字左右半異色、
  跨 chunk 狀態機（UTF-8 逐 byte 切割、CSI 跨批、big5 lead/trail 跨批）、
  big5 模式配對與孤兒、游標運動夾限、BS@80 正規化（含跨批與零拷貝）、
  補寫遞延（批末序列中途／行尾虛位、遞延期間自癒）、fixture 重放
  （期望值經獨立 Python 實作交叉驗證）、效能 sanity
- `auto-login.test.ts`：以錄製的真實登入畫面文字腳本餵入驗證狀態機；
  密碼錯誤路徑餵 Big5 位元組驗證位元組層比對（含跨 chunk 切割）
- `hotkeys.test.ts`：保留鍵、修飾鍵放行、快捷鍵展開
- `blink-overlay.test.ts`：blink 格子掃描、有效背景色（含反白）
- `link-underline.test.ts`：網址格位掃描（含全形字對位）、regex 與 addon 一致
- `image-preview.test.ts`：圖片網址判定（直連圖檔、imgur 改寫）

端對端（真帳號）為手動驗收：登入 → 逛板 → 看文 → 推中文 → 靜置驗 keepalive
→ 斷線重連。
