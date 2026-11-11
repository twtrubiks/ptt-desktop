//! byte 網格終端機模型：80×24、PTT 的「一欄半字」語意。
//!
//! 這個網格是**唯一的**畫面模型：半字覆寫、SGR 劈字（左右半異色）都是網格的原生語意，
//! 不需要另外的修正層。
//!
//! 語意規格見 `docs/architecture.md`「byte 網格」：
//! - 每欄獨立持有窄字元或寬字元的左半／右半（含該欄自己的 SGR）
//! - 覆寫只動被寫的欄，partner 欄降為孤兒半字（保留字形與左右半記號）
//! - big5 模式一欄一 byte（lead 0x81–0xFE + trail），utf-8 模式一欄為整字之半；
//!   SGR 可劈在字中間（左半取字首當下屬性、右半取字尾當下屬性）
//! - 末列 LF／自動換行上捲、頂列 `ESC M` 下捲（兩個方向都要）
//! - 行尾虛位（col=80）的 BS 為純減一（xterm 的 pending-wrap 語意分歧在此不存在）
//! - 沒實作的序列記進 `unknown_sequences` 讓它亮紅燈，不要靜默忽略

use std::collections::BTreeSet;

use crate::wcwidth::wcwidth;

pub const ROWS: usize = 24;
pub const COLS: usize = 80;
const CELLS: usize = ROWS * COLS;

/// 未知序列樣態上限：診斷用，只要看得到「有哪幾種」就夠
const MAX_UNKNOWN: usize = 16;

pub const ATTR_BOLD: u16 = 1 << 0;
pub const ATTR_BLINK: u16 = 1 << 1;
pub const ATTR_INVERSE: u16 = 1 << 2;
pub const ATTR_UNDERLINE: u16 = 1 << 11;
const FG_SHIFT: u16 = 3; // bit3-6 fg+1（0=預設）
const BG_SHIFT: u16 = 7; // bit7-10 bg+1
const COLOR_MASK: u16 = 0xf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    /// 完整字（寬字兩欄同值）；窄字元本身；未配對 big5 byte 為 '\0'
    pub glyph: char,
    /// 0=窄/空, 1=寬字左半, 2=寬字右半
    pub half: u8,
    /// big5 模式未配對 byte 值，其餘 -1
    pub byte: i16,
    pub attr: u16,
}

impl Cell {
    pub const BLANK: Cell = Cell {
        glyph: ' ',
        half: 0,
        byte: -1,
        attr: 0,
    };

