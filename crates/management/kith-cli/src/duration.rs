//! `30m`, `24h`, `7d`, `1h30m`.

/// Parses a duration into whole seconds. Zero is rejected.
#[expect(
    clippy::needless_pass_by_value,
    reason = "bpaf parse hooks take ownership"
)]
pub(crate) fn parse_secs(text: String) -> Result<u64, String> {
    let bad = || format!("invalid duration `{text}`; use forms like 30m, 24h or 7d");
    let mut total: u64 = 0;
    let mut digits_start = None;
    for (i, ch) in text.char_indices() {
        if ch.is_ascii_digit() {
            digits_start.get_or_insert(i);
            continue;
        }
        let start = digits_start.take().ok_or_else(bad)?;
        let unit: u64 = match ch {
            's' => 1,
            'm' => 60,
            'h' => 3600,
            'd' => 86_400,
            'w' => 604_800,
            _ => return Err(bad()),
        };
        let value: u64 = text
            .get(start..i)
            .ok_or_else(bad)?
            .parse()
            .map_err(|_| bad())?;
        total = value
            .checked_mul(unit)
            .and_then(|secs| total.checked_add(secs))
            .ok_or_else(bad)?;
    }
    if digits_start.is_some() || total == 0 {
        return Err(bad());
    }
    Ok(total)
}
