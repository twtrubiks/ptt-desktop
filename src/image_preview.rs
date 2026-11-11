//! 圖片連結 hover 預覽的資料層：判斷連結是否為圖片、以阻塞式 HTTPS 抓圖、嗅探格式。
//! 只處理直連圖檔與 imgur 單圖頁面；不自動掃描畫面（opt-in：滑鼠停住才載圖）。
//! 不帶 Referer（imgur 等圖床對外站 Referer 可能回 403）。
//! 延遲、顯示與定位在 `view.rs`；本模組不持 timer、不碰 UI。

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use gpui::ImageFormat;

/// 抓圖上限（超過直接放棄，避免 hover 到巨圖把記憶體吃光）
const MAX_BYTES: usize = 8 * 1024 * 1024;
/// 動畫 GIF 動輒十幾 MB（PTT 實見 280×497、171 格就有 11.7 MB），另給較寬的上限。
/// 下載時先以這個較大的值為界，嗅探出格式後再依格式檢查
const MAX_GIF_BYTES: usize = 20 * 1024 * 1024;
/// 動畫 GIF 解碼後每一格都是整張畫布的 RGBA，播放時每格都會上傳 GPU 圖集；
/// 總量超過此值就只留第一格靜態顯示。上面那張約 91 MiB，常見的反應圖都在此值內；
/// 這些記憶體只在預覽期間佔用，收掉預覽即釋放
const MAX_GIF_DECODED: usize = 128 * 1024 * 1024;
const MAX_REDIRECTS: usize = 3;
const IO_TIMEOUT: Duration = Duration::from_secs(10);

/// 回傳可直接抓取的圖片網址；非圖片連結回傳 None
pub fn resolve_image_url(uri: &str) -> Option<String> {
    let parsed = parse_url(uri)?;
    let lower_path = parsed.path.to_ascii_lowercase();
    if [".jpg", ".jpeg", ".png", ".gif", ".webp"]
        .iter()
        .any(|ext| lower_path.ends_with(ext))
    {
        return Some(uri.to_string());
    }
    // imgur 單圖頁面 → 直連圖檔；相簿 /a/、/gallery/ 無法改寫，交給點擊開瀏覽器
    if parsed.host == "imgur.com" || parsed.host == "www.imgur.com" {
        let id = parsed.path.strip_prefix('/')?;
        if id.len() >= 5 && id.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Some(format!("https://i.imgur.com/{id}.jpg"));
        }
    }
    None
}

/// 從檔頭嗅探格式（不信任副檔名與 Content-Type）
pub fn sniff_format(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some(ImageFormat::Jpeg)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(ImageFormat::Gif)
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(ImageFormat::Webp)
    } else {
        None
    }
}

/// 阻塞式抓圖（請在背景執行緒呼叫）：跟隨重導向，回傳嗅探到的格式與原始位元組
pub fn fetch(url: &str) -> Result<(ImageFormat, Vec<u8>), String> {
    let mut current = url.to_string();
    for _ in 0..=MAX_REDIRECTS {
        let target = parse_url(&current).ok_or_else(|| format!("無法解析網址 {current}"))?;
        let raw = request(&target)?;
        let resp = parse_response(&raw)?;
        if matches!(resp.status, 301 | 302 | 303 | 307 | 308) {
            let loc = resp
                .header("location")
                .ok_or_else(|| format!("HTTP {} 未附 Location", resp.status))?;
            current = resolve_location(&target, loc);
            continue;
        }
        if resp.status != 200 {
            return Err(format!("HTTP {}", resp.status));
        }
        let format = sniff_format(&resp.body).ok_or("回應不是支援的圖片格式")?;
        if resp.body.len() > size_limit(format) {
            return Err("圖片超過大小上限".to_string());
        }
        let body = if format == ImageFormat::Gif {
            limit_gif_frames(resp.body)
        } else {
            resp.body
        };
        return Ok((format, body));
    }
    Err("重導向次數過多".to_string())
}

fn size_limit(format: ImageFormat) -> usize {
    if format == ImageFormat::Gif {
        MAX_GIF_BYTES
    } else {
        MAX_BYTES
    }
}

// ---- GIF ----

