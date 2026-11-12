//! GPUI 視圖：組裝連線／telnet／網格／自動登入／快捷鍵，並把 80×24 網格畫到畫面上。
//!
//! 畫面是自己畫的（canvas 元素 + 低階 paint API），不是終端機元件：
//! - 每格背景一個 quad（同列同色合併），字形逐格／逐對以格子原點錨定，永不漂移
//! - 寬字左右半各自帶色：同色畫一次，異色以 content mask 各畫半邊（雙色字原生支援）
//! - 方塊／三角（U+2580–259F、◢◣◤◥）以幾何填滿整格，拼接處無縫
//! - 閃爍字（SGR 5）與游標由 500ms tick 驅動；無閃爍字、視窗非作用中、無自動登入、
//!   無提示訊息且沒在等舊連線關閉時 tick 停止
//! - 重連（Ctrl+R／斷線後 Enter）經 `reconnect.rs` 節流：30 秒登入冷卻、線上時要按兩次，
//!   且等舊連線確定關閉才開新的；提示訊息浮在畫面底部，不寫進網格
//! - 關視窗或按 Ctrl+Shift+Q 時連線還在就先正常關閉（等舊連線回報 Closed，最多 `CLOSE_WAIT`）才結束程式，
//!   等待中再關一次視窗就直接結束
//! - 自動登入進行中使用者自己按鍵／貼上／輸入法送字就立刻交還手動（密碼已通過則不提示）；
//!   已登入後的逾時交還只浮出提示，只有登入前的中止（密碼不對等）才寫進網格
//! - 網址常駐底線、點擊開系統瀏覽器；停在圖片連結 250ms 後浮動預覽（`image_preview.rs` 抓圖）
//! - 滑鼠拖曳選取文字：放開即複製到剪貼簿；Ctrl+Shift+C 複製、Ctrl+Shift+V／中鍵貼上
//! - 鍵盤：特殊鍵／Ctrl 組合在 key_down 編碼送出；可列印字元與 IME 提交文字走
//!   `EntityInputHandler::replace_text_in_range`（GPUI 對未攔截的 keydown 會把 key_char
//!   交給 input handler，中文輸入法也走同一條路，統一在此送出）
//! - 帳號設定對話框（Ctrl+Shift+S）：開著時按鍵、文字、貼上都進表單（`account_form.rs`），
//!   不送給 PTT；畫面是把表單的 ANSI 文字列蓋到這一幀的格子上，與 PTT 畫面同一套繪製

use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use gpui::{
    canvas, div, fill, img, point, prelude::*, px, rgb, size, App, Bounds, ClipboardItem,
    ContentMask, Context, CursorStyle, ElementInputHandler, Entity, EntityInputHandler,
    FocusHandle, Focusable, Font, FontFallbacks, Hsla, Image, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Point, Render, RenderImage,
    ScrollDelta, ScrollWheelEvent, SharedString, Size, Task, TextRun, UTF16Selection, Window,
    WindowTextSystem,
};

use crate::account_form::{self, AccountForm, FormAction};
use crate::auto_login::{AutoLogin, Credentials, Event as LoginEvent, Reason};
use crate::config::{self, Config, LoadResult};
use crate::grid::{Cell, Grid, COLS, ROWS};
use crate::hotkeys::{Hotkeys, KeyAction, Mods};
use crate::image_preview;
use crate::net::{self, ConnHandle, NetEvent, PTT_WS_URL};
use crate::palette::{cell_colors, ANSI_16};
use crate::reconnect::{ceil_secs, Decision, ReconnectGuard, CONFIRM_WINDOW, LOGIN_COOLDOWN};
use crate::telnet::TelnetFilter;
use crate::urls::{scan_row, LinkSpan};
use crate::wcwidth::wcwidth;

const TICK: Duration = Duration::from_millis(500);
const MARGIN: f32 = 8.0;
/// 滑鼠掃過圖片連結不觸發，停住才載圖
const PREVIEW_DELAY: Duration = Duration::from_millis(250);
/// 預覽與滑鼠、與視窗邊緣各留的間距
const PREVIEW_EDGE_MARGIN: f32 = 12.0;
/// 等舊連線回報關閉的上限（`net.rs` 的關閉握手上限 2 秒要比它短）。確認重連時逾時就放棄
/// 等待、交還使用者按 Enter，不自動連；關視窗時逾時就直接結束程式
const CLOSE_WAIT: Duration = Duration::from_secs(3);
/// 底部提示訊息的顯示時間（與重連確認窗同長，訊息消失＝確認失效）
const NOTICE_DURATION: Duration = CONFIRM_WINDOW;
/// 沒設定帳密時，啟動後提示帳號設定快捷鍵的顯示時間
const ACCOUNT_HINT_DURATION: Duration = Duration::from_secs(8);
/// 帳號設定對話框在網格上的位置（置中）
const FORM_TOP: usize = (ROWS - account_form::HEIGHT) / 2;
const FORM_LEFT: usize = (COLS - account_form::WIDTH) / 2;
/// 滾輪一格在 gpui 是 3 行（X11、Wayland 皆同；gpui 的 `SCROLL_LINES` 不公開，只能照抄）
const WHEEL_LINES_PER_NOTCH: f32 = 3.0;
/// 一個滾輪事件最多換成幾次方向鍵，觸控板一次甩很大或座標跳動時不會一口氣送出一長串
const WHEEL_MAX_STEPS: i32 = 5;

/// 把滾輪量累積成方向鍵次數：滑鼠滾一格送一次；觸控板的像素量先換算成行，
/// 滑過與一格等量的距離才送一次。不足一格的餘量留到下一個事件
#[derive(Default)]
struct WheelAccum {
    lines: f32,
}

impl WheelAccum {
    /// `lines` 正值是往上捲（gpui 的慣例）。回傳要送幾次 ↑（正）或 ↓（負）
    fn feed(&mut self, lines: f32) -> i32 {
        // 換方向就丟掉反方向的餘量，反轉時第一格就有反應
        if lines * self.lines < 0.0 {
            self.lines = 0.0;
        }
        self.lines += lines;
        let steps = (self.lines / WHEEL_LINES_PER_NOTCH).trunc();
        self.lines -= steps * WHEEL_LINES_PER_NOTCH;
        let steps = steps as i32;
        if steps.abs() > WHEEL_MAX_STEPS {
            self.lines = 0.0;
            return steps.signum() * WHEEL_MAX_STEPS;
        }
        steps
    }

    fn reset(&mut self) {
        self.lines = 0.0;
    }
}

/// 字型校準：以 100px 量一次 █ 的進距與字面框，之後所有字級按比例換算
#[derive(Clone, Copy, Debug)]
struct FontCalib {
    /// █ 的進距 / 字級（CJK 等寬字型為 1.0）
    advance_ratio: f32,
    /// █ 字面框中心在基線上方的高度 / 字級（Noto CJK 約 0.38）
    center_ratio: f32,
}

#[derive(Clone, Copy, Debug)]
struct Layout {
    origin: Point<Pixels>,
    cell_w: Pixels,
    cell_h: Pixels,
    font_size: Pixels,
}

impl Layout {
    fn cell_bounds(&self, row: usize, col: usize, width: usize) -> Bounds<Pixels> {
        Bounds::new(
            point(
                self.origin.x + self.cell_w * col as f32,
                self.origin.y + self.cell_h * row as f32,
            ),
            size(self.cell_w * width as f32, self.cell_h),
        )
    }

    fn cell_at(&self, p: Point<Pixels>) -> Option<(usize, usize)> {
        let x = f32::from(p.x - self.origin.x);
        let y = f32::from(p.y - self.origin.y);
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let col = (x / f32::from(self.cell_w)) as usize;
        let row = (y / f32::from(self.cell_h)) as usize;
        (row < ROWS && col < COLS).then_some((row, col))
    }
}

/// 一幀要畫的東西（從 view 拷出，交給 'static 的 paint 閉包）
struct Frame {
    cells: Vec<Cell>,
    cursor: (usize, usize),
    cursor_visible: bool,
    cursor_on: bool,
    blink_on: bool,
    focused: bool,
    selection: Option<((usize, usize), (usize, usize))>,
    /// 輸入法組字中的預編輯文字（空字串＝沒有）
    preedit: String,
    layout: Layout,
    font: Font,
    calib: FontCalib,
    palette: [Hsla; 16],
}

