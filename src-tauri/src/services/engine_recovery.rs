//! ComfyUI の自動復旧の判定(docs/47)。
//!
//! 接続の切断が続いた時間を数え、しきい値を超えたら再起動を指示する。時刻を引数に取る
//! 純粋な状態機械にして、時間の経過なしでテストできるようにしている。

use std::time::{Duration, Instant};

/// この時間続けて切れたら再起動する。
pub const DISCONNECT_LIMIT: Duration = Duration::from_secs(90);
/// 再起動の試行の上限(接続が回復するまでの通算)。
pub const MAX_ATTEMPTS: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    /// 再起動する(`attempt` は 1 始まり)。終わったら `restart_finished` を呼ぶ。
    Restart {
        attempt: u32,
    },
}

#[derive(Debug, Default)]
pub struct RecoveryTracker {
    disconnected_since: Option<Instant>,
    attempts: u32,
    gave_up: bool,
}

impl RecoveryTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// 監視の 1 回ごとに呼ぶ。`eligible` は、設定が ON で、アプリが管理する ComfyUI が稼働中または異常終了であること。
    pub fn observe(&mut self, now: Instant, connected: bool, eligible: bool) -> Action {
        if connected {
            *self = Self::default();
            return Action::None;
        }
        if !eligible {
            self.disconnected_since = None;
            return Action::None;
        }
        let since = *self.disconnected_since.get_or_insert(now);
        if self.gave_up || now.duration_since(since) < DISCONNECT_LIMIT {
            return Action::None;
        }
        Action::Restart {
            attempt: self.attempts + 1,
        }
    }

    /// 再起動の結果を記録する。あきらめることになったら `true`。
    /// `fatal` は、再試行しても直らない失敗(パス不正・ポート使用中)。
    pub fn restart_finished(&mut self, now: Instant, ok: bool, fatal: bool) -> bool {
        self.attempts += 1;
        // 次の試行は、改めて切断が続いてから
        self.disconnected_since = Some(now);
        if !ok && (fatal || self.attempts >= MAX_ATTEMPTS) {
            self.gave_up = true;
        }
        self.gave_up
    }

    /// 次のジョブの開始を保留すべきか(復旧の対象で、切断中)。
    pub fn hold(&self) -> bool {
        self.disconnected_since.is_some() && !self.gave_up
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(base: Instant, secs: u64) -> Instant {
        base + Duration::from_secs(secs)
    }

    #[test]
    fn restarts_only_after_the_limit() {
        let t0 = Instant::now();
        let mut t = RecoveryTracker::new();
        assert_eq!(t.observe(t0, false, true), Action::None);
        assert!(t.hold());
        assert_eq!(t.observe(at(t0, 89), false, true), Action::None);
        assert_eq!(
            t.observe(at(t0, 90), false, true),
            Action::Restart { attempt: 1 }
        );
    }

    #[test]
    fn reconnecting_resets_everything() {
        let t0 = Instant::now();
        let mut t = RecoveryTracker::new();
        t.observe(t0, false, true);
        assert_eq!(t.observe(at(t0, 50), true, true), Action::None);
        assert!(!t.hold());
        // 切れ直したら、また最初から数える
        assert_eq!(t.observe(at(t0, 200), false, true), Action::None);
        assert_eq!(t.observe(at(t0, 250), false, true), Action::None);
        assert_eq!(
            t.observe(at(t0, 290), false, true),
            Action::Restart { attempt: 1 }
        );
    }

    #[test]
    fn not_eligible_never_restarts_or_holds() {
        let t0 = Instant::now();
        let mut t = RecoveryTracker::new();
        assert_eq!(t.observe(t0, false, false), Action::None);
        assert!(!t.hold());
        assert_eq!(t.observe(at(t0, 1000), false, false), Action::None);
        // 対象に戻っても、そこから数え直す
        assert_eq!(t.observe(at(t0, 1001), false, true), Action::None);
    }

    #[test]
    fn gives_up_after_three_failed_attempts_until_reconnected() {
        let t0 = Instant::now();
        let mut t = RecoveryTracker::new();
        let mut now = t0;
        t.observe(now, false, true);
        for attempt in 1..=3 {
            now = at(now, 90);
            assert_eq!(t.observe(now, false, true), Action::Restart { attempt });
            let gave_up = t.restart_finished(now, false, false);
            assert_eq!(gave_up, attempt == 3);
        }
        assert!(!t.hold());
        assert_eq!(t.observe(at(now, 10_000), false, true), Action::None);
        // 一度つながれば再開する
        t.observe(at(now, 10_001), true, true);
        t.observe(at(now, 10_002), false, true);
        assert_eq!(
            t.observe(at(now, 10_092), false, true),
            Action::Restart { attempt: 1 }
        );
    }

    #[test]
    fn a_fatal_failure_gives_up_at_once() {
        let t0 = Instant::now();
        let mut t = RecoveryTracker::new();
        t.observe(t0, false, true);
        assert_eq!(
            t.observe(at(t0, 90), false, true),
            Action::Restart { attempt: 1 }
        );
        assert!(t.restart_finished(at(t0, 90), false, true));
        assert_eq!(t.observe(at(t0, 150), false, true), Action::None);
    }

    #[test]
    fn a_successful_restart_waits_again_before_the_next_one() {
        let t0 = Instant::now();
        let mut t = RecoveryTracker::new();
        t.observe(t0, false, true);
        t.observe(at(t0, 90), false, true);
        assert!(!t.restart_finished(at(t0, 100), true, false));
        assert_eq!(t.observe(at(t0, 150), false, true), Action::None);
        assert_eq!(
            t.observe(at(t0, 190), false, true),
            Action::Restart { attempt: 2 }
        );
    }
}
