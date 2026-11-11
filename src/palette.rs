//! PTT 傳統 VGA 色盤（與 term.ptt.cc／PttChrome 相同），全 app 單一出處。
//! BBS 慣例：明確黑底（SGR 40）＝預設背景＝純黑、預設前景＝color 7。

use crate::grid::Cell;

pub const DEFAULT_FG: usize = 7;
pub const DEFAULT_BG: usize = 0;

#[rustfmt::skip]
pub const ANSI_16: [u32; 16] = [
    0x000000, 0x800000, 0x008000, 0x808000, 0x000080, 0x800080, 0x008080, 0xc0c0c0,
    0x808080, 0xff0000, 0x00ff00, 0xffff00, 0x0000ff, 0xff00ff, 0x00ffff, 0xffffff,
];

/// 一格的有效前景／背景色盤索引（0–15）。
/// BBS 的 SGR 1 語意是「選亮色」：前景 0–7 提升為 8–15（預設前景視為 7，
/// 故粗體預設字為亮白，與 PttChrome 一致）；反白把兩者互換（PttChrome 語意：
/// 亮化作用於文字色之後再互換，反白＋粗體會得到亮色背景）。
pub fn cell_colors(cell: &Cell) -> (usize, usize) {
    let mut fg = match cell.fg() {
        f if f >= 0 => f as usize,
        _ => DEFAULT_FG,
    };
    let bg = match cell.bg() {
        b if b >= 0 => b as usize,
        _ => DEFAULT_BG,
    };
    if cell.bold() && fg < 8 {
        fg += 8;
    }
    if cell.inverse() {
        (bg, fg)
    } else {
        (fg, bg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::{ATTR_BOLD, ATTR_INVERSE};

    fn cell(attr: u16) -> Cell {
        Cell {
            glyph: 'A',
            half: 0,
            byte: -1,
            attr,
        }
    }

    #[test]
    fn defaults_are_7_on_0() {
        assert_eq!(cell_colors(&cell(0)), (7, 0));
    }

    #[test]
    fn bold_brightens_fg_only() {
        // fg 紅(1) + bold → 亮紅(9)；bg 不變
        let attr = ATTR_BOLD | ((1 + 1) << 3) | ((4 + 1) << 7);
        assert_eq!(cell_colors(&cell(attr)), (9, 4));
        // 已是亮色不再加
        let attr = ATTR_BOLD | ((9 + 1) << 3);
        assert_eq!(cell_colors(&cell(attr)), (9, 0));
    }

    #[test]
    fn inverse_swaps_after_brightening() {
        let attr = ATTR_INVERSE | ATTR_BOLD;
        assert_eq!(cell_colors(&cell(attr)), (0, 15));
    }
}