pub struct PttView {
    focus_handle: FocusHandle,
    config: Config,
    hotkeys: Hotkeys,
    font: Font,
    calib: Option<FontCalib>,
    layout: Layout,

    grid: Grid,
    telnet: TelnetFilter,
    auto_login: Option<AutoLogin>,
    conn: Option<ConnHandle>,
    generation: u64,
    connected: bool,
    connecting: bool,
    awaiting_reconnect: bool,
    /// 重連節流（登入冷卻＋線上二次確認）
    reconnect_guard: ReconnectGuard,
    /// 已確認重連、送出關閉，正在等舊連線回報 Closed（之後才開新連線）
    closing_since: Option<Instant>,
    /// 使用者關了視窗、正在等舊連線關閉：舊連線一回報 Closed（或 `CLOSE_WAIT` 逾時）就結束程式，
    /// 不開新連線
    exiting: bool,
    /// 浮在畫面底部的提示訊息與其到期時間（不寫進網格，免得弄亂 PTT 畫面）
    notice: Option<(SharedString, Instant)>,
    /// 帳號設定對話框（開著時輸入都進表單，不送給 PTT）
    account_form: Option<AccountForm>,

    blink_on: bool,
    cursor_on: bool,
    ticking: bool,
    hover_url: Option<String>,
    unknown_seen: usize,

    /// 拖曳選取：按下的格子（錨點）與目前範圍（錨點 → 游標所在格，含兩端）
    drag_anchor: Option<(usize, usize)>,
    selection: Option<((usize, usize), (usize, usize))>,
    mouse_pos: Point<Pixels>,
    wheel: WheelAccum,

    /// 輸入法預編輯文字。Wayland text-input-v3 下組字中的注音／候選字由應用程式自己畫在
    /// 游標位置（輸入法視窗只列候選字），提交或取消時清空。它不進網格，不送給 PTT。
    preedit: String,

    /// 圖片預覽：已解碼的圖與滑鼠停住時的位置；序號讓遲到的抓圖結果作廢
    preview: Option<(Arc<RenderImage>, Point<Pixels>)>,
    /// 已收掉、等下一次 render 從 GPU 圖集移除的預覽圖。gpui 不會自動釋放畫過的圖，
    /// 不移除的話每張不同的圖會一直佔著解碼後的記憶體與貼圖；移除要 Window，所以延到 render
    stale_previews: Vec<Arc<RenderImage>>,
    preview_seq: u64,
    _preview_task: Option<Task<()>>,

    _net_task: Option<Task<()>>,
}

