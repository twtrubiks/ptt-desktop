//! 帳號設定對話框的表單狀態：帳號、密碼、是否踢掉其他重複登入三欄，只收 ASCII。
//! 不依賴 gpui：按鍵分派與繪製在 `view.rs`，寫檔在 `config.rs`。
//! 畫面輸出是固定寬度的 ANSI 文字列，由 view 餵進暫時的網格再蓋到這一幀上，
//! 對話框與 PTT 畫面走同一套繪製，不需要另外的輸入框元件。
//!
//! 密碼只以 `*` 顯示；本型別刻意不實作 `Debug`，免得密碼被印進 log。

/// 對話框佔的格數
pub const WIDTH: usize = 52;
pub const HEIGHT: usize = 8;

const LABEL_W: usize = 20;
const FIELD_W: usize = 24;
/// PTT 代號上限 12 字
const MAX_USERNAME: usize = 12;
const MAX_PASSWORD: usize = 32;

const BASE: &str = "\x1b[0;37;44m";
const TITLE: &str = "\x1b[1;37;46m";
const LABEL_ON: &str = "\x1b[1;33;44m";
const FIELD_ON: &str = "\x1b[1;37;40m";
const FIELD_OFF: &str = "\x1b[0;37;40m";
const ERROR: &str = "\x1b[1;37;41m";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Username,
    Password,
    Kick,
}

#[derive(Debug, PartialEq, Eq)]
pub enum FormAction {
    /// 表單不處理這個鍵（可列印字元會由 input handler 再以 `insert` 送進來）
    Ignored,
    Handled,
    Save,
    Cancel,
}

pub struct AccountForm {
    username: String,
    password: String,
    kick: bool,
    field: Field,
    error: Option<String>,
}

impl AccountForm {
    /// 以目前的設定值帶出各欄；`kick_other_logins` 只有 "n" 視為否（與設定檔預設一致）
    pub fn new(username: &str, password: &str, kick_other_logins: &str) -> Self {
        Self {
            username: username.to_string(),
            password: password.to_string(),
            kick: !kick_other_logins.trim().eq_ignore_ascii_case("n"),
            field: Field::Username,
            error: None,
        }
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    pub fn password(&self) -> &str {
        &self.password
    }

    pub fn kick_answer(&self) -> &'static str {
        if self.kick {
            "y"
        } else {
            "n"
        }
    }

    /// 儲存失敗時顯示在對話框裡，下一次編輯就清掉
    pub fn set_error(&mut self, message: String) {
        self.error = Some(message);
    }

    /// 特殊鍵。`key` 是 GPUI 的按鍵名（enter、tab、escape、up、down、backspace）
    pub fn key(&mut self, key: &str, shift: bool) -> FormAction {
        match key.to_ascii_lowercase().as_str() {
            "escape" => return FormAction::Cancel,
            "enter" if self.field == Field::Kick => return FormAction::Save,
            "tab" if shift => self.field = self.prev_field(),
            "up" => self.field = self.prev_field(),
            "enter" | "tab" | "down" => self.field = self.next_field(),
            "backspace" => {
                match self.field {
                    Field::Username => self.username.pop(),
                    Field::Password => self.password.pop(),
                    Field::Kick => None,
                };
                self.error = None;
            }
            _ => return FormAction::Ignored,
        }
        FormAction::Handled
    }

    /// 可列印字元、輸入法提交文字與貼上的文字。不合規的字元（中文、換行、控制碼）直接丟掉
    pub fn insert(&mut self, text: &str) {
        for ch in text.chars() {
            match self.field {
                Field::Username => {
                    if ch.is_ascii_alphanumeric() && self.username.len() < MAX_USERNAME {
                        self.username.push(ch);
                    }
                }
                Field::Password => {
                    if (' '..='~').contains(&ch) && self.password.len() < MAX_PASSWORD {
                        self.password.push(ch);
                    }
                }
                Field::Kick => match ch {
                    'y' | 'Y' => self.kick = true,
                    'n' | 'N' => self.kick = false,
                    ' ' => self.kick = !self.kick,
                    _ => {}
                },
            }
        }
        self.error = None;
    }

    fn next_field(&self) -> Field {
        match self.field {
            Field::Username => Field::Password,
            Field::Password => Field::Kick,
            Field::Kick => Field::Username,
        }
    }

    fn prev_field(&self) -> Field {
        match self.field {
            Field::Username => Field::Kick,
            Field::Password => Field::Username,
            Field::Kick => Field::Password,
        }
    }

