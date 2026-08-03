<p align="center">
  <img src="assets/logo.png" alt="ptt-desktop" width="360">
</p>

# ptt-desktop

[![CI](https://github.com/twtrubiks/ptt-desktop/actions/workflows/ci.yml/badge.svg)](https://github.com/twtrubiks/ptt-desktop/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/twtrubiks/ptt-desktop)](https://github.com/twtrubiks/ptt-desktop/releases)
[![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB.svg)](https://tauri.app/)
[![xterm.js](https://img.shields.io/badge/xterm.js-6-2C2C32.svg)](https://xtermjs.org/)
[![TypeScript](https://img.shields.io/badge/TypeScript-7-3178c6.svg)](https://www.typescriptlang.org/)
[![Rust](https://img.shields.io/badge/Rust-dea584.svg)](https://www.rust-lang.org/)

輕量的 PTT 桌面客戶端（Tauri 2 + xterm.js），保留原汁原味的 80×24 BBS
終端畫面，透過 PTT 官方 WebSocket 端點 `wss://ws.ptt.cc/bbs` 連線。
支援 Linux / macOS / Windows。

![畫面](https://cdn.imgpile.com/f/8zn1iB5_xl.png)

## 設計目標：輕量、快

這個專案的首要目標是輕量與快，作法都可以在程式碼裡驗證：

- **系統 WebView**：Tauri 2 使用系統內建 WebView，不像 Electron 打包整個 Chromium
- **沒有前端框架**：純 TypeScript + xterm.js；Rust 側僅 plugin 註冊樣板，不含業務邏輯
- **不做多餘的事**：固定 80×24、`scrollback: 0`；閃爍動畫只在畫面上有
  閃爍字且視窗可見時才跑 timer，其餘時間完全靜止

## 特色

- **原汁原味**：固定 80×24 字元格、完整 ANSI 色彩，不重新排版；
  視窗縮放只調字型大小，文字不會重新斷行
- **自動登入**：帳密寫在設定檔即自動登入到主功能表；
  任一步異常（密碼不對、逾時）立即交還手動，絕不自動重試（避免鎖帳號）
- **自訂快捷鍵**：按鍵 → 送出序列，例如按 `F3` 直跳八卦板
- **防閒置踢出**：每 60 秒送 telnet NOP 保持連線
- **斷線重連**：斷線後按 Enter 即重連並重跑自動登入
- **網址好點**：文章內網址常駐底線、點擊用系統瀏覽器開啟、
  圖片連結滑鼠停留浮動預覽（支援直連圖檔）
- **簽名檔閃爍字**：補上 xterm.js 未實作的 ANSI 閃爍動畫

## 下載安裝

到 [Releases](https://github.com/twtrubiks/ptt-desktop/releases) 下載對應平台的安裝檔：

| 平台 | 檔案 | 備註 |
|---|---|---|
| Linux | `.AppImage` / `.deb` / `.rpm` | AppImage 需先 `chmod +x`；若 Wayland 下畫面異常，可退回 `GDK_BACKEND=x11 ./xxx.AppImage` 執行（此模式下中文輸入不可用） |
| macOS | `.dmg`（universal） | 未經 Apple 公證，首次開啟請對 App 按右鍵 →「打開」，或執行 `xattr -cr "/Applications/ptt-desktop.app"` |
| Windows | `.msi` / `.exe` | 未簽章，SmartScreen 警告時點「更多資訊」→「仍要執行」 |

## 設定檔

`~/.config/ptt-desktop/config.json`，範本見 [`config.example.json`](config.example.json)：

```bash
mkdir -p ~/.config/ptt-desktop
cp config.example.json ~/.config/ptt-desktop/config.json
chmod 600 ~/.config/ptt-desktop/config.json   # 密碼為明文，務必收緊權限
```

| 欄位 | 說明 |
|---|---|
| `username` / `password` | PTT 帳密（留空則跳過自動登入，全手動） |
| `kickOtherLogins` | 「您想刪除其他重複登入的連線嗎」的回答，預設 `n` |
| `fontFamily` / `fontSize` | 終端字型（預設 PTT Square Mono / 18——app 內建、彩繪比例已修正的等寬字型；可改用系統字型，但彩繪會回到縱向拉長） |
| `lineHeight` | 行高倍率，預設 `1`（範圍 1–2，超出自動夾住）。預設是彩繪方塊完全無縫的正方格；**覺得長文字太擠可調 `1.08`**（行距有感、橫縫幾乎看不出），想更寬鬆可往 `1.15`～`1.2`，代價是彩繪列間橫縫按比例變明顯 |
| `hotkeys` | 按鍵 → 送出序列，如 `"F3": "sgossiping\r"` 直跳八卦板 |

按鍵名稱使用 [KeyboardEvent.key](https://developer.mozilla.org/en-US/docs/Web/API/KeyboardEvent/key) 值。
沒有設定檔也能用，只是每次要手動登入。

保留的本地鍵（優先於自訂快捷鍵，不會送給 PTT）：

| 按鍵 | 功能 |
|---|---|
| `Ctrl+R` | 手動重連 |
| `F11` | 全螢幕切換 |

## 已知限制

- 密碼以明文存於設定檔（同 `~/.netrc` 等級的取捨），請自行 `chmod 600`
- 畫面看起來縱向拉長、彩繪比例不對（方塊變長方形）？多半是設定檔的
  `fontFamily` 指向系統字型（如 Noto Sans Mono CJK TC）——改回
  `"PTT Square Mono"`（或刪除該欄位用預設值）即可恢復正方比例
- 「行距」與「彩繪無縫」本質互斥：方塊字形（U+2588 等）字面固定 1em×1em，
  cell 只要高過 1em，方塊列間就露出等比例的橫縫。`lineHeight` 是這條光譜上
  的旋鈕，讀長文與看彩繪各有甜蜜點，無法同時最佳。預設 `1` 站在彩繪保真
  這端；**覺得字擠、行距太密，就把 `lineHeight` 調到 `1.08`**——實測此值
  行距改善有感、橫縫仍幾乎察覺不到，且一般視窗下是寬度受限，加行距不會
  縮小字級，等於把原本閒置的上下黑邊換成行距
- 跨行的網址（被 80 欄硬換行切開）無法點擊，底線也只涵蓋單行部分
- PTT 彩繪（ANSI art，如登入橫幅、主功能表的雨字、彩繪文章）：
  切割亂碼、壓縮條紋、色塊黑縫、縱向拉長、雙色字左右異色、
  分隔線虛線化與方塊梳齒（webgl 自繪寬字只畫半格）、
  黑底彩繪浮灰黑拼布與整體偏色（xterm 預設 Tango 色盤，已改用
  term.ptt.cc 同款傳統 VGA 色盤）、亮色方塊彩繪被 1px 暗線切成磁磚
  （捆綁字型無 Bold face，合成粗體撐破字形圖集；已改為不套字重，
  BBS 的 SGR 1 本來就只表示亮色）皆已修正；
  動態看板反覆覆寫半字造成的蟲蛀缺塊與行尾錯位，由 byte 網格 shadow
  補寫修復。複製含雙色字的畫面時會自動還原為原字。
  根因與修法見[架構文件](docs/architecture.md)

## 開發

```bash
npm install
npm run tauri dev    # 開發模式（首次會編譯 Rust，較久）
npm test             # vitest 單元測試
npm run quality      # format / lint / stylelint / clippy 全套檢查
npm run tauri build  # 打包
```

系統需求：Node.js 24、Rust toolchain；Linux 另需 webkit2gtk-4.1
（其餘系統依賴見 [`.github/workflows/release.yml`](.github/workflows/release.yml)）。

架構與實作細節（WebSocket 端點、telnet 濾層、Big5→UTF-8 切換、
自動登入狀態機、overlay 渲染）見 [技術文件](docs/architecture.md)。

## AI 開發流程（Harness）

本專案用 Claude Code 開發，工作流固定在 `.claude/`：
`/harness-plan` 把需求拆成 `Plans.md` 任務 → `/harness-work` 實作並跑
preflight（quality / build / test）→ 獨立 reviewer 審查通過才 commit。
安全 invariant（自動登入絕不重試、帳密不進 log）與「輕量、不加依賴」
的取捨標準寫在規範裡，由 reviewer 逐條把關。

完整說明見 [Harness 使用說明](docs/harness-guide.md)。

## License

[GPL-3.0](LICENSE)

## Donation

文章都是我自己研究內化後原創，如果有幫助到您，也想鼓勵我的話，歡迎請我喝一杯咖啡 :laughing:

綠界科技ECPAY ( 不需註冊會員 )

![alt tag](https://payment.ecpay.com.tw/Upload/QRCode/201906/QRCode_672351b8-5ab3-42dd-9c7c-c24c3e6a10a0.png)

[贊助者付款](http://bit.ly/2F7Jrha)

歐付寶 ( 需註冊會員 )

![alt tag](https://i.imgur.com/LRct9xa.png)

[贊助者付款](https://payment.opay.tw/Broadcaster/Donate/9E47FDEF85ABE383A0F5FC6A218606F8)

## 贊助名單

[贊助名單](https://github.com/twtrubiks/Thank-you-for-donate)