impl PttView {
    pub fn new(loaded: LoadResult, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let LoadResult { config, error } = loaded;
        let font = Font {
            family: config.font_family.clone().into(),
            fallbacks: Some(FontFallbacks::from_fonts(vec![
                "Noto Sans Mono CJK TC".into(),
                "Noto Sans CJK TC".into(),
                "monospace".into(),
            ])),
            ..gpui::font(config.font_family.clone())
        };
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);
        cx.observe_window_activation(window, |this, window, cx| {
            this.ensure_tick(window, cx);
            cx.notify();
        })
        .detach();
        let view = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            view.update(cx, |view, cx| view.request_exit(window, cx))
                .unwrap_or(true)
        });

        let mut this = Self {
            focus_handle,
            hotkeys: Hotkeys::new(&config.hotkeys),
            font,
            calib: None,
            layout: Layout {
                origin: point(px(0.), px(0.)),
                cell_w: px(10.),
                cell_h: px(20.),
                font_size: px(20.),
            },
            config,
            grid: Grid::new(),
            telnet: TelnetFilter::new(),
            auto_login: None,
            conn: None,
            generation: 0,
            connected: false,
            connecting: false,
            awaiting_reconnect: false,
            reconnect_guard: ReconnectGuard::default(),
            closing_since: None,
            exiting: false,
            notice: None,
            account_form: None,
            blink_on: true,
            cursor_on: true,
            ticking: false,
            hover_url: None,
            unknown_seen: 0,
            drag_anchor: None,
            selection: None,
            mouse_pos: point(px(0.), px(0.)),
            wheel: WheelAccum::default(),
            preedit: String::new(),
            preview: None,
            stale_previews: Vec::new(),
            preview_seq: 0,
            _preview_task: None,
            _net_task: None,
        };
        if let Some(err) = error {
            // 設定檔壞掉不能默默用預設值連線（使用者只會看到「沒有自動登入」卻查不到原因）；
            // 借用斷線重連機制暫停在此，訊息看完按 Enter 再連
            this.awaiting_reconnect = true;
            this.grid.write_text(&format!(
                "\x1b[1;33m[設定檔解析失敗：{err}]\r\n[改用預設值繼續（不會自動登入），請檢查 ~/.config/ptt-desktop/config.json，按 Enter 連線]\x1b[m"
            ));
        } else {
            this.connect(cx);
            if this.config.username.is_empty() || this.config.password.is_empty() {
                this.notice = Some((
                    "按 Ctrl+Shift+S 可設定帳號，之後自動登入".into(),
                    Instant::now() + ACCOUNT_HINT_DURATION,
                ));
            }
        }
        this.ensure_tick(window, cx);
        this
    }

    // ---- 連線流程 ----

    fn connect(&mut self, cx: &mut Context<Self>) {
        if self.connecting {
            return;
        }
        self.connecting = true;
        self.generation += 1;
        let generation = self.generation;
        self.reconnect_guard.mark_connect(Instant::now());
        self.grid.reset();
        self.telnet.reset();
        self.connected = false;
        self.awaiting_reconnect = false;
        self.closing_since = None;
        self.notice = None;
        self.grid
            .write_text(&format!("連線 {PTT_WS_URL} 中...\r\n"));
        // 先啟動自動登入再連線：歡迎畫面可能緊接 Connected 到達
        self.start_auto_login();
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<NetEvent>();
        self.conn = Some(net::connect(tx));
        self._net_task = Some(cx.spawn(async move |this, cx| {
            while let Some(ev) = rx.next().await {
                let alive = this
                    .update(cx, |view, cx| view.on_net_event(generation, ev, cx))
                    .is_ok();
                if !alive {
                    break;
                }
            }
        }));
        self.connecting = false;
        cx.notify();
    }

    /// 使用者要求重連（Ctrl+R，或斷線後按 Enter）。被冷卻或確認擋下時只顯示提示，
    /// 不排程、不自動連——時間到了仍要使用者自己再按
    fn request_reconnect(&mut self, cx: &mut Context<Self>) {
        if self.connecting || self.closing_since.is_some() {
            return;
        }
        let online = self.conn.is_some() && !self.awaiting_reconnect;
        match self.reconnect_guard.request(Instant::now(), online) {
            Decision::Cooldown(left) => self.show_notice(format!(
                "距上次連線不到 {} 秒，請 {} 秒後再按（避免 PTT 判定頻繁登入）",
                LOGIN_COOLDOWN.as_secs(),
                ceil_secs(left)
            )),
            Decision::NeedConfirm => self.show_notice(format!(
                "{} 秒內再按一次 Ctrl+R 確認重連（目前的連線不會先登出）",
                CONFIRM_WINDOW.as_secs()
            )),
            Decision::Go if online => self.begin_close(),
            Decision::Go => self.connect(cx),
        }
        cx.notify();
    }

    /// 線上重連的前半段：送出關閉後等舊連線回報 Closed，才在 `on_net_event` 開新連線，
    /// 新舊連線不會重疊（原版 `main.ts` 的 reconnect 也是先 await disconnect）
    fn begin_close(&mut self) {
        if let Some(al) = self.auto_login.as_mut() {
            al.stop();
        }
        self.auto_login = None;
        if let Some(conn) = &self.conn {
            conn.close();
        }
        self.closing_since = Some(Instant::now());
        self.show_notice("正在關閉目前的連線…".to_string());
    }

    /// Ctrl+Shift+Q：與關視窗走同一條路，連線關好才結束；等待中再按一次就直接結束
    fn quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.request_exit(window, cx) {
            cx.quit();
        }
    }

    /// 使用者要關視窗（或按了結束快捷鍵）。連線還在就先送出關閉、擋下這次關閉，等舊連線回報 Closed 才結束程式，
    /// 讓 `net.rs` 做完 WebSocket 關閉握手；否則程式一結束，連線就被硬切斷。
    /// 已經在等了又關一次就直接放行。回傳 true＝視窗現在就關
    fn request_exit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.conn.is_none() || self.exiting {
            return true;
        }
        self.exiting = true;
        if self.closing_since.is_none() {
            self.begin_close();
        }
        self.show_notice("正在關閉連線，關好就結束（再關一次可立即結束）".to_string());
        self.ensure_tick(window, cx);
        cx.notify();
        false
    }

    /// 舊連線在 `CLOSE_WAIT` 內沒回報關閉（例如卡在握手）：放棄等待並交還使用者。
    /// 丟掉 handle 後執行緒一回到迴圈就會自行關閉；這裡不自動開新連線
    fn abandon_close(&mut self) {
        self.closing_since = None;
        self.conn = None;
        self._net_task = None; // 舊連線的殘留事件隨 task 一起作廢
        self.connected = false;
        self.awaiting_reconnect = true;
        self.notice = None;
        self.grid.write_text(&format!(
            "\r\n\x1b[1;31m[舊連線未在 {} 秒內關閉，已放棄等待，按 Enter 重連]\x1b[m",
            CLOSE_WAIT.as_secs()
        ));
    }

    fn show_notice(&mut self, text: String) {
        self.notice = Some((text.into(), Instant::now() + NOTICE_DURATION));
    }

    // ---- 帳號設定對話框 ----

    /// 以目前的設定值開啟對話框。不動連線與自動登入：對話框開著時按鍵不會送給 PTT
    fn open_account_form(&mut self) {
        self.account_form = Some(AccountForm::new(
            &self.config.username,
            &self.config.password,
            &self.config.kick_other_logins,
        ));
        self.preedit.clear();
        self.selection = None;
        self.drag_anchor = None;
        self.hover_url = None;
        self.clear_preview();
    }

    /// 寫回設定檔並更新記憶體中的帳密，下次連線才生效：這裡不重連也不啟動自動登入，
    /// 連線時機仍由使用者決定（不繞過冷卻、不自動重試）。寫檔失敗就留在對話框顯示原因
    fn save_account_form(&mut self) {
        let Some(form) = self.account_form.as_mut() else {
            return;
        };
        if let Err(e) = config::save_account(form.username(), form.password(), form.kick_answer()) {
            form.set_error(e);
            return;
        }
        self.config.username = form.username().to_string();
        self.config.password = form.password().to_string();
        self.config.kick_other_logins = form.kick_answer().to_string();
        self.account_form = None;
        let saved = if self.config.username.is_empty() || self.config.password.is_empty() {
            "已儲存。帳號或密碼留空，不會自動登入"
        } else if self.awaiting_reconnect {
            "已儲存，按 Enter 連線即自動登入"
        } else {
            "已儲存，下次連線起自動登入（Ctrl+R 重連）"
        };
        self.show_notice(saved.to_string());
    }

    /// 對話框開著時的按鍵。全螢幕、貼上與結束程式照常；其餘不是表單要的鍵一律吃掉，不送給 PTT。
    /// 沒帶修飾鍵的可列印字元放行，GPUI 會把它交給 input handler 再進表單
    fn on_form_key(
        &mut self,
        action: KeyAction,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ks = &ev.keystroke;
        match action {
            KeyAction::ToggleFullscreen => {
                window.toggle_fullscreen();
                cx.stop_propagation();
                return;
            }
            KeyAction::Paste => {
                self.paste(cx.read_from_clipboard());
                cx.stop_propagation();
                cx.notify();
                return;
            }
            KeyAction::Quit => {
                self.quit(window, cx);
                cx.stop_propagation();
                return;
            }
            _ => {}
        }
        if ks.modifiers.control || ks.modifiers.alt || ks.modifiers.platform {
            cx.stop_propagation();
            return;
        }
        let Some(form) = self.account_form.as_mut() else {
            return;
        };
        match form.key(&ks.key, ks.modifiers.shift) {
            FormAction::Ignored => return,
            FormAction::Handled => {}
            FormAction::Cancel => self.account_form = None,
            FormAction::Save => self.save_account_form(),
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn start_auto_login(&mut self) {
        if self.config.username.is_empty() || self.config.password.is_empty() {
            return;
        }
        let mut al = AutoLogin::new(Credentials {
            username: self.config.username.clone(),
            password: self.config.password.clone(),
            kick_other_logins: self.config.kick_other_logins.clone(),
        });
        al.start(Instant::now());
        self.auto_login = Some(al);
    }

    fn on_net_event(&mut self, generation: u64, ev: NetEvent, cx: &mut Context<Self>) {
        if generation != self.generation {
            return;
        }
        match ev {
            NetEvent::Connected => {
                self.connected = true;
            }
            NetEvent::Binary(data) => {
                let filtered = self.telnet.filter(&data);
                if filtered.is_empty() {
                    return;
                }
                dump_raw(&filtered);
                // 位元組層先餵：偵測到 UTF-8 密碼提示會先切 utf-8、Big5「密碼不對」會先撥回
                // big5，本 chunk 隨後就以正確編碼進網格，亂碼不會先上畫面
                if let Some(al) = self.auto_login.as_mut() {
                    let events = al.feed(&filtered, Instant::now());
                    self.handle_login_events(events);
                }
                self.grid.feed(&filtered);
                let responses = self.grid.take_responses();
                if !responses.is_empty() {
                    self.send_bytes(&responses); // DSR 游標位置回報（xterm.js 亦自動回覆）
                }
                if self.grid.unknown_sequences.len() > self.unknown_seen {
                    self.unknown_seen = self.grid.unknown_sequences.len();
                    eprintln!("[grid] 未實作的終端序列：{:?}", self.grid.unknown_sequences);
                }
                self.hover_url = None;
                self.selection = None; // 畫面變了，選取範圍對不上原文字
                self.clear_preview();
                cx.notify();
            }
            NetEvent::Closed(reason) => {
                if let Some(al) = self.auto_login.as_mut() {
                    al.stop();
                }
                self.auto_login = None;
                self.connected = false;
                self.conn = None;
                if self.exiting {
                    cx.quit();
                    return;
                }
                if self.closing_since.is_some() {
                    // 使用者已確認重連，舊連線確定關閉了才開新的
                    self.connect(cx);
                    return;
                }
                self.awaiting_reconnect = true;
                self.grid.write_text(&format!(
                    "\r\n\x1b[1;31m[已斷線：{reason}，按 Enter 重連]\x1b[m"
                ));
                cx.notify();
            }
        }
    }

    fn handle_login_events(&mut self, events: Vec<LoginEvent>) {
        for ev in events {
            match ev {
                LoginEvent::Send(text) => self.send_bytes(text.as_bytes()),
                LoginEvent::SwitchUtf8 => self.grid.set_utf8(true),
                // Big5「密碼不對」命中＝伺服器仍在 Big5（舊版），撥回讓錯誤畫面可讀；
                // 只切解碼不清畫面，登入畫面與錯誤訊息一起留著
                LoginEvent::ResetBig5 => self.grid.set_utf8(false),
                LoginEvent::Finished { reason, detail } => {
                    self.auto_login = None;
                    match reason {
                        Reason::Done => {}
                        // 已登入：PTT 不知道網格裡多寫的字，寫進去會一直殘留，只浮出提示
                        Reason::HandedOver => self.show_notice(
                            "已登入，自動登入沒等到主選單，接下來請手動操作".to_string(),
                        ),
                        // 登入前出錯：停在登入畫面，要使用者處理，寫進網格留著
                        Reason::Aborted => self.grid.write_text(&format!(
                            "\r\n\x1b[1;33m[自動登入中止（{}），請手動操作]\x1b[m",
                            detail.unwrap_or_else(|| "未知原因".to_string())
                        )),
                    }
                }
            }
        }
    }

    /// 自動登入進行中使用者自己送了鍵 → 立刻交還，不再替使用者按鍵。否則兩邊各送一個鍵，
    /// 多出的 Enter 會讓 PTT 略過主選單（輸入緩衝有鍵時不重畫）直接進分類看板。
    /// 密碼已通過時不提示：剩下的只是按掉歡迎畫面，使用者按的鍵就有同樣效果，沒有事要處理
    fn take_over_login(&mut self) {
        if let Some(al) = self.auto_login.take() {
            if !al.authenticated() {
                self.show_notice("偵測到按鍵，自動登入已停止，接下來請手動操作".to_string());
            }
        }
    }

    fn send_bytes(&self, bytes: &[u8]) {
        if self.awaiting_reconnect || self.closing_since.is_some() {
            return;
        }
        if let Some(conn) = &self.conn {
            conn.send(bytes);
        }
    }

    /// 文字依網格目前的編碼送出：登入後 PTT 在 UTF-8 模式；登入前（Big5）只會有 ASCII，
    /// 真有非 ASCII 也以 Big5 編碼，跟伺服器當下的解讀一致
    fn send_text(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.grid.is_utf8() {
            self.send_bytes(text.as_bytes());
        } else {
            let (bytes, _, _) = encoding_rs::BIG5.encode(text);
            self.send_bytes(&bytes);
        }
    }

    // ---- 選取／剪貼簿 ----

    fn copy_selection(&self, cx: &mut Context<Self>) {
        let Some((a, b)) = self.selection else {
            return;
        };
        let text = self.grid.text_in_range(a, b);
        if text.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        cx.write_to_primary(ClipboardItem::new_string(text));
    }

    /// 貼上：換行一律轉成 Enter（PTT 以 CR 當換行；貼多行文章時每行各送一個 Enter）
    fn paste(&mut self, item: Option<ClipboardItem>) {
        let Some(text) = item.and_then(|i| i.text()) else {
            return;
        };
        if text.is_empty() {
            return;
        }
        if let Some(form) = self.account_form.as_mut() {
            form.insert(&text);
            return;
        }
        self.take_over_login();
        let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
        self.send_text(&normalized);
    }

    // ---- 圖片預覽 ----

    fn clear_preview(&mut self) {
        self.preview_seq += 1;
        self._preview_task = None;
        if let Some((image, _)) = self.preview.take() {
            self.stale_previews.push(image);
        }
    }

    /// 停在圖片連結上：延遲後在背景執行緒抓圖，期間滑鼠移開即作廢
    fn schedule_preview(&mut self, url: String, cx: &mut Context<Self>) {
        self.clear_preview();
        let seq = self.preview_seq;
        let svg = cx.svg_renderer();
        self._preview_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PREVIEW_DELAY).await;
            let (tx, rx) = futures::channel::oneshot::channel();
            std::thread::Builder::new()
                .name("ptt-img".into())
                .spawn(move || {
                    // 自己解碼成 RenderImage，不交給 gpui 的 asset 快取（`img(Arc<Image>)` 會把
                    // 每張解碼結果永久留在 App 裡，收掉預覽也不釋放）
                    let decoded = image_preview::fetch(&url).and_then(|(format, bytes)| {
                        Image::from_bytes(format, bytes)
                            .to_image_data(svg)
                            .map_err(|e| format!("解碼失敗：{e}"))
                    });
                    let _ = tx.send(decoded);
                })
                .ok();
            let fetched = match rx.await {
                Ok(Ok(image)) => image,
                Ok(Err(e)) => {
                    log::info!("[preview] 抓圖失敗：{e}");
                    return;
                }
                Err(_) => return,
            };
            let _ = this.update(cx, |view, cx| {
                if view.preview_seq == seq {
                    view.preview = Some((fetched, view.mouse_pos));
                    cx.notify();
                }
            });
        }));
    }

    // ---- tick（閃爍、游標、登入逾時）----

    fn needs_tick(&self, window: &Window) -> bool {
        self.auto_login.is_some()
            || window.is_window_active()
            || self.grid.has_blink()
            || self.notice.is_some()
            || self.closing_since.is_some()
    }

    fn ensure_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ticking || !self.needs_tick(window) {
            return;
        }
        self.ticking = true;
        cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(TICK).await;
            let keep_going = this
                .update_in(cx, |view, window, cx| view.tick(window, cx))
                .unwrap_or(false);
            if !keep_going {
                break;
            }
        })
        .detach();
    }

    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.blink_on = !self.blink_on;
        self.cursor_on = !self.cursor_on;
        let now = Instant::now();
        if let Some(al) = self.auto_login.as_mut() {
            let events = al.poll(now);
            self.handle_login_events(events);
        }
        if self.notice.as_ref().is_some_and(|(_, until)| now >= *until) {
            self.notice = None;
        }
        if self
            .closing_since
            .is_some_and(|since| now.saturating_duration_since(since) >= CLOSE_WAIT)
        {
            if self.exiting {
                cx.quit();
            } else {
                self.abandon_close();
            }
        }
        if !self.needs_tick(window) {
            // 停下來時要停在「文字可見」的相位
            self.blink_on = true;
            self.cursor_on = true;
            self.ticking = false;
            cx.notify();
            return false;
        }
        cx.notify();
        true
    }

    // ---- 輸入 ----

    fn on_key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let mods = Mods {
            ctrl: ks.modifiers.control,
            alt: ks.modifiers.alt,
            shift: ks.modifiers.shift,
            platform: ks.modifiers.platform,
        };
        let action = self
            .hotkeys
            .translate(&ks.key, ks.key_char.as_deref(), mods);
        if self.account_form.is_some() {
            self.on_form_key(action, ev, window, cx);
            self.ensure_tick(window, cx);
            return;
        }
        match action {
            KeyAction::Reconnect => {
                self.request_reconnect(cx);
                cx.stop_propagation();
            }
            KeyAction::ToggleFullscreen => {
                window.toggle_fullscreen();
                cx.stop_propagation();
            }
            KeyAction::Copy => {
                self.copy_selection(cx);
                cx.stop_propagation();
            }
            KeyAction::Paste => {
                self.paste(cx.read_from_clipboard());
                cx.stop_propagation();
            }
            KeyAction::Quit => {
                self.quit(window, cx);
                cx.stop_propagation();
            }
            KeyAction::AccountSettings => {
                self.open_account_form();
                cx.stop_propagation();
                cx.notify();
            }
            KeyAction::Send(bytes) => {
                if self.awaiting_reconnect {
                    if bytes == b"\r" {
                        self.request_reconnect(cx);
                    }
                } else {
                    self.take_over_login();
                    self.send_bytes(&bytes);
                }
                cx.stop_propagation();
            }
            KeyAction::Pass => {}
        }
        self.ensure_tick(window, cx);
    }

    fn url_at(&self, p: Point<Pixels>) -> Option<String> {
        let (row, col) = self.layout.cell_at(p)?;
        scan_row(self.grid.row_cells(row))
            .into_iter()
            .find(|s: &LinkSpan| col >= s.col && col < s.col + s.width)
            .map(|s| s.url)
    }

    /// 拖曳時座標夾進網格（拖出視窗邊緣仍能選到最邊緣的格子）
    fn cell_at_clamped(&self, p: Point<Pixels>) -> (usize, usize) {
        let lay = &self.layout;
        let x = f32::from(p.x - lay.origin.x) / f32::from(lay.cell_w);
        let y = f32::from(p.y - lay.origin.y) / f32::from(lay.cell_h);
        let col = (x.floor().max(0.0) as usize).min(COLS - 1);
        let row = (y.floor().max(0.0) as usize).min(ROWS - 1);
        (row, col)
    }

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.mouse_pos = ev.position;
        if self.account_form.is_some() {
            return; // 對話框開著：不選取、不預覽底下的畫面
        }
        if let (Some(anchor), Some(MouseButton::Left)) = (self.drag_anchor, ev.pressed_button) {
            let cur = self.cell_at_clamped(ev.position);
            let sel = (cur != anchor).then_some((anchor, cur));
            if sel != self.selection {
                self.selection = sel;
                cx.notify();
            }
            return;
        }
        let url = self.url_at(ev.position);
        if url != self.hover_url {
            match url.as_deref().and_then(image_preview::resolve_image_url) {
                Some(img_url) => self.schedule_preview(img_url, cx),
                None => self.clear_preview(),
            }
            self.hover_url = url;
            cx.notify();
        }
    }

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        if self.account_form.is_some() {
            return;
        }
        self.drag_anchor = Some(self.cell_at_clamped(ev.position));
        if self.selection.take().is_some() {
            cx.notify();
        }
    }

    fn on_mouse_up(&mut self, ev: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let dragged = self.drag_anchor.take().is_some();
        if self.selection.is_some() {
            // 放開即複製（終端機慣例），選取框留著直到下次點擊或畫面更新
            self.copy_selection(cx);
        } else if dragged {
            if let Some(url) = self.url_at(ev.position) {
                cx.open_url(&url);
            }
        }
    }

    fn on_middle_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.paste(cx.read_from_primary());
        if self.account_form.is_some() {
            cx.notify();
        }
    }

    /// 滾輪一格送一次 ↑／↓（PTT 不收滑鼠事件，只能翻成按鍵）。自動登入進行中不送：
    /// 滾輪容易誤觸，不該像按鍵那樣把登入打斷；未連線、等重連、關閉中、帳號設定對話框開著也不送
    fn on_scroll_wheel(&mut self, ev: &ScrollWheelEvent, _: &mut Window, _: &mut Context<Self>) {
        if self.account_form.is_some()
            || self.auto_login.is_some()
            || !self.connected
            || self.awaiting_reconnect
            || self.closing_since.is_some()
        {
            self.wheel.reset();
            return;
        }
        let lines = match ev.delta {
            ScrollDelta::Lines(d) => d.y,
            ScrollDelta::Pixels(d) => f32::from(d.y) / f32::from(self.layout.cell_h),
        };
        let steps = self.wheel.feed(lines);
        if steps == 0 {
            return;
        }
        let key: &[u8] = if steps > 0 { b"\x1b[A" } else { b"\x1b[B" };
        self.send_bytes(&key.repeat(steps.unsigned_abs() as usize));
    }

    // ---- 版面與字型 ----

    fn calibrate(&mut self, text_system: &WindowTextSystem) -> FontCalib {
        if let Some(c) = self.calib {
            return c;
        }
        let probe = px(100.);
        let font_id = text_system.resolve_font(&self.font);
        let advance_ratio = text_system
            .advance(font_id, probe, '█')
            .map(|s| f32::from(s.width) / f32::from(probe))
            .ok()
            .filter(|r| *r > 0.1)
            .unwrap_or(1.0);
        // 字面中心：Noto Sans Mono CJK 的 █ 字面為 (0,-120)–(1000,880)，中心在基線上方
        // 0.38em。cosmic-text 回報的 typographic bounds 在此量到 0.5（實機 2026-11-08），
        // 用它排版字會往下掉、底部被裁，故只記錄不採用；換字型若對不齊，調這個常數
        let measured = text_system
            .typographic_bounds(font_id, probe, '█')
            .map(|b| {
                ((f32::from(b.origin.y) + f32::from(b.size.height) / 2.0) / f32::from(probe)).abs()
            })
            .unwrap_or(f32::NAN);
        let center_ratio = 0.38;
        let calib = FontCalib {
            advance_ratio,
            center_ratio,
        };
        eprintln!(
            "[font] {} advance_ratio={advance_ratio:.3} center_ratio={center_ratio:.3} (measured {measured:.3})",
            self.font.family
        );
        self.calib = Some(calib);
        calib
    }

    fn compute_layout(&self, viewport: Size<Pixels>, scale: f32, calib: FontCalib) -> Layout {
        let lh = self.config.line_height;
        let avail_w = (f32::from(viewport.width) - 2.0 * MARGIN).max(80.0);
        let avail_h = (f32::from(viewport.height) - 2.0 * MARGIN).max(48.0);
        // 寬度受限或高度受限取小者；格寬對齊到裝置像素，相鄰格共用同一條邊界線
        let cw = (avail_w / COLS as f32).min(avail_h / (ROWS as f32 * 2.0 * lh));
        let cw_dev = (cw * scale).floor().max(1.0);
        let ch_dev = (cw_dev * 2.0 * lh).round().max(1.0);
        let cell_w = px(cw_dev / scale);
        let cell_h = px(ch_dev / scale);
        let total_w = cell_w * COLS as f32;
        let total_h = cell_h * ROWS as f32;
        let ox = ((f32::from(viewport.width - total_w) / 2.0) * scale).round() / scale;
        let oy = ((f32::from(viewport.height - total_h) / 2.0) * scale).round() / scale;
        Layout {
            origin: point(px(ox), px(oy)),
            cell_w,
            cell_h,
            // 全形字（1em 進距）恰好兩格寬
            font_size: px(f32::from(cell_w) * 2.0 / calib.advance_ratio),
        }
    }
}

