//! 手動重連的節流：登入冷卻＋線上時的二次確認。
//!
//! PTT 對短時間內反覆登入會顯示「請勿頻繁登入」；連按 Ctrl+R 就是連續登入。這裡只決定
//! 「這次按鍵放不放行」，連線本身由 view 處理。被擋下時**不排程、不自動連**，時間到了
//! 仍要使用者再按一次——與自動登入「絕不自動重試」同一原則。
//!
//! 冷卻從每次開始連線（含啟動時那次）起算，不論那次有沒有登入成功：PTT 的門檻不公開
//! （公開的 pttbbs 原始碼找不到這句提示），寧可保守。計時不持有 timer，呼叫端傳入 `now`。

use std::time::{Duration, Instant};

/// 兩次連線的最短間隔
pub const LOGIN_COOLDOWN: Duration = Duration::from_secs(30);
/// 線上時第一次按重連後，要在這段時間內再按一次才算確認
pub const CONFIRM_WINDOW: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// 距上次連線太近，擋下；附剩餘時間
    Cooldown(Duration),
    /// 線上時的第一次按鍵：等使用者在 `CONFIRM_WINDOW` 內再按一次
    NeedConfirm,
    /// 放行
    Go,
}

#[derive(Default)]
pub struct ReconnectGuard {
    last_connect: Option<Instant>,
    armed_at: Option<Instant>,
}

impl ReconnectGuard {
    /// 使用者要求重連。`online`：目前連線還在（含握手中）——斷線後按 Enter 不需確認
    pub fn request(&mut self, now: Instant, online: bool) -> Decision {
        if let Some(last) = self.last_connect {
            let elapsed = now.saturating_duration_since(last);
            if elapsed < LOGIN_COOLDOWN {
                self.armed_at = None;
                return Decision::Cooldown(LOGIN_COOLDOWN - elapsed);
            }
        }
        if !online {
            self.armed_at = None;
            return Decision::Go;
        }
        match self.armed_at.take() {
            Some(at) if now.saturating_duration_since(at) <= CONFIRM_WINDOW => Decision::Go,
            _ => {
                self.armed_at = Some(now);
                Decision::NeedConfirm
            }
        }
    }

    /// 實際開始連線時呼叫（啟動那次也要記），冷卻由此起算
    pub fn mark_connect(&mut self, now: Instant) {
        self.last_connect = Some(now);
        self.armed_at = None;
    }
}

/// 剩餘時間以整秒顯示，無條件進位（剩 0.2 秒也顯示 1 秒，不會出現「0 秒後再試」）
pub fn ceil_secs(d: Duration) -> u64 {
    d.as_secs() + u64::from(d.subsec_nanos() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn first_connect_has_no_cooldown() {
        let mut g = ReconnectGuard::default();
        assert_eq!(g.request(Instant::now(), false), Decision::Go);
    }

    #[test]
    fn offline_within_cooldown_is_blocked_with_remaining_time() {
        let t0 = Instant::now();
        let mut g = ReconnectGuard::default();
        g.mark_connect(t0);
        assert_eq!(
            g.request(t0 + secs(10), false),
            Decision::Cooldown(LOGIN_COOLDOWN - secs(10))
        );
    }

    #[test]
    fn offline_after_cooldown_goes_without_confirm() {
        let t0 = Instant::now();
        let mut g = ReconnectGuard::default();
        g.mark_connect(t0);
        assert_eq!(g.request(t0 + LOGIN_COOLDOWN, false), Decision::Go);
    }

    #[test]
    fn online_needs_second_press_within_window() {
        let t0 = Instant::now();
        let mut g = ReconnectGuard::default();
        g.mark_connect(t0);
        let t1 = t0 + LOGIN_COOLDOWN + secs(1);
        assert_eq!(g.request(t1, true), Decision::NeedConfirm);
        assert_eq!(g.request(t1 + CONFIRM_WINDOW, true), Decision::Go);
    }

    #[test]
    fn online_second_press_too_late_rearms_instead_of_going() {
        let t0 = Instant::now();
        let mut g = ReconnectGuard::default();
        g.mark_connect(t0);
        let t1 = t0 + LOGIN_COOLDOWN + secs(1);
        assert_eq!(g.request(t1, true), Decision::NeedConfirm);
        let late = t1 + CONFIRM_WINDOW + Duration::from_millis(1);
        assert_eq!(g.request(late, true), Decision::NeedConfirm);
        // 逾期那次重新起算確認窗
        assert_eq!(g.request(late + secs(1), true), Decision::Go);
    }

    #[test]
    fn confirm_is_consumed_so_third_press_needs_confirm_again() {
        let t0 = Instant::now();
        let mut g = ReconnectGuard::default();
        let t1 = t0 + secs(1);
        assert_eq!(g.request(t1, true), Decision::NeedConfirm);
        assert_eq!(g.request(t1 + secs(1), true), Decision::Go);
        // 呼叫端沒有真的連（例如舊連線還在關），下一次按鍵不得沿用已用掉的確認
        assert_eq!(g.request(t1 + secs(2), true), Decision::NeedConfirm);
    }

    #[test]
    fn cooldown_wins_over_confirm_and_clears_armed_state() {
        let t0 = Instant::now();
        let mut g = ReconnectGuard::default();
        assert_eq!(g.request(t0, true), Decision::NeedConfirm);
        g.mark_connect(t0 + secs(1)); // 例如斷線後按 Enter 連上了
        assert!(matches!(
            g.request(t0 + secs(2), true),
            Decision::Cooldown(_)
        ));
        // 冷卻過後，之前的確認不得殘留
        let after = t0 + secs(1) + LOGIN_COOLDOWN;
        assert_eq!(g.request(after, true), Decision::NeedConfirm);
    }

    #[test]
    fn mark_connect_clears_pending_confirm() {
        let t0 = Instant::now();
        let mut g = ReconnectGuard::default();
        assert_eq!(g.request(t0, true), Decision::NeedConfirm);
        g.mark_connect(t0 + secs(1));
        let after = t0 + secs(1) + LOGIN_COOLDOWN;
        assert_eq!(g.request(after, true), Decision::NeedConfirm);
    }

    #[test]
    fn ceil_secs_rounds_up_partial_seconds() {
        assert_eq!(ceil_secs(Duration::from_millis(200)), 1);
        assert_eq!(ceil_secs(secs(20)), 20);
        assert_eq!(ceil_secs(Duration::from_millis(20_001)), 21);
    }
}
