//! WebSocket 連線（tungstenite + native-tls）：帶 Origin header 連 PTT 官方端點，
//! 下行 binary 丟進 channel 交給 UI，並以 telnet NOP 做應用層 keepalive（防閒置踢出）。
//!
//! ws.ptt.cc 不支援 TLS 1.3、對 rustls 預設後端回 protocol_version alert，
//! 需走 native-tls（openssl），實測 TLS 1.2 握手成功（沿用原專案的結論）。
//!
//! 一條連線一個 std thread。握手完成後 socket 改為非阻塞，執行緒以 poll(2) 同時等
//! 「socket 可讀」與「喚醒通道可讀」：UI 送按鍵時把資料放進佇列、再寫一個 byte 到
//! 喚醒通道，執行緒立刻醒來送出。舊做法是 read 以 50 ms 逾時輪詢、逾時才處理上行
//! 佇列，實測按鍵平均多等約 25 ms；閒置時 poll 直接睡到下次 keepalive。
//!
//! 每次醒來都把 `ws.read()` 讀到 WouldBlock 為止：TLS 與 tungstenite 內部可能已緩衝
//! 解好的資料，socket 不會因此再變成可讀，只靠 poll 會漏畫面。tungstenite 對
//! WouldBlock 保留半截 frame 與待送 frame 的狀態，非阻塞不會弄壞串流。
//! 把 `ConnHandle` drop 掉即關閉連線（喚醒通道寫端關閉 → 執行緒醒來看到佇列斷開就收尾）。
//!
//! 關閉一律走正常程序（`close_gracefully`）：送出 WebSocket Close、等伺服器回覆 Close
//! （最多 `CLOSE_HANDSHAKE`），再送 TLS close_notify，socket 關掉之後才回報 `Closed`。
//! UI 收到 `Closed` 時連線確實已經關了，重連與關視窗都靠這一點等舊連線收乾淨。
//!
//! 只支援 Unix（Linux／macOS）：喚醒通道用 `UnixStream::pair`，等待用 `libc::poll`。

use std::io::{self, ErrorKind, Read, Write};
use std::net::TcpStream;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use futures::channel::mpsc::UnboundedSender;
use tungstenite::client::ClientRequestBuilder;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Error, Message, WebSocket};

pub const PTT_WS_URL: &str = "wss://ws.ptt.cc/bbs";
pub const PTT_ORIGIN: &str = "https://term.ptt.cc";
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(60);
/// 首選 telnet NOP（FF F1），不進畫面；備選 NUL（0x00）
const KEEPALIVE_BYTES: &[u8] = &[0xff, 0xf1];
/// 送出緩衝滿時最多等多久可寫；超過視為連線異常
const WRITE_STALL: Duration = Duration::from_secs(10);
/// 一輪最多讀幾則下行訊息：伺服器持續有資料時也要回頭處理上行佇列與 keepalive
const MAX_READS_PER_TURN: usize = 256;
/// 送出 Close 後等伺服器回覆的上限。要比 `view.rs` 的 `CLOSE_WAIT`（3 秒）短，
/// UI 才等得到正常關閉的結果，不會先一步逾時放棄
const CLOSE_HANDSHAKE: Duration = Duration::from_secs(2);

type Ws = WebSocket<MaybeTlsStream<TcpStream>>;

#[derive(Debug)]
pub enum NetEvent {
    Connected,
    Binary(Vec<u8>),
    Closed(String),
}

enum Outgoing {
    Bytes(Vec<u8>),
    Close,
}

pub struct ConnHandle {
    tx: Sender<Outgoing>,
    /// 喚醒通道寫端（非阻塞）
    wake: UnixStream,
}

impl ConnHandle {
    pub fn send(&self, bytes: &[u8]) {
        let _ = self.tx.send(Outgoing::Bytes(bytes.to_vec()));
        self.wake();
    }

    pub fn close(&self) {
        let _ = self.tx.send(Outgoing::Close);
        self.wake();
    }

    /// 通道寫滿（WouldBlock）代表執行緒已有尚未處理的喚醒，忽略即可
    fn wake(&self) {
        let _ = (&self.wake).write(&[1]);
    }
}

/// 開一條新連線；事件經 `events` 回報（UI 以 generation 過濾舊連線的殘留事件）
pub fn connect(events: UnboundedSender<NetEvent>) -> ConnHandle {
    let (tx, rx) = mpsc::channel();
    let (wake, wake_rx) = wake_pair().expect("建立喚醒通道");
    thread::Builder::new()
        .name("ptt-ws".into())
        .spawn(move || run(events, rx, wake_rx))
        .expect("spawn websocket thread");
    ConnHandle { tx, wake }
}