impl Focusable for PttView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

// IME／可列印字元入口：GPUI 把未攔截 keydown 的 key_char 與輸入法提交文字都送到這裡
impl EntityInputHandler for PttView {
    fn text_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        (!self.preedit.is_empty()).then(|| 0..self.preedit.encode_utf16().count())
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.preedit.is_empty() {
            self.preedit.clear();
            cx.notify();
        }
    }

    fn replace_text_in_range(
        &mut self,
        _range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 提交：預編輯結束，文字送給 PTT
        if !self.preedit.is_empty() {
            self.preedit.clear();
            cx.notify();
        }
        if let Some(form) = self.account_form.as_mut() {
            form.insert(new_text); // 表單只收 ASCII，輸入法提交的中文在這裡被丟掉
            cx.notify();
            return;
        }
        if !new_text.is_empty() {
            self.take_over_login();
        }
        self.send_text(new_text);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range_utf16: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 組字中：Wayland 下輸入法不畫預編輯文字，由 paint_frame 畫在游標格
        if self.account_form.is_some() {
            return; // 表單不收中文，組字中的文字也不畫
        }
        if self.preedit != new_text {
            self.preedit = new_text.to_string();
            cx.notify();
        }
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // 輸入法候選窗跟著游標格；組字中則涵蓋整段預編輯文字，候選窗才不會蓋住它
        let (row, col) = self.grid.cursor();
        let width = preedit_cells(&self.preedit).max(1).min(COLS - col);
        Some(self.layout.cell_bounds(row, col, width))
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl Render for PttView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for image in self.stale_previews.drain(..) {
            let _ = window.drop_image(image); // 只是從圖集移除，圖不在圖集裡也無害
        }
        let text_system = window.text_system().clone();
        let calib = self.calibrate(&text_system);
        self.layout = self.compute_layout(window.viewport_size(), window.scale_factor(), calib);

        let mut cells = self.grid.cells().to_vec();
        let mut cursor = self.grid.cursor();
        let mut cursor_visible = self.grid.cursor_visible();
        if let Some(form) = &self.account_form {
            cursor = overlay_form(&mut cells, form);
            cursor_visible = true;
        }
        let frame = Frame {
            cells,
            cursor,
            cursor_visible,
            cursor_on: self.cursor_on,
            blink_on: self.blink_on,
            focused: self.focus_handle.is_focused(window),
            selection: self.selection,
            preedit: self.preedit.clone(),
            layout: self.layout,
            font: self.font.clone(),
            calib,
            palette: ANSI_16.map(|c| rgb(c).into()),
        };
        let entity: Entity<Self> = cx.entity();
        let focus_handle = self.focus_handle.clone();
        let cursor_style = if self.drag_anchor.is_some() {
            CursorStyle::IBeam
        } else if self.hover_url.is_some() {
            CursorStyle::PointingHand
        } else {
            CursorStyle::Arrow
        };

        div()
            .size_full()
            .bg(gpui::black())
            .track_focus(&self.focus_handle)
            .cursor(cursor_style)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::on_middle_up))
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
            .child(
                canvas(
                    move |_, _, _| {},
                    move |bounds, _, window, cx| {
                        window.handle_input(
                            &focus_handle,
                            ElementInputHandler::new(bounds, entity),
                            cx,
                        );
                        paint_frame(&frame, window, cx);
                    },
                )
                .size_full(),
            )
            .children(self.preview.as_ref().map(|(image, at)| {
                preview_element(image.clone(), *at, window.viewport_size(), self.preview_seq)
            }))
            .children(
                self.notice
                    .as_ref()
                    .map(|(text, _)| notice_element(text.clone(), &self.font, self.layout)),
            )
    }
}

