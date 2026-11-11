//! PTT 桌面客戶端（GPUI 版）：進入點只做視窗與視圖的組裝。

mod auto_login;
mod config;
mod grid;
mod hotkeys;
mod image_preview;
mod net;
mod palette;
mod reconnect;
mod telnet;
mod urls;
mod view;
mod wcwidth;
mod wcwidth_data;

use gpui::{
    px, size, App, AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions,
};

use crate::view::PttView;

/// 把 gpui 內部的 log 轉到 stderr（預設只出 warn 以上；`PTT_LOG=debug` 可放寬），
/// 診斷視窗／字型／平台問題用；不含任何個資（帳密從不進 log）
struct StderrLogger;

impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::max_level()
    }
    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!(
                "[{}] {}: {}",
                record.level(),
                record.target(),
                record.args()
            );
        }
    }
    fn flush(&self) {}
}

static LOGGER: StderrLogger = StderrLogger;

fn init_logging() {
    let level = match std::env::var("PTT_LOG").as_deref() {
        Ok("trace") => log::LevelFilter::Trace,
        Ok("debug") => log::LevelFilter::Debug,
        Ok("info") => log::LevelFilter::Info,
        _ => log::LevelFilter::Warn,
    };
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(level);
    }
}

fn main() {
    init_logging();
    let loaded = config::load();
    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1000.), px(640.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("PTT".into()),
                    ..Default::default()
                }),
                focus: true,
                app_id: Some("ptt-desktop".into()),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| PttView::new(loaded, window, cx)),
        )
        .expect("開啟視窗失敗");
        // 關閉前的正常斷線在 `PttView`（on_window_should_close）；走到這裡連線已經關好或放棄等待
        cx.on_window_closed(|cx| cx.quit()).detach();
        cx.activate(true);
    });
}
