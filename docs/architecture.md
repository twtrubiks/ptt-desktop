# ptt-desktop 架構

PTT 桌面客戶端，GPUI 全 Rust。固定 80×24，經官方 WebSocket 端點連線。
首要設計目標是輕量、快。

## 模組

`src/` 每檔一模組、單一職責，組裝只在 `view.rs`（事件串接）與 `main.rs`（開視窗）。
標「（有狀態）」的模組改動必附 edge case 測試。

| 模組 | 職責 |
|---|---|
| `main.rs` | 進入點：log、讀設定、開視窗 |
| `view.rs` | 視窗與繪製（格子量測、字形、方塊幾何、閃爍、游標、底線、選取、圖片預覽），以及鍵盤／滑鼠／IME 輸入與各模組的串接 |
| `net.rs`（有狀態） | WebSocket 連線：一條連線一個執行緒，keepalive，正常關閉 |
| `telnet.rs`（有狀態） | 下行濾層：丟 HTTP 前導、濾 IAC 協商 |
| `grid.rs`（有狀態） | 80×24 byte 網格，唯一的畫面模型，也是終端序列解析器 |
| `auto_login.rs`（有狀態） | 自動登入狀態機，驅動編碼切換 |
| `reconnect.rs`（有狀態） | 重連節流：30 秒冷卻、線上二次確認 |
| `config.rs` | 讀 `~/.config/ptt-desktop/config.json` |
| `hotkeys.rs` | 自訂快捷鍵與終端機按鍵編碼 |
| `urls.rs` | 從一列文字找出網址範圍 |
| `image_preview.rs` | 圖片連結判斷、阻塞式 HTTPS 抓圖、解碼 |
| `palette.rs` | PTT 傳統 VGA 16 色 |
| `wcwidth.rs`／`wcwidth_data.rs` | 字寬表（資料檔為產生檔，勿手改） |

## 資料流

```
下行：ws.ptt.cc → net.rs（執行緒）→ channel → view.rs
        → telnet.rs 濾層 → auto_login.rs（位元組層比對，可能切編碼）→ grid.rs → 重繪
上行：鍵盤／IME／貼上／滾輪 → hotkeys.rs 編碼 → net.rs 佇列 → 喚醒網路執行緒送出
```

`view.rs` 必須先把 bytes 餵給 `auto_login`、處理完編碼切換事件，再餵給 `grid`，
提示才不會先以錯的編碼上畫面。

## 連線（`net.rs`）

- 端點 `wss://ws.ptt.cc/bbs`，必須帶 `Origin: https://term.ptt.cc`。
- ws.ptt.cc 不支援 TLS 1.3，rustls 預設後端握手失敗，所以用 native-tls（openssl）。
- 握手後 socket 改非阻塞，執行緒以 `poll(2)` 同時等 socket 與喚醒通道：UI 送鍵即喚醒，
  按鍵不必等讀取逾時；閒置時直接睡到下次 keepalive。每次醒來要把 `ws.read()` 讀到
  `WouldBlock` 為止，TLS 內部已緩衝的資料不會讓 socket 再變可讀。
- 每 60 秒送 telnet NOP（`FF F1`）防閒置踢出。
- 關閉一律走 `close_gracefully`：送 WebSocket Close、等伺服器回覆（最多 2 秒）、
  送 TLS close_notify、關 socket，之後才回報 `Closed`。重連與關視窗都靠這個回報確認舊連線已收乾淨。
- 只支援 Unix（`UnixStream::pair`、`libc::poll`）。

## telnet 濾層（`telnet.rs`）

以 byte 為單位的狀態機，IAC 序列可能跨 frame：

- 串流最前面的 `HTTP/1.1 200 OK\r\n\r\n` 前導丟棄，只出現一次；開頭不是 `HTTP/` 就把已收位元組放回一般處理。
- IAC 協商只濾不回應：`IAC WILL/WONT/DO/DONT <opt>` 三個 byte、`IAC SB … IAC SE` 整段丟棄。
- `IAC IAC` 還原為資料中的 `0xFF`。

## 編碼

登入前是 Big5；帳號後加逗號（`id,`）登入後切 UTF-8。切換點在串流中間，新舊 PTT 時機不同：

- **新版**：收到 `id,` 立即切，密碼提示已是 UTF-8。
- **舊版**：密碼提示仍是 Big5，送出密碼後才是 UTF-8；密碼錯時從未切換。

`auto_login.rs` 對每個提示字樣同時準備 Big5 與 UTF-8 兩種位元組序列，在位元組層比對：

- 看到 UTF-8 的密碼提示就發 `SwitchUtf8`（新版），否則送出密碼後發（舊版）。`SwitchUtf8` 保證只發一次。
- 「密碼不對」以 Big5 位元組命中（舊版）才發 `ResetBig5` 把網格撥回 Big5；UTF-8 命中則維持。
- `view.rs` 收到事件呼叫 `grid.set_utf8()`。沒設定帳密時不跑自動登入，由使用者手動登入。

上行全程 UTF-8：登入前只有 ASCII，登入後 PTT 已在 UTF-8 模式。

## byte 網格（`grid.rs`）

PTT 的畫面是「一欄一 byte」的網格，寬字佔兩欄但兩欄各自獨立，這與一般終端機「寬字是原子 cell」不同。

1. 每欄獨立持有窄字元或寬字元的左半／右半，各有自己的 SGR 屬性。
2. 覆寫只改被寫的欄，另一半降為孤兒半字（保留字形與左右半記號），繪製時只畫那半邊。
   EL／ED 切在字中間時同樣只清範圍內半邊。