/// 把帳號設定對話框蓋到這一幀的格子上（不動 PTT 的網格），回傳游標所在格。
/// 表單給的是 ANSI 文字列，餵進暫時的網格取出格子，寬字與顏色的處理跟 PTT 畫面完全一樣；
/// 只在對話框開著時每幀建一次
fn overlay_form(cells: &mut [Cell], form: &AccountForm) -> (usize, usize) {
    let mut scratch = Grid::new();
    scratch.set_utf8(true);
    for (i, line) in form.lines().iter().enumerate() {
        let row = FORM_TOP + i;
        scratch.write_text(&format!("\x1b[{};{}H{line}", row + 1, FORM_LEFT + 1));
        let start = row * COLS + FORM_LEFT;
        let end = start + account_form::WIDTH;
        cells[start..end].copy_from_slice(&scratch.cells()[start..end]);
    }
    let (row, col) = form.cursor();
    (FORM_TOP + row, FORM_LEFT + col)
}

/// 底部置中的提示訊息（重連冷卻／確認／關閉中）。浮在網格上、不寫進網格，
/// 線上時顯示也不會弄亂 PTT 畫面；字級跟網格走，視窗縮放時一起縮放
fn notice_element(text: SharedString, font: &Font, layout: Layout) -> impl IntoElement {
    div()
        .absolute()
        .left(px(0.))
        .right(px(0.))
        .bottom(layout.cell_h)
        .flex()
        .justify_center()
        .child(
            div()
                .font(font.clone())
                .text_size(layout.font_size * 0.8)
                .text_color(rgb(0x000000))
                .bg(rgb(0xffff55))
                .px(layout.cell_w)
                .py(layout.cell_h * 0.15)
                .child(text),
        )
}

