//! 自動登入狀態機（移植自原專案 `src/auto-login.ts`）。
//!
//! 對 telnet 濾層後的**原始位元組**做 pattern matching：每個提示字樣同時準備 Big5 與
//! UTF-8 兩種位元組序列，登入前後的編碼切換因此不影響比對（原版在解碼後文字比對，
//! 唯編碼切換點偵測在位元組層；這裡統一在位元組層，少一條 codec 依賴）。
//!
//! 安全 invariant：任一步異常（密碼錯、提示重複、逾時）立即交還手動，**絕不自動重試**。
//! 計時不持有 timer：呼叫端以 `poll(now)` 定期查逾時（GPUI 版由 500ms tick 驅動）。
//!
//! 分兩階段。**登入前**（代號、密碼）出任何異常都算中止，要讓使用者看到原因。看到
//! 「密碼正確」（或只在密碼通過後才出現的重複登入提示）即進入**已登入**階段：之後只剩按掉
//! 歡迎畫面、回答重複登入這類便利動作，逾時不再算中止，改成安靜交還（`Reason::HandedOver`）。
//! 依 pttbbs `mbbsd.c`，密碼通過到主選單之間還可能出現錯誤嘗試記錄、新看板、使用者條款、
//! 進站畫面等提示，這些刻意不自動回答，留給使用者自己看；而且 PTT 輸入緩衝裡有鍵時不重畫
//! （`pfterm.c` 的 typeahead），使用者多按一個 Enter 就會略過主選單直接進分類看板，
//! 「【主功能表】」根本不會送來。這些情況以前都會在 10 秒後誤報「中止（逾時）」。

use std::time::{Duration, Instant};

/// 登入前每一步的逾時
pub const STEP_TIMEOUT: Duration = Duration::from_secs(10);
/// 已登入後每一步的逾時。回答重複登入後伺服器會故意先睡 0～5 秒，選 y 還要踢連線、
/// 每輪再睡 1～3 秒（`multi_user_check`），10 秒不夠；逾時在這階段只是安靜交還，放寬無害
pub const POST_AUTH_TIMEOUT: Duration = Duration::from_secs(20);
const BUFFER_LIMIT: usize = 4096;

pub struct Credentials {
    pub username: String,
    pub password: String,
    /// 「您想刪除其他重複登入的連線嗎」的回答（預設 y）
    pub kick_other_logins: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// 看到主選單
    Done,
    /// 已登入，但沒等到主選單（停在不認得的提示，或使用者已自己操作）：安靜交還手動
    HandedOver,
    /// 登入前出錯，附原因
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// 送出文字（UTF-8 編碼）給伺服器
    Send(String),
    /// 伺服器已切 UTF-8：呼叫端須在餵本 chunk 給畫面模型**之前**切換解碼
    SwitchUtf8,
    /// Big5「密碼不對」命中＝伺服器仍在 Big5（舊版），呼叫端撥回 big5 解碼
    ResetBig5,
    Finished {
        reason: Reason,
        detail: Option<String>,
    },
}

struct Needle {
    big5: Vec<u8>,
    utf8: Vec<u8>,
}

