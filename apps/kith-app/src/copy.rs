//! Every string the shell shows: the tray menu, notifications, the autostart offer and the
//! window title. Strings the page also shows are the same words as its `copy.ts`; a test
//! keeps them equal. Vocabulary and rules are the design spec's §12.

/// The separator between facts on one line.
pub(crate) const DOT: &str = " · ";
/// The window title and the tray tooltip.
pub(crate) const APP: &str = "kith";

/// The tray icon's first line when kith works: `● Connected to home · 11 of 12 devices`.
pub(crate) fn connected(network: &str, online: u64, total: u64) -> String {
    let devices = if total == 1 { "device" } else { "devices" };
    format!("● Connected to {network}{DOT}{online} of {total} {devices}")
}

/// The first line when the daemon is not running. Same words as the page's gate.
pub(crate) const DAEMON_DOWN: &str = "○ kith isn't running on this device";
/// The first line when this OS user may not talk to the daemon.
pub(crate) const NOT_IN_GROUP: &str = "○ kith is running, but this user can't talk to it";

/// The first line when the shell and the daemon speak different protocols; the window's
/// gate says which versions.
pub(crate) const VERSION_SKEW: &str = "○ Update kith on this device";

/// The first line when this device is in no network yet.
pub(crate) const NO_NETWORK: &str = "○ Not in a network yet";

/// The exit choice that sends internet traffic through this device's own connection.
pub(crate) const NO_EXIT: &str = "No exit · this device's own connection";

/// An exit choice; the network is named only when there is more than one.
pub(crate) fn exit_choice(exit: &str, network: Option<&str>) -> String {
    match network {
        Some(network) => format!("{exit}{DOT}{network}"),
        None => exit.to_owned(),
    }
}

/// The tray item that opens what needs this person: `1 needs you…`, `2 need you…`.
pub(crate) fn needs_you(count: usize) -> String {
    let verb = if count == 1 { "needs" } else { "need" };
    format!("{count} {verb} you…")
}

/// Opens the window.
pub(crate) const OPEN: &str = "Open kith";
/// Quits the shell; the daemon keeps running.
pub(crate) const QUIT: &str = "Quit";

/// The autostart offer, made once.
pub(crate) const AUTOSTART_QUESTION: &str = "Keep kith in the menu bar when you log in?";
/// Accepts the autostart offer.
pub(crate) const AUTOSTART_YES: &str = "Keep in menu bar";
/// Declines the autostart offer.
pub(crate) const AUTOSTART_NO: &str = "Not now";

/// The only action a notification carries.
pub(crate) const REVIEW: &str = "Review";

/// A join request (admins only). Same words as `copy.notify.join`.
pub(crate) fn notify_join(device: &str, network: &str) -> String {
    format!("\"{device}\" wants to join {network}")
}

/// The body of a join request notification. Same words as `copy.notify.joinBody`.
pub(crate) const NOTIFY_JOIN_BODY: &str = "Open kith to check the code and approve.";

/// The exit in use went offline. Same words as `copy.notify.exitOffline`.
pub(crate) fn notify_exit_offline(exit: &str) -> String {
    format!("Your exit, {exit}, is offline")
}

/// What happens to internet traffic while the exit is offline and traffic is blocked.
pub(crate) const EXIT_OFFLINE_BLOCKED: &str = "Traffic is blocked.";
/// What happens to internet traffic while the exit is offline and traffic goes directly.
pub(crate) const EXIT_OFFLINE_DIRECT: &str = "Traffic now goes directly.";

/// A recently used device went offline. Same words as `copy.notify.deviceOffline`.
pub(crate) fn notify_device_offline(device: &str) -> String {
    format!("{device} went offline")
}

/// The daemon stopped for more than 10 s. Same words as `copy.notify.daemonStopped`.
pub(crate) const NOTIFY_DAEMON_STOPPED: &str = "kith isn't running on this device";

/// Several notifications in a short time, shown as one. Same words as
/// `copy.notify.coalesced`.
pub(crate) fn notify_coalesced(count: usize) -> String {
    format!("{count} things need you in kith")
}

/// Switching exits from the tray failed and the previous choice stays.
pub(crate) fn exit_switch_failed(exit: &str, current: Option<&str>) -> String {
    match current {
        Some(current) => format!(
            "Couldn't switch: {exit} isn't reachable right now. You're still using {current}."
        ),
        None => format!(
            "Couldn't switch: {exit} isn't reachable right now. You're still using this device's own connection."
        ),
    }
}