/// 預覽的位置與大小 `(left, top, w, h)`：放在滑鼠左右較寬的一側，高度可用滿視窗，
/// 等比縮到放得下（不放大）；垂直以滑鼠為中心，再夾回視窗內。
/// 寬高自己算而不靠 `max_w`／`max_h`：taffy 對兩軸各自夾限、不保比例，直式圖會被縮得過小
fn preview_rect(
    image: (f32, f32),
    mouse: (f32, f32),
    viewport: (f32, f32),
) -> (f32, f32, f32, f32) {
    let (iw, ih) = (image.0.max(1.0), image.1.max(1.0));
    let (mx, my) = mouse;
    let (vw, vh) = viewport;
    let m = PREVIEW_EDGE_MARGIN;
    let on_right = mx < vw / 2.0;
    let avail_w = (if on_right { vw - mx } else { mx } - 2.0 * m).max(1.0);
    let avail_h = (vh - 2.0 * m).max(1.0);
    let scale = (avail_w / iw).min(avail_h / ih).min(1.0);
    let (w, h) = (iw * scale, ih * scale);
    let left = if on_right { mx + m } else { mx - m - w };
    let top = (my - h / 2.0).min(vh - m - h).max(m);
    (left, top, w, h)
}

/// 浮動預覽。`img` 要有 id，gpui 才會跨幀記住目前播到第幾格，GIF 才會動；id 綁 `seq` 讓
/// 每張圖各用一份狀態——共用的話，上一張的格數會帶進格數較少的下一張，`render_size`
/// 直接越界 panic
fn preview_element(
    image: Arc<RenderImage>,
    at: Point<Pixels>,
    viewport: Size<Pixels>,
    seq: u64,
) -> impl IntoElement {
    let size = image.size(0);
    let (left, top, w, h) = preview_rect(
        (size.width.0 as f32, size.height.0 as f32),
        (f32::from(at.x), f32::from(at.y)),
        (f32::from(viewport.width), f32::from(viewport.height)),
    );
    img(image)
        .id(("ptt-preview", seq as usize))
        .absolute()
        .left(px(left))
        .top(px(top))
        .w(px(w))
        .h(px(h))
}

/// 診斷用：`PTT_DUMP=<檔案>` 時把 telnet 濾層後的下行 bytes 原樣附加進該檔
/// （對應原專案的 dev-recorder；錄檔含個資，請放在 gitignore 的 recordings/ 下）
fn dump_raw(bytes: &[u8]) {
    use std::io::Write;
    let Some(path) = std::env::var_os("PTT_DUMP") else {
        return;
    };
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = f.write_all(bytes);
    }
}

// ---- 繪製 ----

fn paint_frame(f: &Frame, window: &mut Window, cx: &mut App) {
    let lay = f.layout;
    // 1. 背景：同列連續同色合併成一個 quad
    for row in 0..ROWS {
        let mut run_start = 0;
        let mut run_color = bg_of(&f.cells[row * COLS], f);
        for col in 1..=COLS {
            let color = if col < COLS {
                Some(bg_of(&f.cells[row * COLS + col], f))
            } else {
                None
            };
            if color != Some(run_color) {
                if run_color != 0 {
                    window.paint_quad(fill(
                        lay.cell_bounds(row, run_start, col - run_start),
                        f.palette[run_color],
                    ));
                }
                if let Some(c) = color {
                    run_start = col;
                    run_color = c;
                }
            }
        }
    }

    // 2. 字形
    let text_system = window.text_system().clone();
    for row in 0..ROWS {
        let mut col = 0;
        while col < COLS {
            let i = row * COLS + col;
            let cell = f.cells[i];
            let (fg, _) = cell_colors(&cell);
            let visible = !(cell.blink() && !f.blink_on);
            let paired = cell.half == 1
                && col + 1 < COLS
                && f.cells[i + 1].half == 2
                && f.cells[i + 1].glyph == cell.glyph;
            let fg_color = f.palette[fg];
            if cell.underline() && visible {
                paint_underline(
                    window,
                    lay.cell_bounds(row, col, 1),
                    fg_color,
                    window.scale_factor(),
                );
            }
            if cell.byte >= 0 || !visible {
                col += 1;
                continue;
            }
            match cell.half {
                0 => {
                    if cell.glyph != ' ' && cell.glyph != '\0' {
                        let b = lay.cell_bounds(row, col, 1);
                        draw_glyph(&text_system, window, cx, f, cell.glyph, b, b, fg_color);
                    }
                    col += 1;
                }
                1 if paired => {
                    let right = f.cells[i + 1];
                    let (fg_r, _) = cell_colors(&right);
                    let full = lay.cell_bounds(row, col, 2);
                    let right_visible = !(right.blink() && !f.blink_on);
                    if right.underline() && right_visible {
                        paint_underline(
                            window,
                            lay.cell_bounds(row, col + 1, 1),
                            f.palette[fg_r],
                            window.scale_factor(),
                        );
                    }
                    if fg_r == fg && right_visible {
                        draw_glyph(
                            &text_system,
                            window,
                            cx,
                            f,
                            cell.glyph,
                            full,
                            full,
                            fg_color,
                        );
                    } else {
                        let left_clip = lay.cell_bounds(row, col, 1);
                        draw_glyph(
                            &text_system,
                            window,
                            cx,
                            f,
                            cell.glyph,
                            full,
                            left_clip,
                            fg_color,
                        );
                        if right_visible {
                            let right_clip = lay.cell_bounds(row, col + 1, 1);
                            draw_glyph(
                                &text_system,
                                window,
                                cx,
                                f,
                                cell.glyph,
                                full,
                                right_clip,
                                f.palette[fg_r],
                            );
                        }
                    }
                    col += 2;
                }
                // 孤兒半字：字形以虛擬的兩格外框定位，只露出自己那一半
                half => {
                    let start = if half == 1 {
                        col as isize
                    } else {
                        col as isize - 1
                    };
                    let clip = lay.cell_bounds(row, col, 1);
                    let full = Bounds::new(
                        point(
                            lay.origin.x + lay.cell_w * start as f32,
                            lay.origin.y + lay.cell_h * row as f32,
                        ),
                        size(lay.cell_w * 2.0, lay.cell_h),
                    );
                    draw_glyph(
                        &text_system,
                        window,
                        cx,
                        f,
                        cell.glyph,
                        full,
                        clip,
                        fg_color,
                    );
                    col += 1;
                }
            }
        }
    }

    // 3. 網址常駐底線
    let link_color: Hsla = gpui::rgba(0xffffff99).into();
    for row in 0..ROWS {
        for span in scan_row(&f.cells[row * COLS..(row + 1) * COLS]) {
            paint_underline(
                window,
                lay.cell_bounds(row, span.col, span.width),
                link_color,
                window.scale_factor(),
            );
        }
    }

    // 4. 選取範圍：半透明覆蓋（列優先，中間列整列）
    if let Some((a, b)) = f.selection {
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let sel_color: Hsla = gpui::rgba(0xffffff50).into();
        for row in a.0..=b.0 {
            let c0 = if row == a.0 { a.1 } else { 0 };
            let c1 = if row == b.0 { b.1 } else { COLS - 1 };
            window.paint_quad(fill(lay.cell_bounds(row, c0, c1 - c0 + 1), sel_color));
        }
    }

    // 5. 游標：有焦點時閃爍的半透明方塊，失焦時空心框
    let (crow, ccol) = f.cursor;
    let cb = lay.cell_bounds(crow, ccol, 1);
    if !f.cursor_visible {
        // DECTCEM 關閉：不畫
    } else if f.focused {
        if f.cursor_on {
            window.paint_quad(fill(cb, gpui::rgba(0xc0c0c0b0)));
        }
    } else {
        window.paint_quad(gpui::outline(
            cb,
            gpui::rgba(0xc0c0c0b0),
            gpui::BorderStyle::Solid,
        ));
    }

    // 6. 輸入法預編輯文字：從游標格往右畫，深灰底、白字、整段底線；超出右緣的部分不畫
    if !f.preedit.is_empty() {
        let bg: Hsla = gpui::rgb(0x3a3a3a).into();
        let fg: Hsla = gpui::rgb(0xffffff).into();
        let mut col = ccol;
        for ch in f.preedit.chars() {
            let w = preedit_char_cells(ch);
            if w == 0 {
                continue;
            }
            if col + w > COLS {
                break;
            }
            let b = lay.cell_bounds(crow, col, w);
            window.paint_quad(fill(b, bg));
            draw_glyph(&text_system, window, cx, f, ch, b, b, fg);
            col += w;
        }
        if col > ccol {
            paint_underline(
                window,
                lay.cell_bounds(crow, ccol, col - ccol),
                fg,
                window.scale_factor(),
            );
        }
    }
}