/// GIF 解碼後總量超過 `MAX_GIF_DECODED` 時截成只剩第一格（其後直接接 trailer）；
/// 結構看不懂就原樣交給解碼器處理
fn limit_gif_frames(bytes: Vec<u8>) -> Vec<u8> {
    let Some(scan) = scan_gif(&bytes) else {
        return bytes;
    };
    let canvas = scan.width as usize * scan.height as usize * 4;
    if canvas.saturating_mul(scan.frame_ends.len()) <= MAX_GIF_DECODED {
        return bytes;
    }
    let mut first = bytes[..scan.frame_ends[0]].to_vec();
    first.push(0x3B);
    first
}

struct GifScan {
    /// 邏輯畫布尺寸：解碼器把每一格都合成到這張畫布上
    width: u16,
    height: u16,
    /// 每一格影像資料結束後的位置
    frame_ends: Vec<usize>,
}

/// 只走區塊結構、不解 LZW：數出幾格與每格的結束位置。
/// 資料在區塊邊界結束（缺 trailer）視同結束；區塊中途斷掉或遇到不認得的區塊回傳 None
fn scan_gif(b: &[u8]) -> Option<GifScan> {
    if b.len() < 13 || !(b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a")) {
        return None;
    }
    let width = u16::from_le_bytes([b[6], b[7]]);
    let height = u16::from_le_bytes([b[8], b[9]]);
    let mut pos = 13 + color_table_len(b[10]);
    let mut frame_ends = Vec::new();
    while let Some(&tag) = b.get(pos) {
        match tag {
            // 擴充區塊：0x21、label，之後是 sub-block 串
            0x21 => pos = skip_sub_blocks(b, pos + 2)?,
            // 影像：0x2C、9 bytes 描述（第 9 個是 packed）、區域色盤、LZW 最小碼長，之後是 sub-block 串
            0x2C => {
                let packed = *b.get(pos + 9)?;
                pos = skip_sub_blocks(b, pos + 10 + color_table_len(packed) + 1)?;
                frame_ends.push(pos);
            }
            0x3B => break,
            _ => return None,
        }
    }
    Some(GifScan {
        width,
        height,
        frame_ends,
    })
}

fn color_table_len(packed: u8) -> usize {
    if packed & 0x80 != 0 {
        3 << ((packed & 0x07) + 1)
    } else {
        0
    }
}

/// 跳過一串 sub-block（長度位元組 + 資料，長度 0 結束），回傳結束後的位置
fn skip_sub_blocks(b: &[u8], mut pos: usize) -> Option<usize> {
    loop {
        let len = *b.get(pos)? as usize;
        pos += 1 + len;
        if len == 0 {
            return Some(pos);
        }
    }
}

// ---- URL ----

#[derive(Debug, PartialEq, Eq)]
struct Url {
    tls: bool,
    host: String,
    port: u16,
    /// 路徑（不含 query）
    path: String,
    /// 送給伺服器的 request target（含 query）
    target: String,
}

fn parse_url(uri: &str) -> Option<Url> {
    let (tls, rest) = match uri.strip_prefix("https://") {
        Some(r) => (true, r),
        None => (false, uri.strip_prefix("http://")?),
    };
    let rest = rest.split('#').next().unwrap_or("");
    let authority_end = rest.find(['/', '?']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let mut target = rest[authority_end..].to_string();
    if target.is_empty() || target.starts_with('?') {
        target.insert(0, '/');
    }
    let path = target.split('?').next().unwrap_or("/").to_string();
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !h.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => {
            (h.to_string(), p.parse().ok()?)
        }
        _ => (authority.to_string(), if tls { 443 } else { 80 }),
    };
    if host.is_empty() {
        return None;
    }
    Some(Url {
        tls,
        host: host.to_ascii_lowercase(),
        port,
        path,
        target,
    })
}

fn resolve_location(base: &Url, location: &str) -> String {
    if location.starts_with("http://") || location.starts_with("https://") {
        return location.to_string();
    }
    let scheme = if base.tls { "https" } else { "http" };
    let default_port = if base.tls { 443 } else { 80 };
    let authority = if base.port == default_port {
        base.host.clone()
    } else {
        format!("{}:{}", base.host, base.port)
    };
    if location.starts_with('/') {
        format!("{scheme}://{authority}{location}")
    } else {
        let dir = match base.path.rfind('/') {
            Some(i) => &base.path[..=i],
            None => "/",
        };
        format!("{scheme}://{authority}{dir}{location}")
    }
}