3. Big5 模式一欄一 byte（lead 0x81–0xFE + trail）；UTF-8 模式一欄為整字之半。
4. SGR 可以劈在字中間：左半取字首當下的屬性、右半取字尾當下的屬性（雙色字）。
5. 捲動兩個方向都要：第 24 列的 LF 或自動換行上捲一列；第 1 列的 `ESC M` 下捲一列。
   PTT 往下捲頁是「CUP 到第 24 列 + 連續 LF」，往上捲頁是 `ESC[24;1H ESC[J ESC[H ESC M`。
6. 行尾虛位（col=80）的 BS 為純減一。
7. 已處理的序列：SGR、游標定位與移動（CUP、CUU／CUD／CUF／CUB）、EL、ED、游標存取（`ESC 7/8`、
   `CSI s/u`）、`ESC M`、`ESC[?7h/l`（自動換行）、`ESC[?25h/l`（游標顯示）、`ESC[5n`／`ESC[6n`
   （狀態與游標位置回報，要回應）；`ESC[?1000–1006`（滑鼠回報）與 `ESC[?2026`（同步更新）為 no-op。
   **沒實作的序列一律記進 `unknown_sequences` 並印到 stderr，不可靜默忽略**。

盤點序列不能只看一份錄檔：反方向的操作（往上捲頁）可能走另一條文法。

測試 fixture 在 `tests/fixtures/`：`welcome-big5.bin`（歡迎畫面）與 `mainmenu-utf8.bin`
（主功能表實錄，格式為 magic `PTTREC1\n` + 重複的 `[u32 BE 長度][chunk]`，chunk 邊界即 WebSocket
frame 邊界；狀態列的使用者代號已遮蔽）。

## 自動登入（`auto_login.rs`）

不持 timer，由 `view.rs` 的 500 ms tick 呼叫 `poll(now)` 查逾時。

- **分登入前、登入後兩段**，看到「密碼正確」或重複登入提示即算已登入。
  - 登入前每步 10 秒逾時，出錯在畫面寫「[自動登入中止（…）]」，因為畫面還停在登入畫面。
  - 登入後每步 20 秒逾時，只在底部浮出提示、安靜交還。伺服器回答重複登入後會刻意等幾秒，所以放寬。
  - 登入後不再理會代號、密碼提示與「密碼不對」字樣，畫面碰巧含這些字也不會送出帳密。
- **使用者一按鍵就交還**（按鍵、貼上、輸入法送字都算）：避免兩邊各送一個鍵。
- **刻意不自動回答**錯誤嘗試記錄、新看板、使用者條款這類提示，要讓人親眼看到。
- 不論哪一種交還都**不重試**。

## 重連（`reconnect.rs` + `view.rs`）

PTT 短時間內重複登入會顯示「請勿頻繁登入」，所以：

1. **30 秒登入冷卻**：從每次開始連線起算。冷卻中只顯示剩餘秒數，不排程、不自動連。
2. **線上要按兩次**：連線還在時 Ctrl+R 要在 3 秒內按第二次。斷線後按 Enter 不需確認。
3. **等舊連線關閉才開新的**：3 秒內沒關好就放棄等待，交還使用者，不自動連。

冷卻只存在記憶體。`kickOtherLogins` 預設 `y`，萬一舊連線沒清乾淨，下次登入會順手踢掉。

## 關視窗

`on_window_should_close` 攔下關閉，等網路執行緒回報關好才結束，最多 3 秒；等待中再關一次立即結束。
這只是關閉連線，不替使用者送登出按鍵：畫面可能停在編輯文章，盲送按鍵會把字打進去。

## 繪製（`view.rs`）

- 格子大小由視窗大小決定，字級由格寬反推；`lineHeight` 只拉高格子。
- 字形垂直位置用 `center_ratio = 0.38`（針對 Noto Sans Mono CJK TC 校準），換字型對不齊時調它。
- 方塊元素（U+2580–259F、◢◣◤◥）幾何自繪，拼接無縫。
- 半字與雙色字以 content mask 各畫半邊。
- 500 ms tick 負責閃爍字、游標閃爍與自動登入逾時；視窗失焦且畫面上沒有閃爍字時停止。
- 滾輪一格送一次 ↑／↓，自動登入進行中不送。

## 圖片預覽（`image_preview.rs` + `view.rs`）

滑鼠停在圖片連結 250 ms 後，在背景執行緒以阻塞式 HTTPS 抓圖並解碼成 `RenderImage`。
不走 gpui 的 asset 快取，預覽收掉後下一次 render 以 `window.drop_image` 從 GPU 圖集移除，
否則記憶體只增不減。下載上限一般圖 8 MB、GIF 20 MB；GIF 解碼後超過 128 MB 只留第一格。

預覽放在滑鼠左右較寬的一側、高度可用滿視窗，等比縮到放得下（不放大）。寬高由 `preview_rect`
自己算：交給 `max_w`／`max_h` 的話 taffy 兩軸各自夾限、不保比例，直式圖會被縮得過小。

## 除錯

- `PTT_LOG=info|debug|trace`：內部 log 印到 stderr，不含帳密與畫面內容。
- `PTT_DUMP=<檔案>`：把 telnet 濾層後的下行 bytes 原樣附加到檔案。錄檔不含送出的密碼，
  但含 PTT 回顯的帳號，必須放在 gitignore 的 `recordings/`。

## 建置備註

`build.rs` 是連結墊片：gpui 要不帶版號的 `libxkbcommon-x11.so`，Debian 把它放在
`libxkbcommon-x11-dev`。沒裝時在 `$OUT_DIR` 建一個指向執行期 `.so.0` 的 symlink，裝了就什麼都不做。

GPU 用量偏高是 gpui 0.2.2 的問題，目前等上游發版，見 [`comparison.md`](comparison.md#gpu-問題的上游狀態)。