/// 預編輯字元佔的格數：寬字兩格、其餘一格；組合字元（寬度 0）不佔格
fn preedit_char_cells(ch: char) -> usize {
    match wcwidth(ch as u32) {
        0 => 0,
        2 => 2,
        _ => 1,
    }
}

fn preedit_cells(text: &str) -> usize {
    text.chars().map(preedit_char_cells).sum()
}

fn bg_of(cell: &Cell, _f: &Frame) -> usize {
    cell_colors(cell).1
}

fn paint_underline(window: &mut Window, cell: Bounds<Pixels>, color: Hsla, scale: f32) {
    let thickness = px((1.0_f32).max((scale).round()) / scale);
    let b = Bounds::new(
        point(cell.origin.x, cell.origin.y + cell.size.height - thickness),
        size(cell.size.width, thickness),
    );
    window.paint_quad(fill(b, color));
}

/// 把一個字畫進 `full`（整字外框：窄字一格、寬字兩格），只露出 `clip` 範圍。
/// 方塊／三角走幾何路徑（整格填滿，拼接無縫），其餘用字型
#[allow(clippy::too_many_arguments)]
fn draw_glyph(
    text_system: &WindowTextSystem,
    window: &mut Window,
    cx: &mut App,
    f: &Frame,
    ch: char,
    full: Bounds<Pixels>,
    clip: Bounds<Pixels>,
    color: Hsla,
) {
    let mask = Some(ContentMask { bounds: clip });
    window.with_content_mask(mask, |window| {
        if draw_geometry(window, ch as u32, full, color) {
            return;
        }
        let text: SharedString = ch.to_string().into();
        let run = TextRun {
            len: text.len(),
            font: f.font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = text_system.shape_line(text, f.layout.font_size, &[run], None);
        // ShapedLine::paint 把 (ascent+descent) 的行框置中於 line_height；
        // 這裡要的是「字面 em 框置中於格子」，故以校準的字面中心反推 origin
        let em = f32::from(f.layout.font_size);
        let baseline_y = f32::from(full.origin.y)
            + f32::from(full.size.height) / 2.0
            + f.calib.center_ratio * em;
        let line_h = full.size.height;
        let padding_top =
            (f32::from(line_h) - f32::from(line.ascent) - f32::from(line.descent)) / 2.0;
        let origin_y = baseline_y - (padding_top + f32::from(line.ascent));
        let _ = line.paint(point(full.origin.x, px(origin_y)), line_h, window, cx);
    });
}

/// 象限方塊 U+2596–259F 的填格遮罩，bit 順序 [左上, 右上, 左下, 右下]
const QUADRANTS: [u8; 10] = [
    0b0010, 0b0001, 0b1000, 0b1011, 0b1001, 0b1110, 0b1101, 0b0100, 0b0110, 0b0111,
];

fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
    Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
}