/// Anything else that failed. Same words as the page's `Internal` error.
pub(crate) fn internal(message: &str) -> String {
    format!("kith couldn't do this: {message}. Details are in Logs.")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The page's strings, which the shell's must match where both say the same thing.
    const PAGE_COPY: &str = include_str!("../../../crates/management/kith-ui/web/src/lib/copy.ts");

    fn shell_strings() -> Vec<String> {
        vec![
            connected("home", 11, 12),
            DAEMON_DOWN.to_owned(),
            NOT_IN_GROUP.to_owned(),
            VERSION_SKEW.to_owned(),
            NO_NETWORK.to_owned(),
            NO_EXIT.to_owned(),
            exit_choice("vps", Some("home")),
            needs_you(1),
            needs_you(2),
            OPEN.to_owned(),
            QUIT.to_owned(),
            AUTOSTART_QUESTION.to_owned(),
            AUTOSTART_YES.to_owned(),
            AUTOSTART_NO.to_owned(),
            REVIEW.to_owned(),
            notify_join("pixel-8", "home"),
            NOTIFY_JOIN_BODY.to_owned(),
            notify_exit_offline("vps"),
            EXIT_OFFLINE_BLOCKED.to_owned(),
            EXIT_OFFLINE_DIRECT.to_owned(),
            notify_device_offline("tv"),
            NOTIFY_DAEMON_STOPPED.to_owned(),
            notify_coalesced(4),
            exit_switch_failed("vps", Some("nas")),
            exit_switch_failed("vps", None),
            internal("the socket closed"),
        ]
    }

    #[test]
    fn the_shell_says_what_the_spec_says() {
        assert_eq!(
            connected("home", 11, 12),
            "● Connected to home · 11 of 12 devices"
        );
        assert_eq!(
            connected("home", 1, 1),
            "● Connected to home · 1 of 1 device"
        );
        assert_eq!(needs_you(1), "1 needs you…");
        assert_eq!(needs_you(3), "3 need you…");
        assert_eq!(OPEN, "Open kith");
        assert_eq!(QUIT, "Quit");
        assert_eq!(
            AUTOSTART_QUESTION,
            "Keep kith in the menu bar when you log in?"
        );
    }

    #[test]
    fn shared_strings_match_the_page() {
        let page = [
            (
                "`${quoted(device)} wants to join ${network}`",
                "\"${device}\" wants to join ${network}",
                notify_join("${device}", "${network}"),
            ),
            (
                "'Open kith to check the code and approve.'",
                NOTIFY_JOIN_BODY,
                NOTIFY_JOIN_BODY.to_owned(),
            ),
            (
                "`Your exit, ${exit}, is offline`",
                "Your exit, ${exit}, is offline",
                notify_exit_offline("${exit}"),
            ),
            (
                "`${device} went offline`",
                "${device} went offline",
                notify_device_offline("${device}"),
            ),
            (
                "\"kith isn't running on this device\"",
                NOTIFY_DAEMON_STOPPED,
                NOTIFY_DAEMON_STOPPED.to_owned(),
            ),
            (
                "`${n} things need you in kith`",
                "${n} things need you in kith",
                notify_coalesced(0).replacen('0', "${n}", 1),
            ),
            (
                "`No exit${DOT}this device's own connection`",
                "No exit · this device's own connection",
                NO_EXIT.to_owned(),
            ),
            (
                "Traffic is blocked.`",
                EXIT_OFFLINE_BLOCKED,
                EXIT_OFFLINE_BLOCKED.to_owned(),
            ),
            (
                "Traffic now goes directly.`",
                EXIT_OFFLINE_DIRECT,
                EXIT_OFFLINE_DIRECT.to_owned(),
            ),
            (
                "`kith couldn't do this: ${message}. Details are in Logs.`",
                "kith couldn't do this: ${message}. Details are in Logs.",
                internal("${message}"),
            ),
            (
                "`Couldn't switch: ${exit} isn't reachable right now. You're still using ${current}.`",
                "Couldn't switch: ${exit} isn't reachable right now. You're still using ${current}.",
                exit_switch_failed("${exit}", Some("${current}")),
            ),
        ];
        for (in_page, expected, shell) in page {
            assert!(
                PAGE_COPY.contains(in_page),
                "copy.ts no longer has {in_page}"
            );
            assert_eq!(shell, expected);
        }
    }

    #[test]
    fn no_banned_words() {
        for text in shell_strings() {
            let lower = text.to_lowercase();
            for banned in ["!", "successfully", "please", "oops", "error:", "loading…"] {
                assert!(!lower.contains(banned), "{text:?} says {banned:?}");
            }
            for word in ["peer", "node", "mesh", "journal", "dropped", "exit node"] {
                assert!(!lower.contains(word), "{text:?} uses {word:?}");
            }
        }
    }
}
