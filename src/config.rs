//! 設定檔：`~/.config/ptt-desktop/config.json`，與原專案格式相容。
//! 密碼明文為刻意的取捨（同 ~/.netrc 等級，使用者需自行 chmod 600）。
//! 檔案不存在 → 全用預設值（跳過自動登入）；存在但解析失敗 → 預設值 + 錯誤訊息交由畫面顯示。
//! 程式只在使用者於帳號設定對話框按下儲存時寫檔，且只改帳號三欄。

use std::collections::HashMap;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

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

/// 把帳號三欄併進現有設定檔內容，回傳要寫回的 JSON。其餘欄位（字型、快捷鍵、不認得的欄位）
/// 原樣保留。現有內容解析不了就回錯誤、不產生新內容：覆寫會把使用者手寫的設定弄丟
fn merge_account(
    raw: Option<&str>,
    username: &str,
    password: &str,
    kick_other_logins: &str,
) -> Result<String, String> {
    let mut root = match raw {
        Some(raw) if !raw.trim().is_empty() => serde_json::from_str::<serde_json::Value>(raw)
            .map_err(|e| format!("設定檔解析失敗，未覆寫：{e}"))?,
        _ => serde_json::Value::Object(Default::default()),
    };
    let Some(map) = root.as_object_mut() else {
        return Err("設定檔不是 JSON 物件，未覆寫".to_string());
    };
    map.insert("username".to_string(), username.into());
    map.insert("password".to_string(), password.into());
    map.insert("kickOtherLogins".to_string(), kick_other_logins.into());
    let mut out = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    out.push('\n');
    Ok(out)
}

/// 寫到 `path`：先寫同目錄的暫存檔（權限 600）再 rename，中途失敗不會留下半個設定檔。
/// `path` 是 symlink 時寫到它指向的檔案，不把 symlink 換成一般檔。錯誤訊息不含帳密
fn write_account(
    path: &Path,
    username: &str,
    password: &str,
    kick_other_logins: &str,
) -> Result<(), String> {
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let raw = match std::fs::read_to_string(&target) {
        Ok(raw) => Some(raw),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("讀不到設定檔，未覆寫：{e}")),
    };
    let merged = merge_account(raw.as_deref(), username, password, kick_other_logins)?;
    if let Some(dir) = target.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(|e| format!("建立設定目錄失敗：{e}"))?;
    }
    let tmp = target.with_extension("json.tmp");
    let write = || -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        // 暫存檔若是上次殘留的，`mode` 不會套用，另外收緊一次
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        file.write_all(merged.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&tmp, &target)
    };
    write().map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("寫入設定檔失敗：{e}")
    })
}

/// 帳號設定對話框的儲存：只在使用者按下儲存時呼叫
pub fn save_account(username: &str, password: &str, kick_other_logins: &str) -> Result<(), String> {
    let path = config_path().ok_or_else(|| "找不到 HOME，無法決定設定檔位置".to_string())?;
    write_account(&path, username, password, kick_other_logins)
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

    /// 每個測試各用一個暫存目錄（絕不碰真實設定目錄），結束時刪掉
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("ptt-desktop-test-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn mode_of(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn merge_keeps_other_fields() {
        let raw = r#"{"username":"old_id","password":"old_pw","fontFamily":"Some Font",
            "lineHeight":1.5,"hotkeys":{"F3":"sgossiping\r"},"futureField":[1,2]}"#;
        let out = merge_account(Some(raw), "your_ptt_id", "your_password", "n").unwrap();
        let c = parse(&out).unwrap();
        assert_eq!(c.username, "your_ptt_id");
        assert_eq!(c.password, "your_password");
        assert_eq!(c.kick_other_logins, "n");
        assert_eq!(c.font_family, "Some Font");
        assert_eq!(c.line_height, 1.5);
        assert_eq!(
            c.hotkeys.get("F3").map(String::as_str),
            Some("sgossiping\r")
        );
        // 程式不認得的欄位也要留著
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["futureField"], serde_json::json!([1, 2]));
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn merge_without_existing_file_writes_only_account_fields() {
        for raw in [None, Some(""), Some("  \n")] {
            let out = merge_account(raw, "your_ptt_id", "your_password", "y").unwrap();
            let v: serde_json::Value = serde_json::from_str(&out).unwrap();
            assert_eq!(v.as_object().unwrap().len(), 3);
            assert_eq!(parse(&out).unwrap().username, "your_ptt_id");
        }
    }

    #[test]
    fn merge_refuses_to_overwrite_unparseable_config() {
        assert!(merge_account(Some("{ nope"), "a", "b", "y").is_err());
        assert!(merge_account(Some("[1, 2]"), "a", "b", "y").is_err());
        assert!(merge_account(Some("\"text\""), "a", "b", "y").is_err());
    }

    #[test]
    fn merge_errors_do_not_leak_credentials() {
        let e = merge_account(Some("{ nope"), "your_ptt_id", "your_password", "y").unwrap_err();
        assert!(!e.contains("your_password") && !e.contains("your_ptt_id"));
    }

    #[test]
    fn write_creates_private_file_and_directory() {
        let tmp = TempDir::new("create");
        let path = tmp.0.join("nested").join("config.json");
        write_account(&path, "your_ptt_id", "your_password", "y").unwrap();
        assert_eq!(mode_of(&path), 0o600);
        assert_eq!(mode_of(path.parent().unwrap()), 0o700);
        let c = parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(c.username, "your_ptt_id");
        assert_eq!(c.password, "your_password");
        // 暫存檔不留下
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn write_tightens_loose_permissions_and_keeps_other_fields() {
        let tmp = TempDir::new("rewrite");
        let path = tmp.0.join("config.json");
        std::fs::write(&path, EXAMPLE).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        // 上次殘留、權限過寬的暫存檔
        let stale = path.with_extension("json.tmp");
        std::fs::write(&stale, "stale").unwrap();
        std::fs::set_permissions(&stale, std::fs::Permissions::from_mode(0o644)).unwrap();

        write_account(&path, "other_id", "other_pw", "n").unwrap();
        assert_eq!(mode_of(&path), 0o600);
        let c = parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(c.username, "other_id");
        assert_eq!(c.password, "other_pw");
        assert_eq!(c.kick_other_logins, "n");
        assert_eq!(
            c.hotkeys.get("F3").map(String::as_str),
            Some("sgossiping\r")
        );
    }

    #[test]
    fn write_leaves_broken_config_untouched() {
        let tmp = TempDir::new("broken");
        let path = tmp.0.join("config.json");
        std::fs::write(&path, "{ nope").unwrap();
        assert!(write_account(&path, "your_ptt_id", "your_password", "y").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ nope");
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn write_follows_symlink_instead_of_replacing_it() {
        let tmp = TempDir::new("symlink");
        let real = tmp.0.join("real.json");
        let link = tmp.0.join("config.json");
        std::fs::write(&real, "{}").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        write_account(&link, "your_ptt_id", "your_password", "y").unwrap();
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        let c = parse(&std::fs::read_to_string(&real).unwrap()).unwrap();
        assert_eq!(c.username, "your_ptt_id");
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
