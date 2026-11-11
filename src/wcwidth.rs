//! PTT 語意的字元寬度：Big5 映射像 ∪ EAW W/F/A 一律 2 格（資料表 `wcwidth_data.rs`）。
//! 彩繪用的方塊、框線、◤◥ 等屬 East Asian Ambiguous，標準 wcwidth 算 1 格，
//! 但 PTT 版面以 Big5 語意（2 格）編排，算 1 格會壓縮跑版。

use crate::wcwidth_data::{WIDE_RANGES, ZERO_RANGES};

/// table 為 [lo, hi] 交錯的扁平陣列，已排序，二分搜尋
fn in_ranges(cp: u32, table: &[u32]) -> bool {
    if table.is_empty() || cp < table[0] || cp > table[table.len() - 1] {
        return false;
    }
    let mut lo: isize = 0;
    let mut hi: isize = (table.len() / 2) as isize - 1;
    while lo <= hi {
        let mid = ((lo + hi) >> 1) as usize;
        if cp > table[mid * 2 + 1] {
            lo = mid as isize + 1;
        } else if cp < table[mid * 2] {
            hi = mid as isize - 1;
        } else {
            return true;
        }
    }
    false
}

pub fn wcwidth(cp: u32) -> u8 {
    if cp < 32 {
        return 0;
    }
    if cp < 127 {
        return 1;
    }
    if in_ranges(cp, ZERO_RANGES) {
        return 0;
    }
    if in_ranges(cp, WIDE_RANGES) {
        return 2;
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_narrow_cjk_wide() {
        assert_eq!(wcwidth('A' as u32), 1);
        assert_eq!(wcwidth('雨' as u32), 2);
        assert_eq!(wcwidth('　' as u32), 2);
    }

    #[test]
    fn ambiguous_art_chars_are_wide() {
        for ch in ['█', '▇', '◢', '◣', '◤', '◥', '─', '│', '※', '◎', '★'] {
            assert_eq!(wcwidth(ch as u32), 2, "{ch}");
        }
    }

    #[test]
    fn combining_is_zero() {
        assert_eq!(wcwidth(0x0301), 0);
        assert_eq!(wcwidth(0x0009), 0);
    }

    #[test]
    fn latin1_symbols_follow_big5_image() {
        // Big5 映射像內的 Latin-1 符號（如 ×）以 2 格呈現
        assert_eq!(wcwidth('×' as u32), 2);
        assert_eq!(wcwidth('é' as u32), 2); // EAW Ambiguous，PTT 語意算寬
        assert_eq!(wcwidth('¢' as u32), 1); // 不在 Big5 映射像、EAW Narrow
    }
}