    /// 0–7，-1 = 預設
    pub fn fg(&self) -> i8 {
        (((self.attr >> FG_SHIFT) & COLOR_MASK) as i8) - 1
    }
    pub fn bg(&self) -> i8 {
        (((self.attr >> BG_SHIFT) & COLOR_MASK) as i8) - 1
    }
    pub fn bold(&self) -> bool {
        self.attr & ATTR_BOLD != 0
    }
    pub fn blink(&self) -> bool {
        self.attr & ATTR_BLINK != 0
    }
    pub fn inverse(&self) -> bool {
        self.attr & ATTR_INVERSE != 0
    }
    pub fn underline(&self) -> bool {
        self.attr & ATTR_UNDERLINE != 0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EscState {
    Text,
    Esc,
    Csi,
}

pub struct Grid {
    cells: Vec<Cell>,
    row: usize,
    /// 可為 COLS（行尾虛位，下一個可印字元才換行）
    col: usize,
    attr: u16,
    utf8: bool,

    esc: EscState,
    csi: String,

    // UTF-8 多位元組累積（可跨 chunk；中間夾 ESC 序列不中斷——SGR 劈字）
    u8_need: u8,
    u8_have: u8,
    u8_cp: u32,
    /// 字元第一個 byte 當下的屬性（左半用）
    u8_start_attr: u16,

    // big5 lead byte 等待 trail 配對（SGR 不中斷、游標移動即失效）
    lead_idx: Option<usize>,
    lead_byte: u8,

    /// ESC 7 / CSI s 保存的 (row, col, attr)
    saved: Option<(usize, usize, u16)>,

    /// DECAWM（CSI ?7 h/l）：PTT 連線時即送 `ESC[?7l` 關閉自動換行，之後寫到第 80 欄
    /// 不換行、停在末欄覆寫（xterm 同此語意）；關掉後 PTT 的 \r\n 全是明送的
    autowrap: bool,
    /// DECTCEM（CSI ?25 h/l）游標可見
    cursor_visible: bool,
    /// 要回送給伺服器的回應（DSR `ESC[6n` → `ESC[row;colR`，xterm.js 會自動回覆，PTT 連線時會查）
    responses: Vec<u8>,

    /// 本狀態機未實作的序列樣態。reset() 不清空——診斷價值在整個 session 累積
    pub unknown_sequences: BTreeSet<String>,
}

impl Default for Grid {
    fn default() -> Self {
        Self::new()
    }
}

fn seq_name(prefix: &str, b: u8) -> String {
    if (0x20..0x7f).contains(&b) {
        format!("{prefix} {}", b as char)
    } else {
        format!("{prefix} 0x{b:02x}")
    }
}

impl Grid {
    pub fn new() -> Self {
        Self {
            cells: vec![Cell::BLANK; CELLS],
            row: 0,
            col: 0,
            attr: 0,
            utf8: false,
            esc: EscState::Text,
            csi: String::new(),
            u8_need: 0,
            u8_have: 0,
            u8_cp: 0,
            u8_start_attr: 0,
            lead_idx: None,
            lead_byte: 0,
            saved: None,
            autowrap: true,
            cursor_visible: true,
            responses: Vec::new(),
            unknown_sequences: BTreeSet::new(),
        }
    }

    /// 回到 big5 模式並清空網格（重連時機）
    pub fn reset(&mut self) {
        self.cells.fill(Cell::BLANK);
        self.row = 0;
        self.col = 0;
        self.attr = 0;
        self.esc = EscState::Text;
        self.csi.clear();
        self.saved = None;
        self.autowrap = true;
        self.cursor_visible = true;
        self.responses.clear();
        self.set_utf8(false);
    }

    pub fn cursor_visible(&self) -> bool {
        self.cursor_visible
    }

    /// 取走累積的終端回應（呼叫端送回伺服器）
    pub fn take_responses(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.responses)
    }

    /// 切換解碼模式但保留畫面（新舊 PTT 雙軌切換時機由 auto-login 位元組層偵測驅動；
    /// 舊版「密碼不對」撥回 big5 時畫面不該被清掉——錯誤訊息和登入畫面要一起可讀）
    pub fn set_utf8(&mut self, utf8: bool) {
        self.utf8 = utf8;
        self.u8_need = 0;
        self.u8_have = 0;
        self.u8_cp = 0;
        self.lead_idx = None;
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_utf8(&self) -> bool {
        self.utf8
    }

    /// 餵一批下行 bytes（telnet 濾層後）
    pub fn feed(&mut self, data: &[u8]) {
        for &b in data {
            self.step(b);
        }
    }

    /// 以當前模式的編碼把文字寫進畫面（本地訊息用：連線中、斷線、登入中止）
    pub fn write_text(&mut self, text: &str) {
        if self.utf8 {
            self.feed(text.as_bytes());
        } else {
            let (bytes, _, _) = encoding_rs::BIG5.encode(text);
            self.feed(&bytes);
        }
    }

    /// 游標（col 夾到 79）
    pub fn cursor(&self) -> (usize, usize) {
        (self.row, self.col.min(COLS - 1))
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn cell(&self, row: usize, col: usize) -> &Cell {
        &self.cells[row * COLS + col]
    }

    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    pub fn row_cells(&self, row: usize) -> &[Cell] {
        &self.cells[row * COLS..(row + 1) * COLS]
    }

    pub fn has_blink(&self) -> bool {
        self.cells.iter().any(|c| c.attr & ATTR_BLINK != 0)
    }

    /// half≠0 且相鄰欄構成同一個字
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn paired_at(&self, i: usize) -> bool {
        paired_in(&self.cells, i)
    }

    /// 配對後的一列內容（內容檢視／測試用；孤兒與未配對 byte 以 ? 呈現）
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn row_text(&self, row: usize) -> String {
        let mut out = String::new();
        let mut c = 0;
        while c < COLS {
            let i = row * COLS + c;
            let cell = self.cells[i];
            if self.paired_at(i) {
                out.push(cell.glyph);
                c += 2;
            } else if cell.half != 0 || cell.byte >= 0x80 {
                out.push('?');
                c += 1;
            } else {
                out.push(if cell.byte >= 0 {
                    cell.byte as u8 as char
                } else {
                    cell.glyph
                });
                c += 1;
            }
        }
        out
    }

    /// 選取範圍（含兩端、列優先順序）的純文字：每列去尾端空白，列間以 `\n` 相接。
    /// 寬字左半被選到即整字納入；右半單獨被選到不重複輸出；未配對的 Big5 位元組略過
    pub fn text_in_range(&self, start: (usize, usize), end: (usize, usize)) -> String {
        let (a, b) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let mut out = String::new();
        for row in a.0..=b.0.min(ROWS - 1) {
            let c0 = if row == a.0 { a.1 } else { 0 };
            let c1 = if row == b.0 {
                b.1.min(COLS - 1)
            } else {
                COLS - 1
            };
            let mut line = String::new();
            let mut c = c0;
            while c <= c1 {
                let i = row * COLS + c;
                let cell = self.cells[i];
                if self.paired_at(i) {
                    line.push(cell.glyph);
                    c += 2;
                } else if cell.half == 2 && c == c0 && c > 0 && self.paired_at(i - 1) {
                    // 選取從寬字右半開始：補上整字
                    line.push(self.cells[i - 1].glyph);
                    c += 1;
                } else if cell.half != 0 || cell.byte >= 0x80 {
                    c += 1;
                } else {
                    line.push(if cell.byte >= 0 {
                        cell.byte as u8 as char
                    } else {
                        cell.glyph
                    });
                    c += 1;
                }
            }
            let trimmed = line.trim_end();
            out.push_str(trimmed);
            if row != b.0.min(ROWS - 1) {
                out.push('\n');
            }
        }
        out
    }

    // ---- byte 層狀態機 ----

    fn step(&mut self, b: u8) {
        match self.esc {
            EscState::Esc => {
                self.esc = EscState::Text;
                match b {
                    b'[' => {
                        self.esc = EscState::Csi;
                        self.csi.clear();
                    }
                    // RI：文章往上捲頁（ESC[24;1H ESC[J ESC[H ESC M）
                    b'M' => {
                        self.reverse_index();
                        self.lead_idx = None;
                    }
                    b'7' => self.save_cursor(),
                    b'8' => self.restore_cursor(),
                    // 其餘非 CSI 的 ESC 序列視為兩 byte 結束。注意：本狀態機不認得
                    // OSC/DCS/APC 等字串序列；PTT 實錄不送，若出現會在此亮紅燈
                    _ => self.note_unknown(seq_name("ESC", b)),
                }
                return;
            }
            EscState::Csi => {
                if (0x30..=0x3f).contains(&b) {
                    self.csi.push(b as char);
                    return;
                }
                if (0x20..=0x2f).contains(&b) {
                    return; // intermediate bytes，PTT 不用，吃掉
                }
                self.esc = EscState::Text;
                if (0x40..=0x7e).contains(&b) {
                    self.dispatch_csi(b);
                    return;
                }
                // 非法終止 byte：放回一般路徑
            }
            EscState::Text => {}
        }
        match b {
            // UTF-8 累積與 big5 lead 都保留——SGR 可劈在字中間
            0x1b => self.esc = EscState::Esc,
            0x0d => {
                self.col = 0;
                self.lead_idx = None;
            }
            // VT／FF 在 xterm 等同 LF
            0x0a..=0x0c => {
                self.line_feed();
                self.lead_idx = None;
            }
            // 行尾虛位（col=80）採 byte 網格意圖「純減一」（→79）
            0x08 => {
                self.col = self.col.min(COLS).saturating_sub(1);
                self.lead_idx = None;
            }
            0x09 => {
                self.col = (((self.col.min(COLS - 1)) / 8 + 1) * 8).min(COLS - 1);
                self.lead_idx = None;
            }
            // NUL／BEL／DEL 不影響畫面
            0x00 | 0x07 | 0x7f => {}
            b if b < 0x20 => self.note_unknown(seq_name("C0", b)),
            _ => {
                if self.utf8 {
                    self.step_utf8(b);
                } else {
                    self.step_big5(b);
                }
            }
        }
    }

    fn step_utf8(&mut self, b: u8) {
        if self.u8_need > 0 {
            if (0x80..=0xbf).contains(&b) {
                self.u8_cp = (self.u8_cp << 6) | u32::from(b & 0x3f);
                self.u8_have += 1;
                if self.u8_have == self.u8_need {
                    let cp = self.u8_cp;
                    self.u8_need = 0;
                    self.emit_char(cp);
                }
                return;
            }
            // 序列斷裂：以 U+FFFD 呈現（與 TextDecoder non-fatal 同精神），原 byte 重新處理
            self.u8_need = 0;
            self.emit_char(0xfffd);
            self.step_utf8(b);
            return;
        }
        if b < 0x80 {
            self.u8_start_attr = self.attr;
            self.emit_char(u32::from(b));
            return;
        }
        match b {
            0xc2..=0xdf => self.start_u8(u32::from(b & 0x1f), 2),
            0xe0..=0xef => self.start_u8(u32::from(b & 0x0f), 3),
            0xf0..=0xf4 => self.start_u8(u32::from(b & 0x07), 4),
            _ => {
                self.u8_start_attr = self.attr;
                self.emit_char(0xfffd); // 孤兒 continuation / 非法 starter
            }
        }
    }

    fn start_u8(&mut self, bits: u32, need: u8) {
        self.u8_cp = bits;
        self.u8_need = need;
        self.u8_have = 1;
        self.u8_start_attr = self.attr; // 左半的屬性在字元起點取樣
    }

    fn step_big5(&mut self, b: u8) {
        // trail 配對：lead 之後（允許夾 SGR）下一個資料 byte 落在 lead 右鄰欄
        if let Some(li) = self.lead_idx {
            if ((0x40..=0x7e).contains(&b) || (0xa1..=0xfe).contains(&b))
                && self.row * COLS + self.col == li + 1
                && self.col < COLS
            {
                let glyph = decode_big5_pair(self.lead_byte, b);
                self.lead_idx = None;
                // lead 欄的屬性維持 lead byte 寫入當下的值（SGR 劈字＝左右半異色）
                self.cells[li].glyph = glyph;
                self.cells[li].half = 1;
                self.cells[li].byte = -1;
                self.cells[li + 1] = Cell {
                    glyph,
                    half: 2,
                    byte: -1,
                    attr: self.attr,
                };
                self.col += 1;
                return;
            }
        }
        self.lead_idx = None;
        if b < 0x80 {
            self.put_narrow(b as char, self.attr);
            return;
        }
        // 未配對的 big5 byte：byte 網格語意，先佔一欄；0x81-0xFE 掛起等 trail
        self.wrap_if_needed(1);
        let i = self.row * COLS + self.col;
        self.cells[i] = Cell {
            glyph: '\0',
            half: 0,
            byte: i16::from(b),
            attr: self.attr,
        };
        if b >= 0x81 {
            self.lead_idx = Some(i);
            self.lead_byte = b;
        }
        self.col += 1;
    }

    fn emit_char(&mut self, cp: u32) {
        let ch = char::from_u32(cp).unwrap_or('\u{fffd}');
        let w = wcwidth(ch as u32);
        if w == 0 {
            return; // 組合字元：PTT 不會送，忽略
        }
        if w == 1 {
            self.put_narrow(ch, self.u8_start_attr);
        } else {
            // SGR 劈字時右半取字尾當下的屬性
            self.put_wide(ch, self.u8_start_attr, self.attr);
        }
    }

    fn wrap_if_needed(&mut self, width: usize) {
        if self.col + width > COLS {
            if self.autowrap {
                self.line_feed();
                self.col = 0;
            } else {
                self.col = COLS - width; // 不換行：停在末欄覆寫
            }
        }
    }

    fn put_narrow(&mut self, ch: char, attr: u16) {
        self.wrap_if_needed(1);
        let i = self.row * COLS + self.col;
        self.cells[i] = Cell {
            glyph: ch,
            half: 0,
            byte: -1,
            attr,
        };
        self.col += 1;
    }

    fn put_wide(&mut self, ch: char, attr_l: u16, attr_r: u16) {
        self.wrap_if_needed(2);
        let i = self.row * COLS + self.col;
        self.cells[i] = Cell {
            glyph: ch,
            half: 1,
            byte: -1,
            attr: attr_l,
        };
        self.cells[i + 1] = Cell {
            glyph: ch,
            half: 2,
            byte: -1,
            attr: attr_r,
        };
        self.col += 2;
    }

    /// LF／自動換行：末列時整個網格上捲一列（scrollback 0 → 捲出即丟）。
    /// PTT 往下捲頁正是「CUP 到 24 列 + 連續 LF」
    fn line_feed(&mut self) {
        if self.row < ROWS - 1 {
            self.row += 1;
            return;
        }
        self.cells.copy_within(COLS.., 0);
        self.clear_row(ROWS - 1);
        self.shift_pending(-(COLS as isize));
    }

    /// RI（ESC M）：頂列時整個網格下捲一列、頂列清空
    fn reverse_index(&mut self) {
        if self.row > 0 {
            self.row -= 1;
            return;
        }
        self.cells.copy_within(0..(ROWS - 1) * COLS, COLS);
        self.clear_row(0);
        self.shift_pending(COLS as isize);
    }

    fn clear_row(&mut self, row: usize) {
        let blank = Cell {
            attr: self.attr,
            ..Cell::BLANK
        };
        self.cells[row * COLS..(row + 1) * COLS].fill(blank);
    }

    /// 捲動時掛起的 big5 lead 隨之平移，移出畫面者作廢
    fn shift_pending(&mut self, delta: isize) {
        self.lead_idx = self.lead_idx.and_then(|li| {
            let j = li as isize + delta;
            (0..CELLS as isize).contains(&j).then_some(j as usize)
        });
    }

    fn save_cursor(&mut self) {
        self.saved = Some((self.row, self.col.min(COLS - 1), self.attr));
    }

    fn restore_cursor(&mut self) {
        if let Some((r, c, a)) = self.saved {
            self.row = r;
            self.col = c;
            self.attr = a;
        } else {
            self.row = 0;
            self.col = 0;
            self.attr = 0;
        }
        self.lead_idx = None;
    }

    fn dispatch_csi(&mut self, fin: u8) {
        if let Some(private) = self.csi.strip_prefix('?') {
            let modes: Vec<usize> = private.split(';').filter_map(|s| s.parse().ok()).collect();
            return self.dispatch_private(fin, &modes);
        }
        let params: Vec<Option<usize>> = self.csi.split(';').map(|s| s.parse().ok()).collect();
        let p0 = params.first().copied().flatten().unwrap_or(0);
        let n = p0.max(1);
        match fin {
            b'H' | b'f' => {
                let p1 = params.get(1).copied().flatten().unwrap_or(0);
                self.row = (n - 1).min(ROWS - 1);
                self.col = (p1.max(1) - 1).min(COLS - 1);
                self.lead_idx = None;
            }
            b'A' => {
                self.row = self.row.saturating_sub(n);
                self.lead_idx = None;
            }
            b'B' => {
                self.row = (self.row + n).min(ROWS - 1);
                self.lead_idx = None;
            }
            b'C' => {
                self.col = (self.col.min(COLS - 1) + n).min(COLS - 1);
                self.lead_idx = None;
            }
            b'D' => {
                self.col = self.col.min(COLS).saturating_sub(n);
                self.lead_idx = None;
            }
            b'J' => {
                let cur = self.row * COLS + self.col.min(COLS - 1);
                match p0 {
                    2 => self.erase(0, CELLS),
                    1 => self.erase(0, cur + 1),
                    _ => self.erase(cur, CELLS),
                }
                self.lead_idx = None;
            }
            b'K' => {
                let base = self.row * COLS;
                let cur = self.col.min(COLS - 1);
                match p0 {
                    2 => self.erase(base, base + COLS),
                    1 => self.erase(base, base + cur + 1),
                    _ => self.erase(base + cur, base + COLS),
                }
                self.lead_idx = None;
            }
            b'm' => self.apply_sgr(),
            b's' => self.save_cursor(),
            b'u' => self.restore_cursor(),
            // DSR：6 = 回報游標位置（1-based）；其餘（5 = 狀態）回 OK
            b'n' => match p0 {
                6 => {
                    let (r, c) = self.cursor();
                    self.responses
                        .extend_from_slice(format!("\x1b[{};{}R", r + 1, c + 1).as_bytes());
                }
                5 => self.responses.extend_from_slice(b"\x1b[0n"),
                _ => self.note_unknown(seq_name("CSI", fin)),
            },
            // 其餘 CSI 吃掉（PTT 實錄只送 m/H/K/J）。只記 final byte
            _ => self.note_unknown(seq_name("CSI", fin)),
        }
    }

    /// CSI ? … h/l 私有模式：只認 PTT 會送的 7（自動換行）與 25（游標可見），其餘記錄
    fn dispatch_private(&mut self, fin: u8, modes: &[usize]) {
        let set = match fin {
            b'h' => true,
            b'l' => false,
            _ => return self.note_unknown(seq_name("CSI ?", fin)),
        };
        for &m in modes {
            match m {
                7 => self.autowrap = set,
                25 => self.cursor_visible = set,
                // 實機登入時 PTT 會送：關閉 xterm 滑鼠回報（1000/1002/1003/1006）與
                // 同步更新（2026）。本客戶端不做滑鼠回報、每個 chunk 本就整批上畫面，
                // 兩者都是已知且無事可做，不當未知序列亮燈
                1000..=1006 | 2026 => {}
                _ => self.note_unknown(format!("CSI ?{m} {}", fin as char)),
            }
        }
    }

    fn note_unknown(&mut self, name: String) {
        if self.unknown_sequences.len() < MAX_UNKNOWN {
            self.unknown_sequences.insert(name);
        }
    }

    /// BCE：以當前屬性清（背景色沿用）。切在 pair 中間時只清範圍內半邊，
    /// 範圍外的 partner 自然成孤兒半字
    fn erase(&mut self, from: usize, to: usize) {
        let blank = Cell {
            attr: self.attr,
            ..Cell::BLANK
        };
        self.cells[from..to].fill(blank);
    }

    fn apply_sgr(&mut self) {
        let parts: Vec<&str> = if self.csi.is_empty() {
            vec![""]
        } else {
            self.csi.split(';').collect()
        };
        let mut attr = self.attr;
        for part in parts {
            let n: u16 = if part.is_empty() {
                0
            } else {
                match part.parse() {
                    Ok(v) => v,
                    Err(_) => continue,
                }
            };
            match n {
                0 => attr = 0,
                1 => attr |= ATTR_BOLD,
                22 => attr &= !ATTR_BOLD,
                4 => attr |= ATTR_UNDERLINE,
                24 => attr &= !ATTR_UNDERLINE,
                5 => attr |= ATTR_BLINK,
                25 => attr &= !ATTR_BLINK,
                7 => attr |= ATTR_INVERSE,
                27 => attr &= !ATTR_INVERSE,
                30..=37 => attr = (attr & !(COLOR_MASK << FG_SHIFT)) | ((n - 30 + 1) << FG_SHIFT),
                39 => attr &= !(COLOR_MASK << FG_SHIFT),
                40..=47 => attr = (attr & !(COLOR_MASK << BG_SHIFT)) | ((n - 40 + 1) << BG_SHIFT),
                49 => attr &= !(COLOR_MASK << BG_SHIFT),
                _ => {} // 38;5 等 PTT 不用，忽略
            }
        }
        self.attr = attr;
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn paired_in(cells: &[Cell], i: usize) -> bool {
    let c = cells[i];
    match c.half {
        1 => i % COLS < COLS - 1 && cells[i + 1].half == 2 && cells[i + 1].glyph == c.glyph,
        2 => !i.is_multiple_of(COLS) && cells[i - 1].half == 1 && cells[i - 1].glyph == c.glyph,
        _ => false,
    }
}

/// 以 WHATWG big5 表解一對 byte；非法組合得 U+FFFD。
/// 表的覆蓋率足夠：彩繪的「亂碼」真因是 SGR 劈字（本模型原生處理），不是查表查不到
fn decode_big5_pair(lead: u8, trail: u8) -> char {
    let pair = [lead, trail];
    let (s, _) = encoding_rs::BIG5.decode_without_bom_handling(&pair);
    s.chars().next().unwrap_or('\u{fffd}')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf8_grid() -> Grid {
        let mut g = Grid::new();
        g.set_utf8(true);
        g
    }

    fn feed(g: &mut Grid, s: &str) {
        g.feed(s.as_bytes());
    }

    fn fixture_chunks(data: &[u8]) -> Vec<&[u8]> {
        assert_eq!(&data[..8], b"PTTREC1\n");
        let mut chunks = Vec::new();
        let mut i = 8;
        while i < data.len() {
            let len = u32::from_be_bytes(data[i..i + 4].try_into().unwrap()) as usize;
            i += 4;
            chunks.push(&data[i..i + len]);
            i += len;
        }
        chunks
    }

    const MAINMENU: &[u8] = include_bytes!("../tests/fixtures/mainmenu-utf8.bin");
    const WELCOME: &[u8] = include_bytes!("../tests/fixtures/welcome-big5.bin");

    // ---- utf-8 模式：半字覆寫（第五根因核心）----

    #[test]
    fn space_over_left_half_leaves_right_orphan() {
        let mut g = utf8_grid();
        feed(&mut g, "\x1b[1;1H雨");
        assert!(g.paired_at(0));
        feed(&mut g, "\x1b[1;1H ");
        assert_eq!(g.cell(0, 0).glyph, ' ');
        let r = *g.cell(0, 1);
        assert_eq!((r.glyph, r.half), ('雨', 2));
        assert!(!g.paired_at(1));
    }

    #[test]
    fn wide_overwrite_out_of_phase_orphans_neighbor() {
        let mut g = utf8_grid();
        feed(&mut g, "\x1b[1;1H雨");
        feed(&mut g, "\x1b[1;2H天");
        assert_eq!((g.cell(0, 0).glyph, g.cell(0, 0).half), ('雨', 1));
        assert!(!g.paired_at(0));
        assert_eq!((g.cell(0, 1).glyph, g.cell(0, 1).half), ('天', 1));
        assert!(g.paired_at(1));
        assert_eq!(g.row_text(0).chars().take(2).collect::<String>(), "?天");
    }

    #[test]
    fn animation_grammar_space_bs_wide() {
        // 動態看板文法「空格×2 BS×2 寬字」：對齊無損、錯相劈鄰字
        let mut g = utf8_grid();
        feed(&mut g, "\x1b[1;1H雨天");
        feed(&mut g, "\x1b[1;1H  \x08\x08晴");
        assert_eq!(g.row_text(0).chars().take(2).collect::<String>(), "晴天");
        feed(&mut g, "\x1b[1;2H  \x08\x08陰");
        assert_eq!(g.row_text(0).chars().take(3).collect::<String>(), "?陰?");
    }

    #[test]
    fn el_splitting_pair_orphans_left_half() {
        let mut g = utf8_grid();
        feed(&mut g, "\x1b[1;1H雨天");
        feed(&mut g, "\x1b[1;2H\x1b[K");
        assert_eq!((g.cell(0, 0).glyph, g.cell(0, 0).half), ('雨', 1));
        assert_eq!(*g.cell(0, 1), Cell::BLANK);
        assert_eq!(*g.cell(0, 3), Cell::BLANK);
    }

    #[test]
    fn ed_clears_everything_with_bce() {
        let mut g = utf8_grid();
        feed(&mut g, "\x1b[5;5H雨\x1b[44m\x1b[2J");
        assert_eq!(g.cell(4, 4).glyph, ' ');
        assert_eq!(g.cell(0, 0).bg(), 4);
        assert_eq!(g.cell(23, 79).bg(), 4);
    }

    #[test]
    fn pending_wrap_bs_is_pure_decrement() {
        let mut g = utf8_grid();
        feed(&mut g, "\x1b[1;1H");
        feed(&mut g, &"x".repeat(80));
        assert_eq!(g.cursor(), (0, 79));
        feed(&mut g, "\x08\x08");
        assert_eq!(g.cursor(), (0, 78));
        feed(&mut g, "雨");
        assert_eq!(g.row_text(0).chars().skip(78).collect::<String>(), "雨");
    }

    #[test]
    fn wide_char_at_col_80_wraps() {
        let mut g = utf8_grid();
        feed(&mut g, "\x1b[1;80H雨");
        assert_eq!(g.cell(0, 79).glyph, ' ');
        assert_eq!((g.cell(1, 0).glyph, g.cell(1, 0).half), ('雨', 1));
    }

    // ---- SGR 劈字（duotone 語意）----

    #[test]
    fn sgr_inside_utf8_sequence_gives_two_colors() {
        let mut g = utf8_grid();
        let bytes = "雨".as_bytes();
        let mut data = vec![0x1b, b'[', b'3', b'1', b'm', bytes[0]];
        data.extend_from_slice(b"\x1b[34m");
        data.extend_from_slice(&bytes[1..]);
        g.feed(b"\x1b[1;1H");
        g.feed(&data);
        assert!(g.paired_at(0));
        assert_eq!(g.cell(0, 0).fg(), 1);
        assert_eq!(g.cell(0, 1).fg(), 4);
        assert!(g.unknown_sequences.is_empty());
    }

    // ---- 跨 chunk 狀態機 ----

    #[test]
    fn utf8_split_anywhere_matches_single_feed() {
        let text = "\x1b[2;3H雨天\x1b[31m晴";
        let bytes = text.as_bytes();
        let mut whole = utf8_grid();
        whole.feed(bytes);
        for cut in 1..bytes.len() {
            let mut g = utf8_grid();
            g.feed(&bytes[..cut]);
            g.feed(&bytes[cut..]);
            assert_eq!(g.cells(), whole.cells(), "cut at {cut}");
            assert_eq!(g.cursor(), whole.cursor());
        }
    }

    #[test]
    fn csi_params_split_across_chunks() {
        let mut g = utf8_grid();
        g.feed(b"\x1b[3");
        g.feed(b";4Hx");
        assert_eq!(g.cell(2, 3).glyph, 'x');
    }

    // ---- big5 模式 ----

    #[test]
    fn big5_lead_trail_pairs() {
        let mut g = Grid::new();
        let (b, _, _) = encoding_rs::BIG5.encode("雨天");
        g.feed(&b);
        assert_eq!(g.row_text(0).chars().take(2).collect::<String>(), "雨天");
        assert!(g.paired_at(0) && g.paired_at(2));
    }

    #[test]
    fn big5_sgr_between_lead_and_trail_two_colors() {
        let mut g = Grid::new();
        let (b, _, _) = encoding_rs::BIG5.encode("雨");
        let mut data = vec![0x1b, b'[', b'3', b'1', b'm', b[0]];
        data.extend_from_slice(b"\x1b[34m");
        data.push(b[1]);
        g.feed(&data);
        assert!(g.paired_at(0));
        assert_eq!(g.cell(0, 0).glyph, '雨');
        assert_eq!(g.cell(0, 0).fg(), 1);
        assert_eq!(g.cell(0, 1).fg(), 4);
    }

    #[test]
    fn big5_orphan_lead_stays_as_byte() {
        let mut g = Grid::new();
        g.feed(&[0xb6, 0x1b, b'[', b'2', b';', b'1', b'H', b'A']);
        assert_eq!(g.cell(0, 0).byte, 0xb6);
        assert_eq!(g.cell(0, 0).half, 0);
        assert_eq!(g.row_text(0).chars().next(), Some('?'));
        assert_eq!(g.cell(1, 0).glyph, 'A');
    }

    #[test]
    fn big5_pair_split_across_chunks() {
        let mut g = Grid::new();
        let (b, _, _) = encoding_rs::BIG5.encode("雨");
        g.feed(&b[..1]);
        g.feed(&b[1..]);
        assert_eq!(g.cell(0, 0).glyph, '雨');
        assert!(g.paired_at(0));
    }

    #[test]
    fn switch_to_utf8_keeps_screen() {
        let mut g = Grid::new();
        let (b, _, _) = encoding_rs::BIG5.encode("雨");
        g.feed(&b);
        g.set_utf8(true);
        feed(&mut g, "天");
        assert_eq!(g.row_text(0).chars().take(2).collect::<String>(), "雨天");
    }

    #[test]
    fn reset_returns_to_big5_and_clears() {
        let mut g = utf8_grid();
        feed(&mut g, "雨");
        g.reset();
        assert!(!g.is_utf8());
        assert_eq!(g.cell(0, 0).glyph, ' ');
        assert_eq!(g.cursor(), (0, 0));
    }

    #[test]
    fn write_text_encodes_per_mode() {
        let mut g = Grid::new();
        g.write_text("連線中");
        assert!(g.row_text(0).starts_with("連線中"));
        let mut g = utf8_grid();
        g.write_text("連線中");
        assert!(g.row_text(0).starts_with("連線中"));
    }

    // ---- 游標運動 ----

    #[test]
    fn cursor_motion_and_clamping() {
        let mut g = utf8_grid();
        feed(&mut g, "\x1b[5;10H");
        assert_eq!(g.cursor(), (4, 9));
        feed(&mut g, "\x1b[2A\x1b[3C");
        assert_eq!(g.cursor(), (2, 12));
        feed(&mut g, "\x1b[99B\x1b[99D");
        assert_eq!(g.cursor(), (23, 0));
        feed(&mut g, "\x1b[99C");
        assert_eq!(g.cursor(), (23, 79));
        feed(&mut g, "\x1b[H");
        assert_eq!(g.cursor(), (0, 0));
        feed(&mut g, "abc\r");
        assert_eq!(g.cursor(), (0, 0));
        feed(&mut g, "\n");
        assert_eq!(g.cursor(), (1, 0));
        feed(&mut g, "\x1b[0;0H");
        assert_eq!(g.cursor(), (0, 0));
    }

    #[test]
    fn save_restore_cursor_and_attr() {
        let mut g = utf8_grid();
        feed(&mut g, "\x1b[3;4H\x1b[31m\x1b7\x1b[1;1H\x1b[0mA\x1b8B");
        assert_eq!(g.cell(2, 3).glyph, 'B');
        assert_eq!(g.cell(2, 3).fg(), 1);
        assert_eq!(g.cell(0, 0).fg(), -1);
    }

    // ---- 捲動 ----

    #[test]
    fn lf_on_last_row_scrolls_up() {
        let mut g = utf8_grid();
        feed(&mut g, "\x1b[1;1H頂\x1b[24;1H底\x1b[44m\n");
        assert_eq!(g.cell(0, 0).glyph, ' ');
        assert_eq!(g.cell(22, 0).glyph, '底');
        assert!(g.paired_at(22 * COLS));
        assert_eq!(g.cell(23, 0).bg(), 4);
        assert_eq!(g.cursor(), (23, 2)); // LF 不帶 CR，欄位維持在「底」之後
    }

    #[test]
    fn autowrap_on_last_row_scrolls() {
        let mut g = utf8_grid();
        feed(&mut g, "\x1b[1;1HA\x1b[24;1H");
        feed(&mut g, &"x".repeat(80));
        feed(&mut g, "y");
        assert_eq!(g.cell(0, 0).glyph, ' ');
        assert_eq!(g.cell(23, 0).glyph, 'y');
        assert_eq!(g.cell(22, 0).glyph, 'x');
    }

    #[test]
    fn ri_on_top_row_scrolls_down() {
        let mut g = utf8_grid();
        feed(&mut g, "\x1b[1;1H頂\x1b[24;1H底\x1b[H\x1bM");
        assert_eq!(g.cell(0, 0).glyph, ' ');
        assert_eq!(g.cell(1, 0).glyph, '頂');
        assert_eq!(g.cell(23, 0).glyph, ' ');
        assert_eq!(g.cursor(), (0, 0));
    }

    #[test]
    fn ri_not_on_top_only_moves_cursor() {
        let mut g = utf8_grid();
        feed(&mut g, "\x1b[1;1H頂\x1b[3;1H\x1bM");
        assert_eq!(g.cell(0, 0).glyph, '頂');
        assert_eq!(g.cursor(), (1, 0));
    }

    #[test]
    fn big5_pending_lead_shifts_with_scroll() {
        let mut g = Grid::new();
        let (b, _, _) = encoding_rs::BIG5.encode("雨");
        g.feed(b"\x1b[24;1H");
        g.feed(&b[..1]);
        // trail 到達前先捲一列：lead 隨之上移，但游標已離開 → 不再配對（與 xterm 游標語意一致）
        g.feed(b"\n");
        assert_eq!(g.cell(22, 0).byte, b[0] as i16);
    }

    // ---- 未實作序列偵測 ----

    #[test]
    fn known_sequences_not_recorded() {
        let mut g = utf8_grid();
        feed(
            &mut g,
            "\x1b[m\x1b[1;1H\x1b[K\x1b[J\x1bM\x08\r\n\x00\x07\x7f\x1b7\x1b8\x1b[s\x1b[u\t\x1b[?7l\x1b[?25h\x1b[6n",
        );
        assert!(g.unknown_sequences.is_empty(), "{:?}", g.unknown_sequences);
    }

    #[test]
    fn unknown_sequences_recorded_by_kind() {
        let mut g = utf8_grid();
        feed(&mut g, "\x1bZ\x1b[?1049l\x1b[3;4r\x0e\x1b[?3J");
        let got: Vec<&str> = g.unknown_sequences.iter().map(String::as_str).collect();
        assert_eq!(
            got,
            vec!["C0 0x0e", "CSI ? J", "CSI ?1049 l", "CSI r", "ESC Z"]
        );
    }

    #[test]
    fn unknown_sequences_capped() {
        let mut g = utf8_grid();
        for i in 0..40u8 {
            g.feed(&[0x1b, b'[', 0x40 + i]);
        }
        assert!(g.unknown_sequences.len() <= MAX_UNKNOWN);
    }

    // ---- 私有模式與回應（2026-11-08 實機連線錄到 ESC[?7l 與 ESC[6n）----

    #[test]
    fn autowrap_off_pins_cursor_at_last_column() {
        let mut g = utf8_grid();
        feed(&mut g, "\x1b[?7l\x1b[1;79Hab");
        assert_eq!(g.cursor(), (0, 79));
        feed(&mut g, "c"); // 覆寫末欄，不換行
        assert_eq!(g.cell(0, 79).glyph, 'c');
        assert_eq!(g.cell(1, 0).glyph, ' ');
        feed(&mut g, "雨"); // 寬字停在 78–79 欄
        assert_eq!((g.cell(0, 78).glyph, g.cell(0, 78).half), ('雨', 1));
        assert_eq!(g.cursor(), (0, 79));
        feed(&mut g, "\x1b[?7h\x1b[1;80Hxy");
        assert_eq!(g.cell(1, 0).glyph, 'y');
        assert!(g.unknown_sequences.is_empty(), "{:?}", g.unknown_sequences);
    }

    #[test]
    fn dsr_reports_cursor_position() {
        let mut g = utf8_grid();
        feed(&mut g, "\x1b[5;10H\x1b[6n\x1b[5n");
        assert_eq!(g.take_responses(), b"\x1b[5;10R\x1b[0n");
        assert!(g.take_responses().is_empty());
    }

    #[test]
    fn cursor_visibility_mode() {
        let mut g = utf8_grid();
        assert!(g.cursor_visible());
        feed(&mut g, "\x1b[?25l");
        assert!(!g.cursor_visible());
        feed(&mut g, "\x1b[?25h");
        assert!(g.cursor_visible());
        g.feed(b"\x1b[?25l");
        g.reset();
        assert!(g.cursor_visible());
    }

    // ---- fixture 重放 ----

    #[test]
    fn mainmenu_fixture_replays_cleanly() {
        let mut g = utf8_grid();
        for c in fixture_chunks(MAINMENU) {
            g.feed(c);
        }
        assert!(g.row_text(0).contains("主功能表"));
        assert!(g.row_text(0).contains("批踢踢實業坊"));
        assert!(g.row_text(23).contains("我是xxxxxxxxx")); // fixture 已遮蔽
        assert_eq!(g.cursor(), (14, 20));
        assert!(g.unknown_sequences.is_empty(), "{:?}", g.unknown_sequences);
    }

    #[test]
    fn welcome_big5_fixture_decodes() {
        let mut g = Grid::new();
        g.feed(WELCOME);
        let screen: String = (0..ROWS).map(|r| g.row_text(r)).collect();
        // 登入畫面必有的提示字樣（雙色字彩繪 111 處全在網格語意內配對，無 U+FFFD）
        assert!(screen.contains("請輸入代號"), "{screen}");
        assert!(!screen.contains('\u{fffd}'));
        assert!(g.unknown_sequences.is_empty(), "{:?}", g.unknown_sequences);
    }

    #[test]
    fn replay_perf_sanity() {
        let chunks = fixture_chunks(MAINMENU);
        let t0 = std::time::Instant::now();
        for _ in 0..10 {
            let mut g = utf8_grid();
            for c in &chunks {
                g.feed(c);
            }
        }
        assert!(t0.elapsed().as_millis() < 2000);
    }

    #[test]
    fn text_in_range_trims_and_keeps_wide_chars_whole() {
        let mut g = Grid::new();
        g.set_utf8(true);
        g.feed("ab 雨天 xyz\r\nsecond line   \r\nthird".as_bytes());
        // 從 'b' 到第二列的 'c'：第一列去尾空白，寬字右半起點補整字不重複
        assert_eq!(g.text_in_range((0, 1), (1, 2)), "b 雨天 xyz\nsec");
        // 從「雨」右半開始
        assert_eq!(g.text_in_range((0, 4), (0, 6)), "雨天");
        // 反向給定起訖也可
        assert_eq!(g.text_in_range((2, 4), (2, 0)), "third");
        // 單格
        assert_eq!(g.text_in_range((0, 0), (0, 0)), "a");
    }

    #[test]
    fn text_in_range_skips_unpaired_big5_bytes() {
        let mut g = Grid::new();
        let (b, _, _) = encoding_rs::BIG5.encode("雨");
        g.feed(&b[..1]); // 只有 lead byte；接著的空白不是合法 trail，lead 留成孤兒 byte
        g.feed(b" Z");
        assert_eq!(g.text_in_range((0, 0), (0, 3)), " Z");
    }

    #[test]
    fn mouse_reporting_and_sync_update_modes_are_known_noops() {
        let mut g = Grid::new();
        g.feed(b"\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?2026h\x1b[?2026l");
        assert!(g.unknown_sequences.is_empty(), "{:?}", g.unknown_sequences);
        // 真正不認識的私有模式仍要記錄
        g.feed(b"\x1b[?9999h");
        assert!(g.unknown_sequences.contains("CSI ?9999 h"));
    }
}