    /// 欄位裡實際顯示的字（太長只顯示尾段，留一格給游標）；密碼一律是 `*`
    fn shown(&self, field: Field) -> String {
        let room = FIELD_W - 1;
        match field {
            Field::Username => {
                let n = self.username.chars().count();
                self.username.chars().skip(n.saturating_sub(room)).collect()
            }
            Field::Password => "*".repeat(self.password.chars().count().min(room)),
            Field::Kick => format!("{}  (y/n)", self.kick_answer()),
        }
    }

    fn row(&self, field: Field, label: &str) -> String {
        let on = self.field == field;
        format!(
            "{}{}{}{}{BASE}{}",
            if on { LABEL_ON } else { BASE },
            fit(label, LABEL_W),
            if on { FIELD_ON } else { FIELD_OFF },
            fit(&self.shown(field), FIELD_W),
            fit("", WIDTH - LABEL_W - FIELD_W),
        )
    }

    /// `HEIGHT` 列、每列恰好 `WIDTH` 格的 ANSI 文字
    pub fn lines(&self) -> Vec<String> {
        let blank = format!("{BASE}{}", fit("", WIDTH));
        let message = match &self.error {
            Some(e) => format!("{ERROR}{}", fit(&format!("  {e}"), WIDTH)),
            None => format!(
                "{BASE}{}",
                fit("  密碼留空就不自動登入；密碼以明文存在設定檔", WIDTH)
            ),
        };
        vec![
            format!(
                "{TITLE}{}",
                fit("                帳號設定（自動登入）", WIDTH)
            ),
            blank.clone(),
            self.row(Field::Username, "  帳號："),
            self.row(Field::Password, "  密碼："),
            self.row(Field::Kick, "  踢掉其他重複登入："),
            blank,
            message,
            format!(
                "{BASE}{}",
                fit("  Enter 下一欄，最後一欄按 Enter 儲存   Esc 取消", WIDTH)
            ),
        ]
    }

    /// 游標在對話框內的位置（列、欄）
    pub fn cursor(&self) -> (usize, usize) {
        let (row, col) = match self.field {
            Field::Username => (2, self.shown(Field::Username).chars().count()),
            Field::Password => (3, self.shown(Field::Password).len()),
            Field::Kick => (4, 0),
        };
        (row, LABEL_W + col)
    }
}

