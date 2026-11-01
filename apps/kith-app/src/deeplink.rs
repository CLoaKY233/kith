//! `kith://join/<code>` links: an invite opened from a browser, a chat or a QR code. The
//! shell only checks the code's shape and hands it to the page's join flow, which asks
//! the daemon; nothing joins without the person confirming there.

/// The link's prefix, compared without regard to case: QR codes carry it in capitals.
const PREFIX: &str = "kith://join/";
/// An invite is a few hundred base32 characters; anything far longer is not one.
const MAX_CODE: usize = 2048;

/// The invite code in a `kith://join/` link, lowercased, or `None` for anything else.
pub(crate) fn join_code(link: &str) -> Option<String> {
    let head = link.get(..PREFIX.len())?;
    if !head.eq_ignore_ascii_case(PREFIX) {
        return None;
    }
    let code = link.get(PREFIX.len()..)?.trim_end_matches('/');
    let base32 = |c: char| c.is_ascii_alphabetic() || ('2'..='7').contains(&c);
    (!code.is_empty() && code.len() <= MAX_CODE && code.chars().all(base32))
        .then(|| code.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_join_links_with_a_base32_code_pass() {
        assert_eq!(
            join_code("kith://join/mfrggzdf"),
            Some("mfrggzdf".to_owned())
        );
        assert_eq!(
            join_code("KITH://JOIN/MFRGGZDF/"),
            Some("mfrggzdf".to_owned())
        );
        for refused in [
            "kith://join/",
            "kith://join/abc1",
            "kith://join/abc?x=1",
            "kith://request/mfrggzdf",
            "https://join/mfrggzdf",
            "kith:/",
            "kith://joïn/abc",
        ] {
            assert_eq!(join_code(refused), None, "{refused}");
        }
        assert_eq!(
            join_code(&format!("kith://join/{}", "a".repeat(MAX_CODE + 1))),
            None
        );
    }
}
