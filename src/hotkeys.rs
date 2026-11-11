//! 按鍵轉譯：保留的本地鍵（Ctrl+R 重連、F11 全螢幕）→ 設定檔快捷鍵 → 終端機按鍵編碼。
//! 可列印字元**不在此處送出**：GPUI 在 keydown 未被攔下時會把 `key_char` 交給
//! IME input handler（`replace_text_in_range`），中文輸入法的提交文字也走同一條路，
//! 由 view 統一送出，才不會重複送。

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub platform: bool,
}

impl Mods {
    fn any_non_shift(&self) -> bool {
        self.ctrl || self.alt || self.platform
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum KeyAction {
    /// 送出位元組
    Send(Vec<u8>),
    Reconnect,
    ToggleFullscreen,
    /// Ctrl+Shift+C：複製選取文字
    Copy,
    /// Ctrl+Shift+V：貼上剪貼簿文字
    Paste,
    /// 放行（可列印字元由 input handler 送出；其餘忽略）
    Pass,
}

pub struct Hotkeys {
    table: HashMap<String, String>,
}

/// 設定檔沿用瀏覽器 `KeyboardEvent.key` 名稱（F3、PageUp、ArrowUp、?），
/// GPUI 的 key 是小寫短名（f3、pageup、up）；兩邊都正規化到 GPUI 形式
fn normalize(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    match lower.as_str() {
        " " => "space".to_string(),
        "esc" => "escape".to_string(),
        "return" => "enter".to_string(),
        "del" => "delete".to_string(),
        _ => lower.strip_prefix("arrow").unwrap_or(&lower).to_string(),
    }
}

/// 終端機按鍵編碼（PttChrome／xterm 慣例，PTT 皆可辨識）
fn special_key_bytes(key: &str) -> Option<&'static [u8]> {
    Some(match key {
        "enter" => b"\r",
        "backspace" => b"\x7f",
        "tab" => b"\t",
        "escape" => b"\x1b",
        "up" => b"\x1b[A",
        "down" => b"\x1b[B",
        "right" => b"\x1b[C",
        "left" => b"\x1b[D",
        "home" => b"\x1b[1~",
        "insert" => b"\x1b[2~",
        "delete" => b"\x1b[3~",
        "end" => b"\x1b[4~",
        "pageup" => b"\x1b[5~",
        "pagedown" => b"\x1b[6~",
        "f1" => b"\x1bOP",
        "f2" => b"\x1bOQ",
        "f3" => b"\x1bOR",
        "f4" => b"\x1bOS",
        "f5" => b"\x1b[15~",
        "f6" => b"\x1b[17~",
        "f7" => b"\x1b[18~",
        "f8" => b"\x1b[19~",
        "f9" => b"\x1b[20~",
        "f10" => b"\x1b[21~",
        "f11" => b"\x1b[23~",
        "f12" => b"\x1b[24~",
        _ => return None,
    })
}

fn ctrl_byte(key: &str) -> Option<u8> {
    let mut chars = key.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return match key {
            "space" => Some(0x00),
            _ => None,
        };
    }
    match c {
        'a'..='z' => Some(c as u8 & 0x1f),
        '@' => Some(0x00),
        '[' => Some(0x1b),
        '\\' => Some(0x1c),
        ']' => Some(0x1d),
        '^' => Some(0x1e),
        '_' => Some(0x1f),
        _ => None,
    }
}

impl Hotkeys {
    pub fn new(config: &HashMap<String, String>) -> Self {
        Self {
            table: config
                .iter()
                .map(|(k, v)| (normalize(k), v.clone()))
                .collect(),
        }
    }

