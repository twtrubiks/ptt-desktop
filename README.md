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

![畫面](https://cdn.imgpile.com/f/jHdKrEi_xl.png)

## 設計目標：輕量、快

這個專案的首要目標是輕量與快，作法都可以在程式碼裡驗證：

- **系統 WebView**：Tauri 2 使用系統內建 WebView，不像 Electron 打包整個
  Chromium——Linux 主程式執行檔實測約 14 MB
- **沒有前端框架**：純 TypeScript 約 950 行 + xterm.js；Rust 側僅 16 行
  plugin 註冊樣板，不含業務邏輯
- **不做多餘的事**：固定 80×24、`scrollback: 0`；閃爍動畫只在畫面上有
  閃爍字且視窗可見時才跑 timer，其餘時間完全靜止

## 特色

- **原汁原味**：固定 80×24 字元格、完整 ANSI 色彩，不重新排版；
  視窗縮放只調字型大小，排版永遠不跑版
- **自動登入**：帳密寫在設定檔即自動登入到主功能表；
  任一步異常（密碼不對、逾時）立即交還手動，絕不自動重試（避免鎖帳號）
- **自訂快捷鍵**：按鍵 → 送出序列，例如按 `F3` 直跳八卦板
- **防閒置踢出**：每 60 秒送 telnet NOP 保持連線
- **斷線重連**：斷線後按 Enter 即重連並重跑自動登入
- **網址好點**：文章內網址常駐底線、點擊用系統瀏覽器開啟、
  圖片連結滑鼠停留浮動預覽（支援直連圖檔與 imgur）
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
| `fontFamily` / `fontSize` | 終端字型（預設 Noto Sans Mono CJK TC / 18） |
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
- 跨行的網址（被 80 欄硬換行切開）無法點擊，底線也只涵蓋單行部分
- 登入畫面的 Big5 ANSI 藝術若用半字元上色，可能少數破圖（僅限登入前畫面）

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