/// 幾何自繪：以整字外框為座標。回傳 false 表示非幾何字元，交給字型
fn draw_geometry(window: &mut Window, cp: u32, b: Bounds<Pixels>, color: Hsla) -> bool {
    let (x, y, w, h) = (
        f32::from(b.origin.x),
        f32::from(b.origin.y),
        f32::from(b.size.width),
        f32::from(b.size.height),
    );
    let mut fill_rect = |r: Bounds<Pixels>, c: Hsla| window.paint_quad(fill(r, c));
    match cp {
        0x2580 => fill_rect(rect(x, y, w, h / 2.0), color), // ▀ 上半
        0x2581..=0x2588 => {
            let k = (cp - 0x2580) as f32 / 8.0; // ▁▂▃▄▅▆▇█ 下方 k/8
            fill_rect(rect(x, y + h * (1.0 - k), w, h * k), color);
        }
        0x2589..=0x258f => {
            let k = (0x2590 - cp) as f32 / 8.0; // ▉▊▋▌▍▎▏ 左方 k/8
            fill_rect(rect(x, y, w * k, h), color);
        }
        0x2590 => fill_rect(rect(x + w / 2.0, y, w / 2.0, h), color), // ▐ 右半
        0x2591..=0x2593 => {
            // ░▒▓ 以透明度近似 25/50/75% 網點
            let mut c = color;
            c.a *= (cp - 0x2590) as f32 * 0.25;
            fill_rect(rect(x, y, w, h), c);
        }
        0x2594 => fill_rect(rect(x, y, w, h / 8.0), color), // ▔ 上 1/8
        0x2595 => fill_rect(rect(x + w * 7.0 / 8.0, y, w / 8.0, h), color), // ▕ 右 1/8
        0x2596..=0x259f => {
            let mask = QUADRANTS[(cp - 0x2596) as usize];
            let (hw, hh) = (w / 2.0, h / 2.0);
            if mask & 0b1000 != 0 {
                fill_rect(rect(x, y, hw, hh), color);
            }
            if mask & 0b0100 != 0 {
                fill_rect(rect(x + hw, y, hw, hh), color);
            }
            if mask & 0b0010 != 0 {
                fill_rect(rect(x, y + hh, hw, hh), color);
            }
            if mask & 0b0001 != 0 {
                fill_rect(rect(x + hw, y + hh, hw, hh), color);
            }
        }
        0x25e2..=0x25e5 => {
            // ◢◣◤◥ 實心直角三角形
            let mut pb = PathBuilder::fill();
            let (tl, tr, bl, br) = (
                point(px(x), px(y)),
                point(px(x + w), px(y)),
                point(px(x), px(y + h)),
                point(px(x + w), px(y + h)),
            );
            let pts = match cp {
                0x25e2 => [tr, br, bl],
                0x25e3 => [tl, br, bl],
                0x25e4 => [tl, tr, bl],
                _ => [tl, tr, br],
            };
            pb.move_to(pts[0]);
            pb.line_to(pts[1]);
            pb.line_to(pts[2]);
            pb.close();
            if let Ok(path) = pb.build() {
                window.paint_path(path, color);
            }
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    // 假資料同 config.example.json
    fn sample_form() -> AccountForm {
        AccountForm::new("your_ptt_id", "your_password", "y")
    }

    fn overlay_row_text(cells: &[Cell], row: usize) -> String {
        cells[row * COLS..(row + 1) * COLS]
            .iter()
            .filter(|c| c.half != 2)
            .map(|c| if c.glyph == '\0' { ' ' } else { c.glyph })
            .collect()
    }

    #[test]
    fn form_overlay_covers_exactly_its_rectangle() {
        let mut cells = vec![Cell::BLANK; ROWS * COLS];
        overlay_form(&mut cells, &sample_form());
        for row in 0..ROWS {
            for col in 0..COLS {
                let inside = (FORM_TOP..FORM_TOP + account_form::HEIGHT).contains(&row)
                    && (FORM_LEFT..FORM_LEFT + account_form::WIDTH).contains(&col);
                let cell = cells[row * COLS + col];
                if !inside {
                    assert!(cell == Cell::BLANK, "對話框外的格子被動到：{row},{col}");
                }
            }
            if (FORM_TOP..FORM_TOP + account_form::HEIGHT).contains(&row) {
                // 每列左右兩端都是有底色的空白：表單算的寬度與網格的字寬一致，沒有少畫或折行
                for col in [FORM_LEFT, FORM_LEFT + account_form::WIDTH - 1] {
                    let cell = cells[row * COLS + col];
                    assert_eq!(cell.glyph, ' ', "列 {row} 欄 {col}");
                    assert_ne!(cell_colors(&cell).1, 0, "列 {row} 欄 {col} 沒有底色");
                }
            }
        }
    }

    #[test]
    fn form_overlay_shows_values_but_masks_password() {
        let mut cells = vec![Cell::BLANK; ROWS * COLS];
        let cursor = overlay_form(&mut cells, &sample_form());
        let all: String = (0..ROWS).map(|r| overlay_row_text(&cells, r)).collect();
        assert!(overlay_row_text(&cells, FORM_TOP).contains("帳號設定"));
        let username_row = overlay_row_text(&cells, FORM_TOP + 2);
        assert!(username_row.contains("帳號：") && username_row.contains("your_ptt_id"));
        let password_row = overlay_row_text(&cells, FORM_TOP + 3);
        assert!(password_row.contains("密碼：") && password_row.contains("************* "));
        assert!(!all.contains("your_password"));
        // 游標停在帳號欄文字之後
        let (row, col) = cursor;
        assert_eq!(row, FORM_TOP + 2);
        assert_eq!(cells[row * COLS + col - 1].glyph, 'd');
        assert_eq!(cells[row * COLS + col].glyph, ' ');
    }

    #[test]
    fn form_overlay_replaces_what_was_underneath() {
        let mut g = Grid::new();
        g.set_utf8(true);
        for _ in 0..ROWS {
            g.write_text(&"中".repeat(COLS / 2));
        }
        let mut cells = g.cells().to_vec();
        overlay_form(&mut cells, &sample_form());
        // 空白列：對話框範圍內不殘留底下的字，範圍外原樣保留
        let row = &cells[(FORM_TOP + 1) * COLS..(FORM_TOP + 2) * COLS];
        let right = FORM_LEFT + account_form::WIDTH;
        assert!(row[FORM_LEFT..right].iter().all(|c| c.glyph == ' '));
        assert!(row[..FORM_LEFT].iter().all(|c| c.glyph == '中'));
        assert!(row[right..].iter().all(|c| c.glyph == '中'));
    }

    #[test]
    fn preedit_cells_counts_wide_and_narrow_chars() {
        assert_eq!(preedit_cells(""), 0);
        assert_eq!(preedit_cells("abc"), 3);
        // 注音符號與漢字各佔兩格
        assert_eq!(preedit_cells("ㄅㄙ"), 4);
        assert_eq!(preedit_cells("雨天x"), 5);
    }

    #[test]
    fn preedit_combining_marks_take_no_cell() {
        // U+0301 結合用銳音符
        assert_eq!(preedit_cells("e\u{301}"), 1);
    }

    #[test]
    fn wheel_notch_sends_one_key_per_notch() {
        let mut w = WheelAccum::default();
        // 滑鼠滾一格是 ±3 行；往上是正值
        assert_eq!(w.feed(3.0), 1);
        assert_eq!(w.feed(3.0), 1);
        assert_eq!(w.feed(-3.0), -1);
        // 一個事件含兩格（滾得快時 Wayland 會併成一個 frame）
        assert_eq!(w.feed(6.0), 2);
    }

    #[test]
    fn wheel_accumulates_partial_notches() {
        let mut w = WheelAccum::default();
        // 高解析滾輪（value120）一次只給四分之一格
        assert_eq!(w.feed(0.75), 0);
        assert_eq!(w.feed(0.75), 0);
        assert_eq!(w.feed(0.75), 0);
        assert_eq!(w.feed(0.75), 1);
        // 餘量歸零，下一個四分之一格不會送
        assert_eq!(w.feed(0.75), 0);
    }

    #[test]
    fn wheel_direction_change_drops_leftover() {
        let mut w = WheelAccum::default();
        assert_eq!(w.feed(2.5), 0);
        // 反方向一格就要立刻送 ↓，不能先抵掉往上的 2.5 行
        assert_eq!(w.feed(-3.0), -1);
        // 換回往上：先前往下的餘量（0）不影響
        assert_eq!(w.feed(3.0), 1);
    }

    #[test]
    fn wheel_caps_steps_per_event_and_drops_excess() {
        let mut w = WheelAccum::default();
        assert_eq!(w.feed(100.0), WHEEL_MAX_STEPS);
        // 超過上限的部分整個丟掉，不會在下一個事件補送
        assert_eq!(w.feed(1.0), 0);
        assert_eq!(w.feed(-100.0), -WHEEL_MAX_STEPS);
    }

    #[test]
    fn wheel_reset_clears_leftover() {
        let mut w = WheelAccum::default();
        assert_eq!(w.feed(2.9), 0);
        w.reset();
        assert_eq!(w.feed(0.2), 0);
    }

    #[test]
    fn preview_tall_image_uses_full_window_height() {
        // 463×823 直式圖：高度縮到視窗高扣上下邊距，寬度等比，不再被壓在視窗的一小塊
        let (left, top, w, h) = preview_rect((463.0, 823.0), (100.0, 100.0), (1000.0, 720.0));
        assert_eq!(h, 696.0);
        assert!((w / h - 463.0 / 823.0).abs() < 1e-4);
        assert_eq!(left, 112.0);
        // 以滑鼠為中心會超出上緣，夾回邊距
        assert_eq!(top, 12.0);
    }

    #[test]
    fn preview_small_image_keeps_size_and_centers_on_mouse() {
        // 不放大；滑鼠在右半邊時放左側、右緣離滑鼠一個邊距
        assert_eq!(
            preview_rect((100.0, 50.0), (800.0, 300.0), (1000.0, 720.0)),
            (688.0, 275.0, 100.0, 50.0)
        );
    }

    #[test]
    fn preview_wide_image_is_limited_by_the_wider_side() {
        // 滑鼠在 x=300：右側可用 700−24，寬圖以寬度為限
        let (left, _, w, h) = preview_rect((2000.0, 500.0), (300.0, 360.0), (1000.0, 720.0));
        assert_eq!((left, w, h), (312.0, 676.0, 169.0));
    }

    #[test]
    fn preview_never_leaves_the_viewport() {
        let (vw, vh) = (1000.0, 720.0);
        for image in [
            (463.0, 823.0),
            (4000.0, 300.0),
            (300.0, 9000.0),
            (50.0, 50.0),
            (0.0, 0.0),
        ] {
            for mouse in [
                (0.0, 0.0),
                (999.0, 0.0),
                (0.0, 719.0),
                (999.0, 719.0),
                (500.0, 360.0),
            ] {
                let (left, top, w, h) = preview_rect(image, mouse, (vw, vh));
                assert!(w > 0.0 && h > 0.0, "{image:?} {mouse:?}");
                assert!(left >= 0.0 && left + w <= vw, "{image:?} {mouse:?}");
                assert!(top >= 0.0 && top + h <= vh, "{image:?} {mouse:?}");
            }
        }
    }
}