    /// `key`：GPUI keystroke 的 key；`key_char`：可列印字元（含 shift 後的結果）
    pub fn translate(&self, key: &str, key_char: Option<&str>, mods: Mods) -> KeyAction {
        let key = normalize(key);
        // 保留本地鍵 Ctrl+R = 手動重連（不送給 PTT）
        if mods.ctrl && !mods.alt && !mods.platform && key == "r" {
            return KeyAction::Reconnect;
        }
        // 保留本地鍵 F11 = 全螢幕切換，優先於設定檔快捷鍵
        if key == "f11" && !mods.any_non_shift() && !mods.shift {
            return KeyAction::ToggleFullscreen;
        }
        if mods.any_non_shift() {
            if mods.ctrl && !mods.alt && !mods.platform {
                // 終端機慣例：Ctrl+Shift+C/V 是本地複製貼上（Ctrl+C 仍送 0x03 給 PTT）
                if mods.shift {
                    match key.as_str() {
                        "c" => return KeyAction::Copy,
                        "v" => return KeyAction::Paste,
                        _ => {}
                    }
                }
                if let Some(b) = ctrl_byte(&key) {
                    return KeyAction::Send(vec![b]);
                }
            }
            if mods.alt && !mods.ctrl && !mods.platform {
                // xterm altSendsEscape
                if let Some(ch) = key_char {
                    let mut v = vec![0x1b];
                    v.extend_from_slice(ch.as_bytes());
                    return KeyAction::Send(v);
                }
            }
            return KeyAction::Pass;
        }
        let is_special = key.chars().count() > 1;
        // Shift+特殊鍵（如 Shift+F3）視為不同組合，不觸發 F3 的快捷鍵；
        // 可列印字元不在此限——shift 已反映在 key_char（如 "?"）
        if !(mods.shift && is_special) {
            let hit = key_char
                .and_then(|c| self.table.get(&normalize(c)))
                .or_else(|| self.table.get(&key));
            if let Some(seq) = hit {
                return KeyAction::Send(seq.as_bytes().to_vec());
            }
        }
        if let Some(bytes) = special_key_bytes(&key) {
            return KeyAction::Send(bytes.to_vec());
        }
        KeyAction::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hk(pairs: &[(&str, &str)]) -> Hotkeys {
        Hotkeys::new(
            &pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }

    const NONE: Mods = Mods {
        ctrl: false,
        alt: false,
        shift: false,
        platform: false,
    };
    const CTRL: Mods = Mods { ctrl: true, ..NONE };
    const SHIFT: Mods = Mods {
        shift: true,
        ..NONE
    };

    #[test]
    fn f11_toggles_fullscreen_even_if_configured() {
        let h = hk(&[("F11", "sgossiping\r")]);
        assert_eq!(h.translate("f11", None, NONE), KeyAction::ToggleFullscreen);
    }

    #[test]
    fn f11_with_modifier_is_not_fullscreen() {
        let h = hk(&[]);
        assert_ne!(h.translate("f11", None, CTRL), KeyAction::ToggleFullscreen);
    }

    #[test]
    fn ctrl_r_reconnects() {
        let h = hk(&[]);
        assert_eq!(h.translate("r", None, CTRL), KeyAction::Reconnect);
    }

    #[test]
    fn configured_hotkey_expands() {
        let h = hk(&[("F3", "sgossiping\r")]);
        assert_eq!(
            h.translate("f3", None, NONE),
            KeyAction::Send(b"sgossiping\r".to_vec())
        );
    }

    #[test]
    fn printable_key_passes_to_input_handler() {
        let h = hk(&[]);
        assert_eq!(h.translate("a", Some("a"), NONE), KeyAction::Pass);
        assert_eq!(h.translate("a", Some("A"), SHIFT), KeyAction::Pass);
    }

    #[test]
    fn shift_special_does_not_trigger_hotkey_but_still_encodes() {
        let h = hk(&[("F3", "sgossiping\r")]);
        assert_eq!(
            h.translate("f3", None, SHIFT),
            KeyAction::Send(b"\x1bOR".to_vec())
        );
    }

    #[test]
    fn shifted_printable_hotkey_matches_key_char() {
        let h = hk(&[("?", "h")]);
        assert_eq!(
            h.translate("/", Some("?"), SHIFT),
            KeyAction::Send(b"h".to_vec())
        );
    }

    #[test]
    fn browser_key_names_normalize() {
        let h = hk(&[("PageUp", "p"), ("ArrowUp", "u"), ("Escape", "e")]);
        assert_eq!(
            h.translate("pageup", None, NONE),
            KeyAction::Send(b"p".to_vec())
        );
        assert_eq!(
            h.translate("up", None, NONE),
            KeyAction::Send(b"u".to_vec())
        );
        assert_eq!(
            h.translate("escape", None, NONE),
            KeyAction::Send(b"e".to_vec())
        );
    }

    #[test]
    fn special_keys_encode_for_terminal() {
        let h = hk(&[]);
        assert_eq!(
            h.translate("enter", None, NONE),
            KeyAction::Send(b"\r".to_vec())
        );
        assert_eq!(
            h.translate("up", None, NONE),
            KeyAction::Send(b"\x1b[A".to_vec())
        );
        assert_eq!(
            h.translate("pagedown", None, NONE),
            KeyAction::Send(b"\x1b[6~".to_vec())
        );
        assert_eq!(
            h.translate("backspace", None, NONE),
            KeyAction::Send(vec![0x7f])
        );
    }

    #[test]
    fn ctrl_letters_become_control_bytes() {
        let h = hk(&[]);
        assert_eq!(h.translate("c", None, CTRL), KeyAction::Send(vec![0x03]));
        assert_eq!(h.translate("z", None, CTRL), KeyAction::Send(vec![0x1a]));
        assert_eq!(h.translate("[", None, CTRL), KeyAction::Send(vec![0x1b]));
    }

    #[test]
    fn ctrl_shift_c_v_are_local_copy_paste() {
        let h = hk(&[]);
        let cs = Mods {
            ctrl: true,
            shift: true,
            ..NONE
        };
        assert_eq!(h.translate("c", None, cs), KeyAction::Copy);
        assert_eq!(h.translate("v", None, cs), KeyAction::Paste);
        // 沒按 Shift 仍是送給 PTT 的控制碼
        assert_eq!(h.translate("c", None, CTRL), KeyAction::Send(vec![0x03]));
        assert_eq!(h.translate("v", None, CTRL), KeyAction::Send(vec![0x16]));
    }

    #[test]
    fn alt_sends_escape_prefix() {
        let h = hk(&[]);
        let alt = Mods { alt: true, ..NONE };
        assert_eq!(
            h.translate("x", Some("x"), alt),
            KeyAction::Send(b"\x1bx".to_vec())
        );
    }

    #[test]
    fn unknown_combos_pass() {
        let h = hk(&[]);
        let meta = Mods {
            platform: true,
            ..NONE
        };
        assert_eq!(h.translate("q", Some("q"), meta), KeyAction::Pass);
        assert_eq!(h.translate("capslock", None, NONE), KeyAction::Pass);
    }
}
