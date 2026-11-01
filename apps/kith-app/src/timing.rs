//! Every duration the shell uses. Those the page also uses mirror its `timing.ts`; a test
//! keeps them equal.

use core::time::Duration;

/// How often the shell asks again while the daemon is not answering (`DAEMON_POLL_MS`).
pub(crate) const DAEMON_POLL: Duration = Duration::from_millis(2000);
/// Waits before reopening the event stream, then 4 s from there on
/// (`RECONNECT_BACKOFF_MS`).
pub(crate) const RECONNECT_BACKOFF_MS: [u64; 4] = [500, 1000, 2000, 4000];
/// More notifications than this inside the window become one (`NOTIFY_COALESCE_COUNT`).
pub(crate) const NOTIFY_COALESCE_COUNT: usize = 3;
/// The coalescing window, in ms (`NOTIFY_COALESCE_WINDOW_MS`).
pub(crate) const NOTIFY_COALESCE_WINDOW_MS: u64 = 30_000;
/// Nothing older than this notifies, in ms (`NOTIFY_REPLAY_CUTOFF_MS`).
pub(crate) const NOTIFY_REPLAY_CUTOFF_MS: u64 = 30_000;
/// The daemon must stay stopped this long before it notifies, in ms
/// (`DAEMON_STOPPED_NOTIFY_MS`).
pub(crate) const DAEMON_STOPPED_NOTIFY_MS: u64 = 10_000;
/// A device that moved traffic this recently counts as recently used, in ms
/// (`RECENTLY_SEEN_MS`).
pub(crate) const RECENTLY_USED_MS: u64 = 600_000;
/// Journal entries this close together cause one refresh of the tray, not one each.
pub(crate) const REFRESH_COALESCE: Duration = Duration::from_millis(100);
/// The window shows when its page has loaded, or after this long, whichever is first.
pub(crate) const WINDOW_SHOW_FALLBACK: Duration = Duration::from_millis(1000);

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE_TIMING: &str =
        include_str!("../../../crates/management/kith-ui/web/src/lib/live/timing.ts");

    /// The value of `export const <name> = <expr>;`, where `<expr>` is a number or a product
    /// of numbers and `SECOND`, `MINUTE`, `HOUR`.
    fn page_ms(name: &str) -> Option<u64> {
        let line = PAGE_TIMING
            .lines()
            .find(|line| line.starts_with(&format!("export const {name} = ")))?;
        let expr = line.split_once(" = ")?.1.trim_end_matches(';');
        expr.split('*').try_fold(1_u64, |product, factor| {
            let value = match factor.trim() {
                "SECOND" => 1000,
                "MINUTE" => 60_000,
                "HOUR" => 3_600_000,
                number => number.replace('_', "").parse().ok()?,
            };
            product.checked_mul(value)
        })
    }

    #[test]
    fn shared_durations_match_the_page() {
        assert_eq!(
            page_ms("DAEMON_POLL_MS"),
            u64::try_from(DAEMON_POLL.as_millis()).ok()
        );
        assert_eq!(page_ms("NOTIFY_COALESCE_COUNT"), Some(3));
        assert_eq!(NOTIFY_COALESCE_COUNT, 3);
        assert_eq!(
            page_ms("NOTIFY_COALESCE_WINDOW_MS"),
            Some(NOTIFY_COALESCE_WINDOW_MS)
        );
        assert_eq!(
            page_ms("NOTIFY_REPLAY_CUTOFF_MS"),
            Some(NOTIFY_REPLAY_CUTOFF_MS)
        );
        assert_eq!(
            page_ms("DAEMON_STOPPED_NOTIFY_MS"),
            Some(DAEMON_STOPPED_NOTIFY_MS)
        );
        assert_eq!(page_ms("RECENTLY_SEEN_MS"), Some(RECENTLY_USED_MS));
        assert!(PAGE_TIMING.contains("RECONNECT_BACKOFF_MS = [500, 1000, 2000, 4000]"));
    }
}
