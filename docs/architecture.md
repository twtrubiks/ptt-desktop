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
│   ├── terminal.ts          # xterm.js 初始化（固定 80×24 格）、視窗縮放調字型
│   ├── connection.ts        # tauri-plugin-websocket 封裝＋keepalive
│   ├── telnet.ts            # 濾 IAC 協商與 HTTP 前導（有狀態，序列可能跨 frame）
│   ├── codec.ts             # 兩階段解碼：big5 → utf-8（TextDecoder stream 模式）
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
  → codec.ts 解碼（登入前 big5、送出密碼後 utf-8）
  → xterm.write()（同時餵給自動登入狀態機做 pattern matching）
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
- 彩繪的半字元上色（SGR 插在全形字的兩個 byte 中間）會讓 Big5 字被劈開、
  解出 U+FFFD，屬已知取捨，見下方「已知限制：PTT 彩繪（ANSI art）會跑版」。
  這是位元組被切割、不是查表查不到——實錄登入畫面的 FFFD 全數來自切割，
  標準 big5 表的覆蓋率已足夠，換更完整的對照表（如 Big5-UAO）解不出更多字

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

### 渲染補強（overlay canvas）

三個功能都以「透明 canvas 疊在 xterm 文字層上、`onRender` 時重掃可視畫面」
的方式實作，不動 xterm 內部：

- **閃爍字**（`src/blink-overlay.ts`）：xterm.js 的 parser 有解析 SGR 5（blink）
  屬性，但 renderer 未實作動畫（上游自標 FIXME）。掃出帶 blink 屬性的格子，
  off 相位用該格的「有效背景色」蓋住文字，on 相位清空，各 500ms。
  只在畫面上有閃爍字且視窗可見時跑 timer；反白格的蓋色為其前景色
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

## 已知限制：PTT 彩繪（ANSI art）會跑版

PTT 的 Big5 手工彩繪（登入畫面橫幅、主功能表的雨字、彩繪文章、簽名檔）
在 xterm.js 上顯示會跑版：比例、色塊接縫、雙色字都跟 term.ptt.cc 對不起來。

**影響範圍僅止於彩繪的視覺呈現。** 一般文字（看板列表、文章內文、推文、選單、
框線與【】欄位）的排版、對齊與色彩完全正常，所有功能不受影響——
日常逛板／看文／推文與 term.ptt.cc 無異，只有極致彩繪差一點。

### 根因（已調查，不必重查）

四個各自獨立、疊加的根因，每條的修法都已實作並驗證過：

1. **雙色字**：彩繪慣用「把 SGR 色碼插在一個全形字的兩個 byte 中間」讓左右半字
   異色（光登入畫面 4347 bytes 就有 111 處）。TextDecoder 遇到被劈開的字吐
   U+FFFD、孤兒 byte 變雜訊字母。修法是解碼前把 SGR 搬到整個字之後，
   代價是該字整個套用前半色（雙色降級為單色）。
2. **字元寬度**：方塊、框線等彩繪主力字元屬 East Asian Ambiguous，xterm 內建
   wcwidth 算 1 格，但 PTT 版面以 Big5 語意（2 格）編排，行內被壓縮、
   游標定位又拉回正確欄位 → 條紋與縫隙。修法是自訂 unicode provider
   讓 Big5 語意字元一律回報 2 格。
3. **方塊黑縫**：實心方塊拼接處露出黑底細縫。非字型問題——真因是 xterm 預設的
   DOM renderer 逐 `<span>` 排版，相鄰格亞像素取整讓實心塊拼接處露底；
   xterm 內建的 `customGlyphs`（U+2500–259F 當背景色塊滿格自繪）**DOM renderer
   不支援**（`xterm.d.ts` 明載），只在 canvas/webgl 生效。修法是改掛
   `@xterm/addon-canvas`。
4. **縱向拉長**：整張彩繪被縱向拉長約 45%（圓臉變高瘦）。Noto Sans Mono CJK 的
   hhea 行高 1.448em（ascender 1.16 + |descender| 0.288）遠大於全形字寬 1em，
   而 xterm 用 `measureText("W")` 的 `fontBoundingBox` 當每列高度 → 每格是
   「寬 1em × 高 1.448em」的直立長方形。（文字拉長 45% 人眼不敏感，
   但幾何圖案的比例失真一眼可見。）修法是動態量測字型 metrics 把每格壓成正方形。

### 決定：不採用該修法，接受現況

四層全部修完後實測，**UI/UX 仍然會跑版**，因此不採用。

term.ptt.cc（PttChrome）是自繪每個 byte-cell 的 PTT 專用渲染器，雙色與滿格色塊
是它的原生能力；要在通用終端 xterm.js 上做到像素級復刻，等於自幹渲染核心，
與本專案「薄、少依賴、不重新排版」的取向不符，投報率不成立。
彩繪跑版換到的是 xterm.js 帶來的整體穩定度與極小的維護面。

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
- `codec.test.ts`：big5 → utf-8 切換時機、多位元組字跨 frame、切換點 flush
- `auto-login.test.ts`：以錄製的真實登入畫面文字腳本餵入驗證狀態機；
  密碼錯誤路徑餵 Big5 位元組驗證位元組層比對（含跨 chunk 切割）
- `hotkeys.test.ts`：保留鍵、修飾鍵放行、快捷鍵展開
- `blink-overlay.test.ts`：blink 格子掃描、有效背景色（含反白）
- `link-underline.test.ts`：網址格位掃描（含全形字對位）、regex 與 addon 一致
- `image-preview.test.ts`：圖片網址判定（直連圖檔、imgur 改寫）

端對端（真帳號）為手動驗收：登入 → 逛板 → 看文 → 推中文 → 靜置驗 keepalive
→ 斷線重連。
