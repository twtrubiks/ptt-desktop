//! 設定檔：`~/.config/ptt-desktop/config.json`，與原專案格式相容。
//! 密碼明文為刻意的取捨（同 ~/.netrc 等級，使用者需自行 chmod 600）。
//! 檔案不存在 → 全用預設值（跳過自動登入）；存在但解析失敗 → 預設值 + 錯誤訊息交由畫面顯示。

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub username: String,
    pub password: String,
    pub kick_other_logins: String,
    /// GPUI 版自己量測格子，字型不需要修正垂直度量的衍生版；
    /// 預設用系統的 Noto Sans Mono CJK TC（缺字時 GPUI 自行 fallback）
    pub font_family: String,
    /// 與原專案相容保留；GPUI 版字級由視窗大小決定，此欄不使用
    pub font_size: f32,
    /// 行高倍率 1–2：1 = 正方 cell（彩繪方塊無縫）；>1 多出的行距會讓方塊列間露縫
    pub line_height: f32,
    /// 按鍵 → 送出序列，例如 "F3": "sgossiping\r"
    pub hotkeys: HashMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            username: String::new(),
            password: String::new(),
            // 預設踢掉同帳號的其他連線：沒清乾淨的舊連線不會越積越多（手機與電腦同時登入者改 "n"）
            kick_other_logins: "y".to_string(),
            font_family: "Noto Sans Mono CJK TC".to_string(),
            font_size: 18.0,
            line_height: 1.0,
            hotkeys: HashMap::new(),
        }
    }
}

const MAX_LINE_HEIGHT: f32 = 2.0;

pub fn sanitize_line_height(value: f32) -> f32 {
    if !value.is_finite() {
        return 1.0;
    }
    value.clamp(1.0, MAX_LINE_HEIGHT)
}

pub struct LoadResult {
    pub config: Config,
    /// 設定檔存在但解析失敗時的訊息；沒有設定檔為 None（正常情境）
    pub error: Option<String>,
}

pub fn config_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| {
        PathBuf::from(h)
            .join(".config")
            .join("ptt-desktop")
            .join("config.json")
    })
}

pub fn parse(raw: &str) -> Result<Config, String> {
    let mut cfg: Config = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    cfg.line_height = sanitize_line_height(cfg.line_height);
    Ok(cfg)
}

pub fn load() -> LoadResult {
    let Some(path) = config_path() else {
        return LoadResult {
            config: Config::default(),
            error: None,
        };
    };
    let raw = match std::fs::read_to_string(&path) {
        Ok(r) => r,
        Err(_) => {
            return LoadResult {
                config: Config::default(),
                error: None,
            }
        }
    };
    match parse(&raw) {
        Ok(config) => LoadResult {
            config,
            error: None,
        },
        Err(e) => LoadResult {
            config: Config::default(),
            error: Some(e),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 測試一律用 config.example.json 的假資料，絕不碰 ~/.config/ptt-desktop/
    const EXAMPLE: &str = include_str!("../config.example.json");

    #[test]
    fn parses_example_config() {
        let c = parse(EXAMPLE).unwrap();
        assert_eq!(c.username, "your_ptt_id");
        assert_eq!(c.password, "your_password");
        assert_eq!(c.kick_other_logins, "y");
        assert_eq!(c.line_height, 1.0);
        assert_eq!(
            c.hotkeys.get("F3").map(String::as_str),
            Some("sgossiping\r")
        );
    }

    #[test]
    fn missing_fields_use_defaults() {
        let c = parse("{}").unwrap();
        assert_eq!(c, Config::default());
        assert_eq!(c.kick_other_logins, "y");
    }

    #[test]
    fn line_height_is_clamped() {
        assert_eq!(parse(r#"{"lineHeight": 0.5}"#).unwrap().line_height, 1.0);
        assert_eq!(parse(r#"{"lineHeight": 5}"#).unwrap().line_height, 2.0);
        assert_eq!(parse(r#"{"lineHeight": 1.08}"#).unwrap().line_height, 1.08);
        assert_eq!(sanitize_line_height(f32::NAN), 1.0);
    }

    #[test]
    fn broken_json_is_an_error() {
        assert!(parse("{ nope").is_err());
    }

    impl PartialEq for Config {
        fn eq(&self, o: &Self) -> bool {
            self.username == o.username
                && self.password == o.password
                && self.kick_other_logins == o.kick_other_logins
                && self.font_family == o.font_family
                && self.font_size == o.font_size
                && self.line_height == o.line_height
                && self.hotkeys == o.hotkeys
        }
    }
}
