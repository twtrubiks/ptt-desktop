//! 網址掃描：在網格列上找出 http(s) 網址的格位範圍（常駐底線＋點擊開瀏覽器）。
//! 規則比照 `@xterm/addon-web-links` 的預設 regex（原專案逐字複製以保證
//! 「畫了底線」＝「可點擊」；這裡沒有 regex 依賴，以等價的手寫掃描實作）：
//!   (https?|HTTPS?):[/]{2}[^\s"'!*(){}|\\^<>`]*[^\s"':,.!?{}|\\^~\[\]`()<>]
//! 已知限制：PTT 是 server 端 80 欄硬換行，跨行網址只涵蓋單行內的部分。

use crate::grid::{Cell, COLS};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSpan {
    pub col: usize,
    /// 佔的格數
    pub width: usize,
    pub url: String,
}

fn is_body_char(c: char) -> bool {
    !(c.is_whitespace() || "\"'!*(){}|\\^<>`".contains(c))
}

fn is_tail_char(c: char) -> bool {
    !(c.is_whitespace() || "\"':,.!?{}|\\^~[]`()<>".contains(c))
}

/// 一列的可見字元序列：(字元, 起始格, 格寬)。配對寬字一個字佔兩格，孤兒半字與
/// 未配對 byte 以佔位字元補齊，確保格位對應
fn row_chars(cells: &[Cell]) -> Vec<(char, usize, usize)> {
    let mut out = Vec::with_capacity(COLS);
    let mut col = 0;
    while col < cells.len() {
        let c = cells[col];
        let paired_left = c.half == 1
            && col + 1 < cells.len()
            && cells[col + 1].half == 2
            && cells[col + 1].glyph == c.glyph;
        if paired_left {
            out.push((c.glyph, col, 2));
            col += 2;
        } else if c.half != 0 || c.byte >= 0 {
            out.push(('?', col, 1));
            col += 1;
        } else {
            out.push((c.glyph, col, 1));
            col += 1;
        }
    }
    out
}

pub fn scan_row(cells: &[Cell]) -> Vec<LinkSpan> {
    let chars = row_chars(cells);
    let lower: Vec<char> = chars
        .iter()
        .map(|(c, _, _)| c.to_ascii_lowercase())
        .collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let scheme_len = if lower[i..].starts_with(&['h', 't', 't', 'p', 's', ':', '/', '/']) {
            8
        } else if lower[i..].starts_with(&['h', 't', 't', 'p', ':', '/', '/']) {
            7
        } else {
            i += 1;
            continue;
        };
        let start = i;
        let mut end = start + scheme_len;
        while end < chars.len() && is_body_char(chars[end].0) {
            end += 1;
        }
        // 尾端標點不算網址的一部分
        while end > start + scheme_len && !is_tail_char(chars[end - 1].0) {
            end -= 1;
        }
        // 必須有 host（"//" 後至少一個非 '/' 字元）
        let has_host = end > start + scheme_len && chars[start + scheme_len].0 != '/';
        if has_host {
            let url: String = chars[start..end].iter().map(|(c, _, _)| *c).collect();
            let col = chars[start].1;
            let (last_col, last_w) = (chars[end - 1].1, chars[end - 1].2);
            found.push(LinkSpan {
                col,
                width: last_col + last_w - col,
                url,
            });
            i = end;
        } else {
            i = start + scheme_len;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Grid;

    fn row(text: &str) -> Vec<LinkSpan> {
        let mut g = Grid::new();
        g.set_utf8(true);
        g.feed(text.as_bytes());
        scan_row(g.row_cells(0))
    }

    #[test]
    fn url_at_line_start() {
        let spans = row("https://example.com/a?b=1 後面");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].col, 0);
        assert_eq!(spans[0].width, "https://example.com/a?b=1".len());
        assert_eq!(spans[0].url, "https://example.com/a?b=1");
    }

    #[test]
    fn wide_prefix_shifts_start_column() {
        let spans = row("網址 http://x.y");
        assert_eq!(spans[0].col, 5); // 兩個全形字＝4 格 + 空格
        assert_eq!(spans[0].width, "http://x.y".len());
    }

    #[test]
    fn ansi_does_not_matter() {
        let spans = row("\x1b[1;33mhttps://a.b/c\x1b[m");
        assert_eq!(spans[0].url, "https://a.b/c");
    }

    #[test]
    fn trailing_punctuation_excluded() {
        let spans = row("看這 https://a.b/c). 真的");
        assert_eq!(spans[0].url, "https://a.b/c");
    }

    #[test]
    fn positioned_url_keeps_gap_columns() {
        let spans = row("\x1b[1;20Hhttps://a.b");
        assert_eq!(spans[0].col, 19);
    }

    #[test]
    fn two_urls_same_line() {
        let spans = row("http://a.b http://c.d");
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[1].col, 11);
    }

    #[test]
    fn none_when_no_url_or_no_host() {
        assert!(row("沒有網址 http:// 也不算").is_empty());
        assert!(row("plain text").is_empty());
    }

    #[test]
    fn uppercase_scheme_allowed() {
        assert_eq!(row("HTTPS://A.B")[0].url, "HTTPS://A.B");
    }
}
