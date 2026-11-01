// Every time constant the page uses (spec §3.5, §8.6 and the flows of §7). Nothing else
// in the page writes a duration. CSS durations in tokens.css mirror the MOTION block; a
// test keeps them equal.

const SECOND = 1000;
const MINUTE = 60 * SECOND;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** §3.5: CSS transitions, in ms. Mirrored as `--d-*` in tokens.css. */
export const MOTION = {
  hover: 80,
  popIn: 140,
  popOut: 100,
  sheet: 240,
  dialog: 180,
  row: 160,
  tint: 4000,
  banner: 200,
  map: 400,
  spin: 800,
} as const;

/** §3.5 easings, mirrored as `--e-*` in tokens.css. */
export const EASING = {
  linear: 'linear',
  pop: 'cubic-bezier(0.2, 0.8, 0.2, 1)',
  sheet: 'cubic-bezier(0.32, 0.72, 0, 1)',
} as const;

/** §8.6, exact. */
export const TICK_MS = 2000;
export const STALE_MS = 5000;
export const BANNER_DELAY_MS = 3000;
export const TINT_MS = 4000;
export const ROW_ANIMATION_MS = 160;
export const REMOVED_HOLD_MS = 1500;
export const SPINNER_DELAY_MS = 150;
export const SPINNER_MIN_MS = 300;
export const SEEN_TIMEOUT_MS = 10_000;
export const UNDO_MS = 8000;
export const UNDO_ALLOW_RULE_MS = 10_000;
export const PATH_HYSTERESIS_MS = 5000;
export const SORT_FREEZE_MS = 1000;
export const NOTIFY_COALESCE_COUNT = 3;
export const NOTIFY_COALESCE_WINDOW_MS = 30 * SECOND;
export const NOTIFY_REPLAY_CUTOFF_MS = 30 * SECOND;
export const ANNOUNCER_GAP_MS = 3000;
export const JOURNAL_CAP = 2000;
export const QR_AUTO_HIDE_MS = 300_000;

/** §8.5 connection states. */
export const CATCHING_UP_QUIET_MS = 1000;
export const RECONNECT_BACKOFF_MS = [500, 1000, 2000, 4000] as const;
export const RECONNECT_JITTER = 0.25;
export const DAEMON_POLL_MS = 2000;

/** §5 components. */
export const TOOLTIP_DELAY_MS = 400;
export const HOVERCARD_DELAY_MS = 400;
export const COPIED_MS = 1500;
export const TOAST_MS = 5000;
export const TOAST_MAX = 3;
export const RELATIVE_TIME_REFRESH_MS = SECOND;
export const RELATIVE_NOW_MS = 5 * SECOND;
export const RELATIVE_TIME_CLOCK_AFTER_MS = HOUR;
export const RELATIVE_TIME_WEEKDAY_AFTER_MS = DAY;
export const FLAP_COUNT = 3;
export const FLAP_WINDOW_MS = 60 * SECOND;
export const RTT_TINT_TICKS = 3;
export const RTT_TINT_FACTOR = 2;

/** §5 Spark and §9.1 traffic. */
export const SPARK_RESCALE_MS = 300;
export const SPARK_RESCALE_UP = 1.25;
export const SPARK_RESCALE_DOWN = 0.5;
export const TRAFFIC_GAP_MS = 5000;
export const Y_SHRINK_AFTER_MS = 10_000;
export const Y_SHRINK_HEADROOM = 3;
export const READOUT_LINGER_MS = 2000;
export const TRAFFIC_WINDOW_MS = HOUR;
export const TRAFFIC_SAMPLES = 1800;
export const RANGE_MS = { '1m': MINUTE, '15m': 15 * MINUTE, '1h': HOUR } as const;
export const IDLE_BYTES_PER_SEC = 1000;

/** §6.5 Activity. */
export const BLOCKED_GROUP_MS = 10 * MINUTE;
export const BLOCKED_ANNOUNCE_MS = 5 * MINUTE;

/** §7 flows. */
export const SPACES_HINT_MS = SECOND;
export const JOIN_RETRY_HINT_MS = 60 * SECOND;
export const SHEET_CLOSE_AFTER_ONLINE_MS = 2000;
export const PHONE_NOT_CONNECTED_MS = 90 * SECOND;
export const PHONE_NO_INTERNET_MS = 20 * SECOND;
export const PHONE_ABANDONED_MS = DAY;
export const IGNORE_BLOCKED_MS = DAY;
export const NOTIFY_PROMPT_SNOOZE_MS = 7 * DAY;
export const DAEMON_STOPPED_NOTIFY_MS = 10 * SECOND;
export const INVITE_LINK_TTL_MS = 2 * MINUTE;

/** §12.4 verdicts. */
export const QUIET_HANDSHAKE_MS = 3 * MINUTE;
export const RECENTLY_SEEN_MS = 10 * MINUTE;
export const LONG_GONE_MS = DAY;

/** §7.3, §7.4 option values. */
export const INVITE_VALIDITY_SECS = { hour: 3600, day: 86_400, week: 604_800 } as const;