impl Needle {
    fn new(text: &str) -> Self {
        let (big5, _, _) = encoding_rs::BIG5.encode(text);
        Self {
            big5: big5.into_owned(),
            utf8: text.as_bytes().to_vec(),
        }
    }
    fn hit_big5(&self, hay: &[u8]) -> bool {
        contains(hay, &self.big5)
    }
    fn hit_utf8(&self, hay: &[u8]) -> bool {
        contains(hay, &self.utf8)
    }
    fn hit(&self, hay: &[u8]) -> bool {
        self.hit_utf8(hay) || self.hit_big5(hay)
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

/// 中止原因字樣。與比對處共用，勿各自寫死
const WRONG_PASSWORD_DETAIL: &str = "密碼不對";
/// 主功能表偵測字樣：PTT 主選單標題列（以實錄畫面為準）
const MAIN_MENU: &str = "【主功能表】";
/// 密碼通過：pttbbs `login_query` 送出「密碼正確！ 開始登入系統...」
const PASSWORD_OK: &str = "密碼正確";

struct Needles {
    wrong_password: Needle,
    password_ok: Needle,
    main_menu: Needle,
    id_prompt: Needle,
    password_prompt: Needle,
    dup_login: Needle,
    any_key: Needle,
}

pub struct AutoLogin {
    cfg: Credentials,
    needles: Needles,
    buffer: Vec<u8>,
    active: bool,
    sent_username: bool,
    sent_password: bool,
    /// 送出密碼後看到「密碼正確」或重複登入提示＝已登入（見檔頭「分兩階段」）
    authenticated: bool,
    /// 保證 SwitchUtf8 只發一次（位元組層偵測或送密碼後二擇一）
    utf8_switched: bool,
    deadline: Option<Instant>,
}

impl AutoLogin {
    pub fn new(cfg: Credentials) -> Self {
        Self {
            cfg,
            needles: Needles {
                wrong_password: Needle::new(WRONG_PASSWORD_DETAIL),
                password_ok: Needle::new(PASSWORD_OK),
                main_menu: Needle::new(MAIN_MENU),
                id_prompt: Needle::new("請輸入代號"),
                password_prompt: Needle::new("請輸入您的密碼"),
                dup_login: Needle::new("您想刪除其他重複登入的連線嗎"),
                any_key: Needle::new("按任意鍵繼續"),
            },
            buffer: Vec::new(),
            active: false,
            sent_username: false,
            sent_password: false,
            authenticated: false,
            utf8_switched: false,
            deadline: None,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn running(&self) -> bool {
        self.active
    }

    /// 密碼已通過（見檔頭「分兩階段」）。呼叫端據此決定使用者插手時要不要提示
    pub fn authenticated(&self) -> bool {
        self.authenticated
    }

    pub fn start(&mut self, now: Instant) {
        self.active = true;
        self.deadline = Some(now + STEP_TIMEOUT);
    }

    pub fn stop(&mut self) {
        self.active = false;
        self.buffer.clear();
        self.deadline = None;
    }

    /// 餵一批 telnet 濾層後的原始 bytes。回傳事件依序處理；呼叫端必須在把同一批
    /// bytes 餵給畫面模型之前處理 SwitchUtf8／ResetBig5，提示才不會先以錯誤編碼上畫面
    pub fn feed(&mut self, data: &[u8], now: Instant) -> Vec<Event> {
        let mut events = Vec::new();
        if !self.active {
            return events;
        }
        self.buffer.extend_from_slice(data);
        if self.buffer.len() > BUFFER_LIMIT {
            let excess = self.buffer.len() - BUFFER_LIMIT;
            self.buffer.drain(..excess);
        }
        let n = &self.needles;

        if !self.authenticated {
            // 密碼錯：Big5 命中＝伺服器仍在 Big5（舊版）→ 撥回；UTF-8 命中（新版）維持 utf-8
            if n.wrong_password.hit_big5(&self.buffer) {
                events.push(Event::ResetBig5);
                self.finish(&mut events, Reason::Aborted, Some(WRONG_PASSWORD_DETAIL));
                return events;
            }
            if n.wrong_password.hit_utf8(&self.buffer) {
                self.finish(&mut events, Reason::Aborted, Some(WRONG_PASSWORD_DETAIL));
                return events;
            }
            // 新版 PTT：收到「id,」即切 UTF-8，密碼提示已是 UTF-8 → 解碼前先切
            if !self.utf8_switched && n.password_prompt.hit_utf8(&self.buffer) {
                self.utf8_switched = true;
                events.push(Event::SwitchUtf8);
            }
            // 只認自己送出密碼之後的訊號；重複登入提示只在密碼通過後才出現，
            // 「密碼正確」萬一因 typeahead 沒送來，靠它補判。不 return：同一批可能接著就是提示
            if self.sent_password
                && (n.password_ok.hit(&self.buffer) || n.dup_login.hit(&self.buffer))
            {
                self.authenticated = true;
                self.deadline = Some(now + POST_AUTH_TIMEOUT);
            }
        }
        if n.main_menu.hit(&self.buffer) {
            self.finish(&mut events, Reason::Done, None);
            return events;
        }
        // 已登入後不再處理代號／密碼提示，避免畫面內容碰巧含這些字時送出帳密
        if !self.authenticated && n.id_prompt.hit(&self.buffer) {
            if self.sent_username {
                self.finish(&mut events, Reason::Aborted, Some("代號提示重複出現"));
                return events;
            }
            self.sent_username = true;
            let text = format!("{},\r", self.cfg.username); // 逗號啟用 UTF-8 模式
            self.act(&mut events, text, now);
            return events;
        }
        if !self.authenticated && n.password_prompt.hit(&self.buffer) {
            if self.sent_password {
                self.finish(&mut events, Reason::Aborted, Some("密碼提示重複出現"));
                return events;
            }
            self.sent_password = true;
            let text = format!("{}\r", self.cfg.password);
            self.act(&mut events, text, now);
            // 舊版 PTT：提示是 Big5、送出密碼後其後輸出才是 UTF-8 → 此時才切；
            // 新版已在上方提前切換，不得重切
            if !self.utf8_switched {
                self.utf8_switched = true;
                events.push(Event::SwitchUtf8);
            }
            return events;
        }
        if n.dup_login.hit(&self.buffer) {
            let text = format!("{}\r", self.cfg.kick_other_logins);
            self.act(&mut events, text, now);
            return events;
        }
        if n.any_key.hit(&self.buffer) {
            self.act(&mut events, " ".to_string(), now);
        }
        events
    }

    /// 逾時檢查：登入前任一步卡超過 `STEP_TIMEOUT` → 中止；已登入後超過
    /// `POST_AUTH_TIMEOUT` → 安靜交還。兩者都交還手動，都不重試
    pub fn poll(&mut self, now: Instant) -> Vec<Event> {
        let mut events = Vec::new();
        if self.active && self.deadline.is_some_and(|d| now >= d) {
            if self.authenticated {
                self.finish(&mut events, Reason::HandedOver, None);
            } else {
                self.finish(&mut events, Reason::Aborted, Some("逾時"));
            }
        }
        events
    }

    /// 清空已比對內容避免同一提示重複觸發，送出後重新起算逾時
    fn act(&mut self, events: &mut Vec<Event>, text: String, now: Instant) {
        self.buffer.clear();
        events.push(Event::Send(text));
        let timeout = if self.authenticated {
            POST_AUTH_TIMEOUT
        } else {
            STEP_TIMEOUT
        };
        self.deadline = Some(now + timeout);
    }

    fn finish(&mut self, events: &mut Vec<Event>, reason: Reason, detail: Option<&str>) {
        if !self.active {
            return;
        }
        self.stop();
        events.push(Event::Finished {
            reason,
            detail: detail.map(str::to_string),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WRONG_PASSWORD_BIG5: &[u8] = &[0xb1, 0x4b, 0xbd, 0x58, 0xa4, 0xa3, 0xb9, 0xef];

    struct H {
        login: AutoLogin,
        now: Instant,
        sent: Vec<String>,
        switched: usize,
        reset: usize,
        finished: Option<(Reason, Option<String>)>,
    }

    impl H {
        fn new() -> Self {
            let now = Instant::now();
            let mut login = AutoLogin::new(Credentials {
                username: "myid".into(),
                password: "secret".into(),
                kick_other_logins: "n".into(),
            });
            login.start(now);
            Self {
                login,
                now,
                sent: vec![],
                switched: 0,
                reset: 0,
                finished: None,
            }
        }
        fn apply(&mut self, events: Vec<Event>) {
            for e in events {
                match e {
                    Event::Send(s) => self.sent.push(s),
                    Event::SwitchUtf8 => self.switched += 1,
                    Event::ResetBig5 => self.reset += 1,
                    Event::Finished { reason, detail } => self.finished = Some((reason, detail)),
                }
            }
        }
        fn feed_bytes(&mut self, b: &[u8]) {
            let ev = self.login.feed(b, self.now);
            self.apply(ev);
        }
        fn feed_utf8(&mut self, s: &str) {
            self.feed_bytes(s.as_bytes());
        }
        fn feed_big5(&mut self, s: &str) {
            let (b, _, _) = encoding_rs::BIG5.encode(s);
            self.feed_bytes(&b);
        }
        fn advance(&mut self, d: Duration) {
            self.now += d;
            let ev = self.login.poll(self.now);
            self.apply(ev);
        }
    }

    #[test]
    fn big5_needle_matches_recorded_bytes() {
        let n = Needle::new("密碼不對");
        assert_eq!(n.big5, WRONG_PASSWORD_BIG5);
    }

    #[test]
    fn full_flow_old_ptt_big5_prompts() {
        let mut h = H::new();
        h.feed_big5("請輸入代號，或以 guest 參觀，或以 new 註冊: ");
        assert_eq!(h.sent, vec!["myid,\r"]);
        h.feed_big5("\r\n請輸入您的密碼: ");
        assert_eq!(h.sent, vec!["myid,\r", "secret\r"]);
        assert_eq!(h.switched, 1);
        h.feed_utf8("您想刪除其他重複登入的連線嗎？[Y/n] ");
        assert_eq!(h.sent.last().unwrap(), "n\r");
        h.feed_utf8("歡迎您再度拜訪，上次您是從 ... 請按任意鍵繼續");
        h.feed_utf8("請按任意鍵繼續");
        assert_eq!(h.sent.iter().filter(|s| *s == " ").count(), 2);
        h.feed_utf8("【主功能表】 批踢踢實業坊");
        assert_eq!(h.finished, Some((Reason::Done, None)));
        assert!(!h.login.running());
        assert_eq!(h.switched, 1);
        assert_eq!(h.reset, 0);
    }

    #[test]
    fn new_ptt_utf8_prompt_switches_before_sending_once() {
        let mut h = H::new();
        h.feed_big5("請輸入代號：");
        assert_eq!(h.sent, vec!["myid,\r"]);
        // 2026-08-09 實錄：echo + ANSI + UTF-8 提示在同一 chunk
        let mut chunk = b"myid,\x1b[m\x1b[22;1H\x1b[K\x1b[m".to_vec();
        chunk.extend_from_slice("請輸入您的密碼".as_bytes());
        chunk.extend_from_slice(b": ");
        let ev = h.login.feed(&chunk, h.now);
        // 切換事件必須排在送密碼之前（呼叫端據此先撥 codec 再餵畫面）
        assert_eq!(ev, vec![Event::SwitchUtf8, Event::Send("secret\r".into())]);
        h.apply(ev);
        assert_eq!(h.switched, 1);
        // 提示再次出現 → 中止（防重送保護不因位元組層切換而失效）
        h.feed_utf8("請輸入您的密碼: ");
        assert_eq!(
            h.finished,
            Some((Reason::Aborted, Some("密碼提示重複出現".into())))
        );
        assert_eq!(h.sent, vec!["myid,\r", "secret\r"]);
    }

    #[test]
    fn utf8_prompt_split_across_chunks() {
        let mut h = H::new();
        h.feed_big5("請輸入代號：");
        let p = "請輸入您的密碼".as_bytes();
        h.feed_bytes(&p[..5]);
        assert_eq!(h.switched, 0);
        assert_eq!(h.sent.len(), 1);
        h.feed_bytes(&p[5..]);
        assert_eq!(h.switched, 1);
        assert_eq!(h.sent, vec!["myid,\r", "secret\r"]);
    }

    #[test]
    fn new_ptt_wrong_password_utf8_aborts_without_reset() {
        let mut h = H::new();
        h.feed_big5("請輸入代號：");
        h.feed_utf8("請輸入您的密碼: ");
        assert_eq!(h.sent, vec!["myid,\r", "secret\r"]);
        h.feed_utf8("\r\n密碼不對或無此帳號。");
        assert_eq!(h.finished, Some((Reason::Aborted, Some("密碼不對".into()))));
        assert_eq!(h.reset, 0);
        assert_eq!(h.sent.len(), 2);
    }

    #[test]
    fn old_ptt_wrong_password_big5_resets_and_aborts() {
        let mut h = H::new();
        h.feed_big5("請輸入代號：");
        h.feed_big5("請輸入您的密碼：");
        let mut chunk = b"\x1b[1;37m".to_vec();
        chunk.extend_from_slice(WRONG_PASSWORD_BIG5);
        chunk.extend_from_slice(&[0xa9, 0xce]);
        let ev = h.login.feed(&chunk, h.now);
        assert_eq!(ev[0], Event::ResetBig5);
        h.apply(ev);
        assert_eq!(h.finished, Some((Reason::Aborted, Some("密碼不對".into()))));
        assert_eq!(h.reset, 1);
        assert_eq!(h.sent, vec!["myid,\r", "secret\r"]);
        // 之後任何輸入都不再動作
        h.feed_big5("請輸入代號：");
        assert_eq!(h.sent.len(), 2);
    }

    #[test]
    fn big5_wrong_password_split_across_chunks() {
        let mut h = H::new();
        h.feed_bytes(&WRONG_PASSWORD_BIG5[..3]);
        assert!(h.finished.is_none());
        h.feed_bytes(&WRONG_PASSWORD_BIG5[3..]);
        assert_eq!(h.reset, 1);
        assert!(h.finished.is_some());
    }

    #[test]
    fn stop_disables_feed() {
        let mut h = H::new();
        h.login.stop();
        h.feed_bytes(WRONG_PASSWORD_BIG5);
        assert!(h.finished.is_none());
        assert_eq!(h.reset, 0);
    }

    #[test]
    fn repeated_id_prompt_aborts() {
        let mut h = H::new();
        h.feed_big5("請輸入代號：");
        h.feed_big5("請輸入代號：");
        assert_eq!(
            h.finished,
            Some((Reason::Aborted, Some("代號提示重複出現".into())))
        );
        assert_eq!(h.sent, vec!["myid,\r"]);
    }

    #[test]
    fn step_timeout_aborts_and_each_action_rearms() {
        let mut h = H::new();
        h.advance(Duration::from_secs(9));
        assert!(h.finished.is_none());
        h.feed_big5("請輸入代號：");
        h.advance(Duration::from_secs(9));
        assert!(h.finished.is_none()); // 送出代號後重新起算
        h.advance(Duration::from_secs(2));
        assert_eq!(h.finished, Some((Reason::Aborted, Some("逾時".into()))));
        assert!(!h.login.running());
        // 中止後不再有事件
        h.advance(Duration::from_secs(60));
        assert_eq!(h.sent, vec!["myid,\r"]);
    }

    /// 走到「密碼已送出」
    fn h_after_password() -> H {
        let mut h = H::new();
        h.feed_big5("請輸入代號：");
        h.feed_utf8("請輸入您的密碼: ");
        assert_eq!(h.sent, vec!["myid,\r", "secret\r"]);
        h
    }

    #[test]
    fn authenticated_only_after_password_is_confirmed() {
        assert!(!H::new().login.authenticated());
        // 密碼送出但還沒確認：可能是錯的，仍算登入前
        let mut h = h_after_password();
        assert!(!h.login.authenticated());
        h.feed_utf8("正在檢查密碼...密碼正確！ 開始登入系統...");
        assert!(h.login.authenticated());
        // 之後按掉歡迎畫面也不會退回登入前
        h.feed_utf8("請按任意鍵繼續");
        assert!(h.login.authenticated());
    }

    #[test]
    fn timeout_after_password_ok_hands_over_quietly() {
        let mut h = h_after_password();
        h.feed_utf8("正在檢查密碼...密碼正確！ 開始登入系統...");
        assert!(h.login.authenticated);
        // 登入前的 10 秒逾時不再適用
        h.advance(STEP_TIMEOUT);
        assert!(h.finished.is_none());
        h.advance(POST_AUTH_TIMEOUT - STEP_TIMEOUT);
        assert_eq!(h.finished, Some((Reason::HandedOver, None)));
        assert!(!h.login.running());
        assert_eq!(h.sent, vec!["myid,\r", "secret\r"]);
    }

    #[test]
    fn password_ok_in_big5_also_counts() {
        let mut h = h_after_password();
        h.feed_big5("密碼正確！ 開始登入系統...");
        assert!(h.login.authenticated);
    }

    #[test]
    fn unknown_prompt_after_login_is_left_to_user() {
        let mut h = h_after_password();
        h.feed_utf8("密碼正確！ 開始登入系統...");
        h.feed_utf8("歡迎您再度拜訪，上次您是從 ... 請按任意鍵繼續");
        assert_eq!(h.sent.last().unwrap(), " ");
        // pttbbs check_bad_login 的提示：刻意不自動回答
        h.feed_utf8("您要刪除以上錯誤嘗試的記錄嗎? [Y/n] ");
        assert_eq!(h.sent, vec!["myid,\r", "secret\r", " "]);
        h.advance(POST_AUTH_TIMEOUT);
        assert_eq!(h.finished, Some((Reason::HandedOver, None)));
        assert_eq!(h.sent.len(), 3);
    }

    #[test]
    fn password_ok_before_password_sent_does_not_count() {
        let mut h = H::new();
        h.feed_utf8("密碼正確");
        assert!(!h.login.authenticated);
        h.advance(STEP_TIMEOUT);
        assert_eq!(h.finished, Some((Reason::Aborted, Some("逾時".into()))));
    }

    #[test]
    fn timeout_after_password_without_confirmation_still_aborts() {
        // 例如兩步驟驗證之類不認得的提示：沒看到「密碼正確」就不算登入，維持中止訊息
        let mut h = h_after_password();
        h.feed_utf8("請輸入驗證碼：");
        h.advance(STEP_TIMEOUT);
        assert_eq!(h.finished, Some((Reason::Aborted, Some("逾時".into()))));
    }

    #[test]
    fn dup_login_prompt_marks_login_when_password_ok_missing() {
        let mut h = h_after_password();
        h.feed_utf8("注意: 您有其它連線已登入此帳號。您想刪除其他重複登入的連線嗎？[Y/n] ");
        assert!(h.login.authenticated);
        assert_eq!(h.sent.last().unwrap(), "n\r");
        // 伺服器回答後故意先睡、再踢連線：超過 10 秒仍要等
        h.advance(Duration::from_secs(15));
        assert!(h.finished.is_none());
        h.feed_utf8("歡迎您再度拜訪 請按任意鍵繼續");
        assert_eq!(h.sent.last().unwrap(), " ");
        h.feed_utf8("【主功能表】");
        assert_eq!(h.finished, Some((Reason::Done, None)));
    }

    #[test]
    fn password_ok_and_dup_prompt_in_same_chunk_answers_once() {
        let mut h = h_after_password();
        h.feed_utf8("密碼正確！ 開始登入系統...\r\n您想刪除其他重複登入的連線嗎？[Y/n] ");
        assert_eq!(h.sent, vec!["myid,\r", "secret\r", "n\r"]);
        assert!(h.login.authenticated);
    }

    #[test]
    fn credential_prompts_and_wrong_password_text_ignored_after_login() {
        let mut h = h_after_password();
        h.feed_utf8("密碼正確！ 開始登入系統...");
        h.feed_utf8("請輸入代號 請輸入您的密碼 密碼不對");
        h.feed_bytes(WRONG_PASSWORD_BIG5);
        assert_eq!(h.sent, vec!["myid,\r", "secret\r"]);
        assert!(h.finished.is_none());
        assert!(h.login.running());
        assert_eq!(h.reset, 0);
    }

    #[test]
    fn buffer_is_bounded() {
        let mut h = H::new();
        for _ in 0..100 {
            h.feed_bytes(&[b'x'; 1000]);
        }
        assert!(h.login.buffer.len() <= BUFFER_LIMIT);
    }
}