/// 截斷或補空白到恰好 `width` 格。這裡的文字只有 ASCII（一格）與全形字（兩格）；
/// 控制字元會打亂網格，一律略過
fn fit(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars().filter(|c| !c.is_control()) {
        let w = if ch.is_ascii() { 1 } else { 2 };
        if used + w > width {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.extend(std::iter::repeat_n(' ', width - used));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // 假資料同 config.example.json
    fn form() -> AccountForm {
        AccountForm::new("your_ptt_id", "your_password", "y")
    }

    #[test]
    fn prefills_from_existing_config() {
        let f = form();
        assert_eq!(f.username(), "your_ptt_id");
        assert_eq!(f.password(), "your_password");
        assert_eq!(f.kick_answer(), "y");
        assert_eq!(AccountForm::new("", "", "n").kick_answer(), "n");
        assert_eq!(AccountForm::new("", "", " N ").kick_answer(), "n");
        // 不認得的值照設定檔預設當成 y
        assert_eq!(AccountForm::new("", "", "").kick_answer(), "y");
    }

    #[test]
    fn password_never_appears_in_lines() {
        let f = form();
        let lines = f.lines();
        assert!(lines.iter().all(|l| !l.contains("your_password")));
        assert!(lines[3].contains(&"*".repeat("your_password".len())));
        assert!(lines[2].contains("your_ptt_id"));
    }

    #[test]
    fn enter_walks_fields_then_saves() {
        let mut f = form();
        assert_eq!(f.key("enter", false), FormAction::Handled);
        assert_eq!(f.field, Field::Password);
        assert_eq!(f.key("enter", false), FormAction::Handled);
        assert_eq!(f.field, Field::Kick);
        assert_eq!(f.key("enter", false), FormAction::Save);
        assert_eq!(f.key("escape", false), FormAction::Cancel);
    }

    #[test]
    fn tab_and_arrows_wrap_around() {
        let mut f = form();
        f.key("up", false);
        assert_eq!(f.field, Field::Kick);
        f.key("tab", false);
        assert_eq!(f.field, Field::Username);
        f.key("tab", true);
        assert_eq!(f.field, Field::Kick);
        f.key("down", false);
        assert_eq!(f.field, Field::Username);
    }

    #[test]
    fn unhandled_keys_are_ignored() {
        let mut f = form();
        assert_eq!(f.key("a", false), FormAction::Ignored);
        assert_eq!(f.key("f3", false), FormAction::Ignored);
        assert_eq!(f.key("space", false), FormAction::Ignored);
        assert_eq!(f.username(), "your_ptt_id");
    }

    #[test]
    fn username_takes_only_ascii_alphanumerics_up_to_limit() {
        let mut f = AccountForm::new("", "", "y");
        f.insert("ab 中文-c\r\n1");
        assert_eq!(f.username(), "abc1");
        f.insert("xxxxxxxxxxxxxxxxxxxx");
        assert_eq!(f.username().len(), MAX_USERNAME);
    }

    #[test]
    fn password_takes_printable_ascii_only() {
        let mut f = AccountForm::new("", "", "y");
        f.key("enter", false);
        f.insert("a b!~密碼\x1b\n\x7f");
        assert_eq!(f.password(), "a b!~");
        f.insert(&"x".repeat(100));
        assert_eq!(f.password().len(), MAX_PASSWORD);
    }

    #[test]
    fn backspace_edits_current_field_only() {
        let mut f = form();
        f.key("backspace", false);
        assert_eq!(f.username(), "your_ptt_i");
        assert_eq!(f.password(), "your_password");
        f.key("enter", false);
        f.key("backspace", false);
        assert_eq!(f.password(), "your_passwor");
        f.key("enter", false);
        assert_eq!(f.key("backspace", false), FormAction::Handled);
        assert_eq!(f.kick_answer(), "y");
        // 空欄位再按不會出錯
        let mut empty = AccountForm::new("", "", "y");
        assert_eq!(empty.key("backspace", false), FormAction::Handled);
        assert_eq!(empty.username(), "");
    }

    #[test]
    fn kick_field_accepts_y_n_and_space() {
        let mut f = form();
        f.key("up", false);
        f.insert("n");
        assert_eq!(f.kick_answer(), "n");
        f.insert("Y");
        assert_eq!(f.kick_answer(), "y");
        f.insert(" ");
        assert_eq!(f.kick_answer(), "n");
        f.insert("中x");
        assert_eq!(f.kick_answer(), "n");
        // 帳密不受影響
        assert_eq!(f.username(), "your_ptt_id");
        assert_eq!(f.password(), "your_password");
    }

    #[test]
    fn overlong_prefill_is_kept_and_shows_tail() {
        // 設定檔手寫的值超過輸入上限：不截斷（存檔才不會改壞），只是不能再加字
        let long = "p".repeat(MAX_PASSWORD + 8);
        let mut f = AccountForm::new("", &long, "y");
        f.key("enter", false);
        f.insert("q");
        assert_eq!(f.password(), long);
        assert_eq!(f.cursor(), (3, LABEL_W + FIELD_W - 1));
    }

    #[test]
    fn cursor_follows_field_and_text() {
        let mut f = AccountForm::new("ab", "", "y");
        assert_eq!(f.cursor(), (2, LABEL_W + 2));
        f.key("enter", false);
        assert_eq!(f.cursor(), (3, LABEL_W));
        f.insert("xyz");
        assert_eq!(f.cursor(), (3, LABEL_W + 3));
        f.key("enter", false);
        assert_eq!(f.cursor(), (4, LABEL_W));
    }

    #[test]
    fn error_shows_until_next_edit() {
        let mut f = form();
        f.set_error("寫入失敗".to_string());
        assert!(f.lines()[6].contains("寫入失敗"));
        f.insert("a");
        assert!(!f.lines()[6].contains("寫入失敗"));
        f.set_error("寫入失敗".to_string());
        f.key("backspace", false);
        assert!(!f.lines()[6].contains("寫入失敗"));
    }

    #[test]
    fn lines_have_fixed_height() {
        assert_eq!(form().lines().len(), HEIGHT);
    }

    #[test]
    fn fit_pads_truncates_and_drops_control_chars() {
        assert_eq!(fit("ab", 4), "ab  ");
        assert_eq!(fit("abcdef", 4), "abcd");
        // 全形字兩格：剩一格放不下就改補空白
        assert_eq!(fit("中文", 3), "中 ");
        assert_eq!(fit("a\x1b[m\nb", 6), "a[mb  ");
    }
}