// ---- HTTP ----

fn request(url: &Url) -> Result<Vec<u8>, String> {
    let addr = (url.host.as_str(), url.port)
        .to_socket_addrs()
        .map_err(|e| format!("DNS 失敗：{e}"))?
        .next()
        .ok_or("DNS 無結果")?;
    let stream = TcpStream::connect_timeout(&addr, IO_TIMEOUT).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|e| e.to_string())?;
    let req = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: ptt-gpui/0.2\r\nAccept: image/*\r\nConnection: close\r\n\r\n",
        url.target, url.host
    );
    if url.tls {
        let connector = native_tls::TlsConnector::new().map_err(|e| e.to_string())?;
        let mut tls = connector
            .connect(&url.host, stream)
            .map_err(|e| e.to_string())?;
        talk(&mut tls, req.as_bytes())
    } else {
        let mut plain = stream;
        talk(&mut plain, req.as_bytes())
    }
}

fn talk<S: Read + Write>(s: &mut S, req: &[u8]) -> Result<Vec<u8>, String> {
    s.write_all(req).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    loop {
        let n = s.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > MAX_GIF_BYTES {
            return Err("圖片超過大小上限".to_string());
        }
    }
    Ok(buf)
}

#[derive(Debug)]
struct Response {
    status: u16,
    /// (小寫名稱, 值)
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Response {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

fn parse_response(raw: &[u8]) -> Result<Response, String> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("HTTP 回應不完整")?;
    let head = std::str::from_utf8(&raw[..split]).map_err(|_| "HTTP 標頭非 UTF-8")?;
    let mut lines = head.split("\r\n");
    let status_line = lines.next().ok_or("缺狀態列")?;
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("狀態列無法解析：{status_line}"))?;
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let mut resp = Response {
        status,
        headers,
        body: Vec::new(),
    };
    let body = &raw[split + 4..];
    let chunked = resp
        .header("transfer-encoding")
        .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
    resp.body = if chunked {
        dechunk(body)?
    } else if let Some(len) = resp
        .header("content-length")
        .and_then(|v| v.parse::<usize>().ok())
    {
        body[..len.min(body.len())].to_vec()
    } else {
        body.to_vec() // Connection: close，讀到 EOF 即為完整內容
    };
    Ok(resp)
}

fn dechunk(mut body: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let line_end = body
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or("chunked 資料不完整")?;
        let size_str = std::str::from_utf8(&body[..line_end]).map_err(|_| "chunk 大小非 ASCII")?;
        let size_hex = size_str.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_hex, 16).map_err(|_| "chunk 大小無法解析")?;
        body = &body[line_end + 2..];
        if size == 0 {
            return Ok(out);
        }
        if body.len() < size + 2 {
            return Err("chunk 內容不完整".to_string());
        }
        out.extend_from_slice(&body[..size]);
        body = &body[size + 2..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_image_links_resolve_to_themselves() {
        assert_eq!(
            resolve_image_url("https://example.com/a/b.JPG?x=1").as_deref(),
            Some("https://example.com/a/b.JPG?x=1")
        );
        assert_eq!(
            resolve_image_url("http://x.y/p.webp").as_deref(),
            Some("http://x.y/p.webp")
        );
        assert_eq!(resolve_image_url("https://example.com/a.jpg.html"), None);
    }

    #[test]
    fn imgur_single_image_page_rewrites_to_direct_link() {
        assert_eq!(
            resolve_image_url("https://imgur.com/AbCd123").as_deref(),
            Some("https://i.imgur.com/AbCd123.jpg")
        );
        assert_eq!(
            resolve_image_url("https://www.imgur.com/AbCd123").as_deref(),
            Some("https://i.imgur.com/AbCd123.jpg")
        );
        // 相簿／gallery／太短的 id 不處理
        assert_eq!(resolve_image_url("https://imgur.com/a/AbCd123"), None);
        assert_eq!(resolve_image_url("https://imgur.com/gallery/AbCd123"), None);
        assert_eq!(resolve_image_url("https://imgur.com/abc"), None);
    }

    #[test]
    fn non_http_or_non_image_is_none() {
        assert_eq!(resolve_image_url("ftp://example.com/a.png"), None);
        assert_eq!(
            resolve_image_url("https://www.ptt.cc/bbs/Gossiping/index.html"),
            None
        );
        assert_eq!(resolve_image_url("not a url"), None);
    }

    #[test]
    fn url_parts_split_correctly() {
        let u = parse_url("https://Example.com:8443/dir/pic.png?size=l#frag").unwrap();
        assert_eq!(
            u,
            Url {
                tls: true,
                host: "example.com".to_string(),
                port: 8443,
                path: "/dir/pic.png".to_string(),
                target: "/dir/pic.png?size=l".to_string(),
            }
        );
        let bare = parse_url("http://host").unwrap();
        assert_eq!(
            (bare.port, bare.path.as_str(), bare.target.as_str()),
            (80, "/", "/")
        );
    }

    #[test]
    fn redirect_locations_resolve_against_base() {
        let base = parse_url("https://imgur.com/dir/AbCd123").unwrap();
        assert_eq!(
            resolve_location(&base, "https://i.imgur.com/AbCd123.jpg"),
            "https://i.imgur.com/AbCd123.jpg"
        );
        assert_eq!(
            resolve_location(&base, "/removed.png"),
            "https://imgur.com/removed.png"
        );
        assert_eq!(
            resolve_location(&base, "other.png"),
            "https://imgur.com/dir/other.png"
        );
        let odd = parse_url("http://h:8080/x").unwrap();
        assert_eq!(resolve_location(&odd, "/y"), "http://h:8080/y");
    }

    #[test]
    fn sniffs_formats_by_magic_bytes() {
        assert_eq!(
            sniff_format(b"\x89PNG\r\n\x1a\nrest"),
            Some(ImageFormat::Png)
        );
        assert_eq!(
            sniff_format(b"\xff\xd8\xff\xe0JFIF"),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(sniff_format(b"GIF89a...."), Some(ImageFormat::Gif));
        assert_eq!(
            sniff_format(b"RIFF\x00\x00\x00\x00WEBPVP8 "),
            Some(ImageFormat::Webp)
        );
        assert_eq!(sniff_format(b"<html>"), None);
        assert_eq!(sniff_format(b""), None);
    }

    #[test]
    fn parses_content_length_response() {
        let raw =
            b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 4\r\n\r\nabcdEXTRA";
        let r = parse_response(raw).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.header("content-type"), Some("image/png"));
        assert_eq!(r.body, b"abcd");
    }

    #[test]
    fn parses_chunked_response() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWiki\r\n5;ext=1\r\npedia\r\n0\r\n\r\n";
        let r = parse_response(raw).unwrap();
        assert_eq!(r.body, b"Wikipedia");
    }

    #[test]
    fn truncated_chunked_body_is_an_error() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n10\r\nshort\r\n";
        assert!(parse_response(raw).is_err());
    }

    #[test]
    fn redirect_status_and_location_are_exposed() {
        let raw = b"HTTP/1.1 302 Found\r\nLocation: /elsewhere.jpg\r\n\r\n";
        let r = parse_response(raw).unwrap();
        assert_eq!(r.status, 302);
        assert_eq!(r.header("location"), Some("/elsewhere.jpg"));
        assert!(r.body.is_empty());
    }

    #[test]
    fn missing_header_terminator_is_an_error() {
        assert!(parse_response(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n").is_err());
    }

    // ---- GIF ----

    /// 一格 1×1 影像：GCE（延遲 100ms）＋影像描述（無區域色盤）＋最小合法 LZW 資料
    const FRAME: &[u8] = &[
        0x21, 0xF9, 0x04, 0x00, 0x0A, 0x00, 0x00, 0x00, //
        0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0x00, //
        0x02, 0x02, 0x44, 0x01, 0x00,
    ];

    /// 邏輯畫布 width×height、全域色盤 2 色、NETSCAPE 循環擴充、frames 格、結尾 trailer
    fn gif(width: u16, height: u16, frames: usize) -> Vec<u8> {
        let mut b = b"GIF89a".to_vec();
        b.extend_from_slice(&width.to_le_bytes());
        b.extend_from_slice(&height.to_le_bytes());
        b.extend_from_slice(&[0x80, 0, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF]);
        b.extend_from_slice(&[0x21, 0xFF, 0x0B]);
        b.extend_from_slice(b"NETSCAPE2.0");
        b.extend_from_slice(&[0x03, 0x01, 0x00, 0x00, 0x00]);
        for _ in 0..frames {
            b.extend_from_slice(FRAME);
        }
        b.push(0x3B);
        b
    }

    #[test]
    fn scan_counts_frames_and_reads_canvas_size() {
        let scan = scan_gif(&gif(400, 300, 3)).unwrap();
        assert_eq!(
            (scan.width, scan.height, scan.frame_ends.len()),
            (400, 300, 3)
        );
        // 第一格結束後正好接第二格的 GCE
        let b = gif(400, 300, 3);
        assert_eq!(
            &b[scan.frame_ends[0]..scan.frame_ends[0] + 2],
            &[0x21, 0xF9]
        );
    }

    #[test]
    fn scan_skips_local_color_table() {
        let mut b = gif(1, 1, 0);
        b.pop(); // 拿掉 trailer

        // 影像描述 packed = 0x81：區域色盤 4 色（12 bytes）
        b.extend_from_slice(&[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0x81]);
        b.extend_from_slice(&[0; 12]);
        b.extend_from_slice(&[0x02, 0x02, 0x44, 0x01, 0x00]);
        b.extend_from_slice(FRAME);
        b.push(0x3B);
        assert_eq!(scan_gif(&b).unwrap().frame_ends.len(), 2);
    }

    #[test]
    fn scan_tolerates_missing_trailer_but_not_cut_blocks() {
        let mut no_trailer = gif(1, 1, 2);
        no_trailer.pop();
        assert_eq!(scan_gif(&no_trailer).unwrap().frame_ends.len(), 2);

        let cut = &gif(1, 1, 2)[..40];
        assert!(scan_gif(cut).is_none());
        assert!(scan_gif(b"GIF89a").is_none());
        assert!(scan_gif(b"\x89PNG\r\n\x1a\n.....").is_none());
    }

    #[test]
    fn small_animation_is_left_untouched() {
        let b = gif(400, 300, 50);
        assert_eq!(limit_gif_frames(b.clone()), b);
        // PTT 實見的 280×497、171 格（解碼後約 91 MiB）要能播
        let seen = gif(280, 497, 171);
        assert_eq!(limit_gif_frames(seen.clone()), seen);
    }

    #[test]
    fn oversized_animation_keeps_only_first_frame() {
        // 2000×2000 一格 16 MB：8 格剛好等於上限不動，9 格超過就截成第一格
        let at_limit = gif(2000, 2000, 8);
        assert_eq!(limit_gif_frames(at_limit.clone()), at_limit);

        let cut = limit_gif_frames(gif(2000, 2000, 9));
        // 與直接組出的單格 GIF 逐 byte 相同：表頭、全域色盤、循環擴充都保留，第一格後接 trailer
        assert_eq!(cut, gif(2000, 2000, 1));
    }

    #[test]
    fn gif_gets_a_larger_download_limit_than_still_images() {
        assert_eq!(size_limit(ImageFormat::Gif), MAX_GIF_BYTES);
        for still in [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::Webp] {
            assert_eq!(size_limit(still), MAX_BYTES);
        }
        // 11.7 MB 的 GIF 要能通過，同樣大小的 JPEG 仍擋下
        let seen = 11_655_948;
        assert!(seen <= size_limit(ImageFormat::Gif));
        assert!(seen > size_limit(ImageFormat::Jpeg));
    }

    #[test]
    fn unparseable_gif_is_passed_through() {
        let junk = b"GIF89a\x01\x00\x01\x00\x00\x00\x00\x99garbage".to_vec();
        assert_eq!(limit_gif_frames(junk.clone()), junk);
    }
}