/// 喚醒通道：兩端都非阻塞，UI 寫入不會卡住，執行緒清空時讀到 WouldBlock 即止
fn wake_pair() -> io::Result<(UnixStream, UnixStream)> {
    let (w, r) = UnixStream::pair()?;
    w.set_nonblocking(true)?;
    r.set_nonblocking(true)?;
    Ok((w, r))
}

fn run(events: UnboundedSender<NetEvent>, rx: Receiver<Outgoing>, wake_rx: UnixStream) {
    let closed = |reason: String| {
        let _ = events.unbounded_send(NetEvent::Closed(reason));
    };
    let uri: tungstenite::http::Uri = match PTT_WS_URL.parse() {
        Ok(u) => u,
        Err(e) => return closed(format!("URL 錯誤：{e}")),
    };
    let request = ClientRequestBuilder::new(uri).with_header("Origin", PTT_ORIGIN);
    let (mut ws, _response) = match tungstenite::connect(request) {
        Ok(v) => v,
        Err(e) => return closed(format!("{e}")),
    };
    let sock_fd = match nonblocking_fd(&ws) {
        Ok(fd) => fd,
        Err(e) => return closed(format!("設定 socket 非阻塞失敗：{e}")),
    };
    if events.unbounded_send(NetEvent::Connected).is_err() {
        return;
    }
    let mut last_keepalive = Instant::now();
    loop {
        // 先清喚醒再看佇列：清空後才送進來的資料會留下新的喚醒 byte，不會漏
        drain_wake(&wake_rx);
        // 上行佇列
        loop {
            match rx.try_recv() {
                Ok(Outgoing::Bytes(b)) => {
                    if let Err(e) = send_now(&mut ws, sock_fd, Message::Binary(b.into())) {
                        return closed(format!("{e}"));
                    }
                }
                Ok(Outgoing::Close) | Err(TryRecvError::Disconnected) => {
                    close_gracefully(&mut ws, sock_fd, CLOSE_HANDSHAKE);
                    // 先關 socket 再回報：UI 一收到 Closed 就可能結束程式，不能讓連線關到一半
                    drop(ws);
                    return closed("使用者中斷".to_string());
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        if last_keepalive.elapsed() >= KEEPALIVE_INTERVAL {
            last_keepalive = Instant::now();
            let nop = Message::Binary(KEEPALIVE_BYTES.to_vec().into());
            if let Err(e) = send_now(&mut ws, sock_fd, nop) {
                return closed(format!("{e}"));
            }
        }
        // 下行：讀到 WouldBlock 為止（TLS 1.2 由伺服器發起重新協商時 SSL_read 可能要先寫，
        // 也回 WouldBlock；那時只等 POLLIN 會晚到下次喚醒才重試，PTT 實務上不會發生）
        let mut drained = false;
        for _ in 0..MAX_READS_PER_TURN {
            match ws.read() {
                Ok(Message::Binary(b)) => {
                    if events.unbounded_send(NetEvent::Binary(b.to_vec())).is_err() {
                        return;
                    }
                }
                Ok(Message::Text(t)) => {
                    if events
                        .unbounded_send(NetEvent::Binary(t.as_bytes().to_vec()))
                        .is_err()
                    {
                        return;
                    }
                }
                Ok(Message::Close(frame)) => {
                    let reason = frame
                        .map(|f| f.reason.to_string())
                        .filter(|r| !r.is_empty())
                        .unwrap_or_else(|| "連線已關閉".to_string());
                    return closed(reason);
                }
                // Ping/Pong 由 tungstenite 在 read 時自動回應
                Ok(_) => {}
                Err(Error::Io(e)) if e.kind() == ErrorKind::WouldBlock => {
                    drained = true;
                    break;
                }
                Err(Error::ConnectionClosed) | Err(Error::AlreadyClosed) => {
                    return closed("連線已關閉".to_string());
                }
                Err(e) => return closed(format!("{e}")),
            }
        }
        if !drained {
            continue; // 還有資料沒讀完，不睡，先回頭處理上行
        }
        let until_keepalive = KEEPALIVE_INTERVAL.saturating_sub(last_keepalive.elapsed());
        let mut fds = [
            pollfd(sock_fd, libc::POLLIN),
            pollfd(wake_rx.as_raw_fd(), libc::POLLIN),
        ];
        if let Err(e) = wait(&mut fds, until_keepalive) {
            return closed(format!("poll 失敗：{e}"));
        }
    }
}

/// 握手完成後把 socket 改為非阻塞，回傳 fd 給 poll 用
fn nonblocking_fd(ws: &Ws) -> io::Result<RawFd> {
    let tcp = match ws.get_ref() {
        MaybeTlsStream::Plain(s) => s,
        MaybeTlsStream::NativeTls(t) => t.get_ref(),
        _ => return Err(io::Error::new(ErrorKind::Unsupported, "未支援的串流型別")),
    };
    tcp.set_nonblocking(true)?;
    Ok(tcp.as_raw_fd())
}

/// 送出並確實 flush。送出緩衝滿（按鍵這種小資料幾乎不會）時 tungstenite 保留待送
/// frame 並回 WouldBlock，等可寫再 flush；卡超過 `WRITE_STALL` 視為連線異常。
/// 等待期間不讀下行：雙方緩衝同時滿且伺服器先寫完才讀時會誤判逾時，小資料實務上碰不到
fn send_now(ws: &mut Ws, fd: RawFd, msg: Message) -> Result<(), Error> {
    let deadline = Instant::now() + WRITE_STALL;
    let mut result = ws.send(msg);
    loop {
        match result {
            Err(Error::Io(ref e)) if e.kind() == ErrorKind::WouldBlock => {}
            other => return other,
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(Error::Io(io::Error::new(ErrorKind::TimedOut, "送出逾時")));
        }
        wait(&mut [pollfd(fd, libc::POLLOUT)], left).map_err(Error::Io)?;
        result = ws.flush();
    }
}

/// 正常關閉：送出 WebSocket Close 後繼續讀，直到收到伺服器回覆的 Close（握手完成）、
/// 對方先斷線或超過 `timeout`，最後送 TLS close_notify。這段期間收到的畫面資料直接丟掉，
/// 使用者已經要關了。伺服器不回覆時不能讓關視窗卡住，所以一定有上限
fn close_gracefully(ws: &mut Ws, fd: RawFd, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    let mut unsent = would_block(&ws.close(None));
    let outcome = loop {
        if unsent {
            unsent = would_block(&ws.flush());
        }
        let idle = match ws.read() {
            Ok(Message::Close(_)) => break "伺服器已回覆關閉",
            Ok(_) => false,
            Err(Error::Io(e)) if e.kind() == ErrorKind::WouldBlock => true,
            Err(_) => break "連線已斷，不再等回覆",
        };
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break "等不到伺服器回覆關閉，逾時放棄";
        }
        if idle {
            let events = if unsent {
                libc::POLLIN | libc::POLLOUT
            } else {
                libc::POLLIN
            };
            if let Err(e) = wait(&mut [pollfd(fd, events)], left) {
                log::warn!("[net] 關閉時 poll 失敗：{e}");
                break "poll 失敗";
            }
        }
    };
    log::info!("[net] {outcome}");
    if let MaybeTlsStream::NativeTls(tls) = ws.get_mut() {
        let _ = tls.shutdown();
    }
}

/// 送出緩衝滿、資料還留在 tungstenite 裡等下次 flush
fn would_block<T>(result: &Result<T, Error>) -> bool {
    matches!(result, Err(Error::Io(e)) if e.kind() == ErrorKind::WouldBlock)
}

/// 清空喚醒通道（內容無意義，只用來叫醒 poll）；寫端關閉時讀到 0，同樣停止
fn drain_wake(mut r: &UnixStream) {
    let mut buf = [0u8; 64];
    while matches!(r.read(&mut buf), Ok(n) if n > 0) {}
}

fn pollfd(fd: RawFd, events: libc::c_short) -> libc::pollfd {
    libc::pollfd {
        fd,
        events,
        revents: 0,
    }
}

/// poll(2) 等任一 fd 就緒或逾時。被訊號打斷視同醒來，外層迴圈會重新檢查一切
fn wait(fds: &mut [libc::pollfd], timeout: Duration) -> io::Result<()> {
    // 無條件進位到毫秒：截斷成 0 會讓 keepalive 前最後不到 1 ms 變成空轉
    let ms = timeout.as_micros().div_ceil(1000).min(i32::MAX as u128) as libc::c_int;
    // SAFETY: fds 是有效、獨占的 pollfd 切片，nfds 與其長度一致
    let r = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, ms) };
    if r < 0 {
        let e = io::Error::last_os_error();
        if e.kind() != ErrorKind::Interrupted {
            return Err(e);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle() -> (ConnHandle, Receiver<Outgoing>, UnixStream) {
        let (tx, rx) = mpsc::channel();
        let (wake, wake_rx) = wake_pair().unwrap();
        (ConnHandle { tx, wake }, rx, wake_rx)
    }

    fn wait_wake(wake_rx: &UnixStream, timeout: Duration) -> Duration {
        let t = Instant::now();
        wait(&mut [pollfd(wake_rx.as_raw_fd(), libc::POLLIN)], timeout).unwrap();
        t.elapsed()
    }

    #[test]
    fn send_wakes_a_waiting_thread_immediately() {
        let (h, rx, wake_rx) = handle();
        let waiter = thread::spawn(move || (wait_wake(&wake_rx, Duration::from_secs(10)), wake_rx));
        thread::sleep(Duration::from_millis(20));
        h.send(b"a");
        let (elapsed, wake_rx) = waiter.join().unwrap();
        assert!(
            elapsed < Duration::from_secs(5),
            "沒被喚醒，等到逾時：{elapsed:?}"
        );
        drain_wake(&wake_rx);
        assert!(matches!(rx.try_recv(), Ok(Outgoing::Bytes(b)) if b == b"a"));
    }

    #[test]
    fn drained_channel_does_not_wake_again() {
        let (h, _rx, wake_rx) = handle();
        h.send(b"a");
        h.send(b"b");
        drain_wake(&wake_rx);
        let elapsed = wait_wake(&wake_rx, Duration::from_millis(50));
        assert!(
            elapsed >= Duration::from_millis(40),
            "清空後仍被舊喚醒叫醒：{elapsed:?}"
        );
    }

    #[test]
    fn wake_never_blocks_the_ui_even_when_channel_is_full() {
        let (h, _rx, _wake_rx) = handle();
        let t = Instant::now();
        for _ in 0..10_000 {
            h.send(b"x");
        }
        assert!(t.elapsed() < Duration::from_secs(10));
    }

    /// 本機 WebSocket 伺服器（`server` 在另一條執行緒跑）配一條已握手、非阻塞的客戶端連線，
    /// 客戶端型別與正式連線相同，只是不走 TLS
    fn local_pair<T: Send + 'static>(
        server: impl FnOnce(WebSocket<TcpStream>) -> T + Send + 'static,
    ) -> (Ws, RawFd, thread::JoinHandle<T>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let srv = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            server(tungstenite::accept(stream).unwrap())
        });
        let tcp = TcpStream::connect(addr).unwrap();
        let (ws, _) =
            tungstenite::client::client(format!("ws://{addr}/"), MaybeTlsStream::Plain(tcp))
                .map_err(|e| e.to_string())
                .unwrap();
        let fd = nonblocking_fd(&ws).unwrap();
        (ws, fd, srv)
    }

    #[test]
    fn close_waits_for_server_reply_and_discards_pending_data() {
        let (mut ws, fd, srv) = local_pair(|mut ws| {
            ws.send(Message::Binary(b"screen".to_vec().into())).unwrap();
            let mut got_close = false;
            loop {
                match ws.read() {
                    Ok(Message::Close(_)) => got_close = true,
                    Ok(_) => {}
                    Err(_) => return got_close, // 回完 Close 後伺服器端關掉連線
                }
            }
        });
        let t = Instant::now();
        close_gracefully(&mut ws, fd, Duration::from_secs(5));
        let elapsed = t.elapsed();
        assert!(srv.join().unwrap(), "伺服器沒收到 Close frame");
        assert!(
            elapsed < Duration::from_secs(2),
            "伺服器有回覆卻等到逾時：{elapsed:?}"
        );
    }

    #[test]
    fn close_gives_up_after_timeout_when_server_never_replies() {
        let (done_tx, done_rx) = mpsc::channel::<()>();
        // 伺服器握手後就不讀不回，連線一直開著，直到客戶端那邊測完
        let (mut ws, fd, srv) = local_pair(move |ws| {
            let _ = done_rx.recv();
            drop(ws);
        });
        let t = Instant::now();
        close_gracefully(&mut ws, fd, Duration::from_millis(200));
        let elapsed = t.elapsed();
        done_tx.send(()).unwrap();
        srv.join().unwrap();
        assert!(
            elapsed >= Duration::from_millis(150),
            "沒等回覆就走了：{elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "逾時沒有生效：{elapsed:?}"
        );
    }

    #[test]
    fn close_returns_at_once_when_server_already_hung_up() {
        let (mut ws, fd, srv) = local_pair(drop);
        srv.join().unwrap(); // 伺服器握手完直接關掉 TCP，沒有 Close frame
        let t = Instant::now();
        close_gracefully(&mut ws, fd, Duration::from_secs(5));
        let elapsed = t.elapsed();
        assert!(
            elapsed < Duration::from_secs(2),
            "對方已斷線還在等回覆：{elapsed:?}"
        );
    }

    #[test]
    fn dropping_handle_wakes_thread_and_disconnects_queue() {
        let (h, rx, wake_rx) = handle();
        drop(h);
        let elapsed = wait_wake(&wake_rx, Duration::from_secs(10));
        assert!(
            elapsed < Duration::from_secs(5),
            "寫端關閉沒叫醒：{elapsed:?}"
        );
        drain_wake(&wake_rx); // 讀到 EOF 必須停，不能空轉
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Disconnected)));
    }
}
