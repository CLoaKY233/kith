//! The local time zone, for showing times the way the user reads them.

/// How far local time is ahead of UTC at `unix_secs`, in seconds; 0 if unknown.
#[cfg(unix)]
#[expect(unsafe_code, reason = "localtime_r has no safe std wrapper")]
#[cfg_attr(
    all(target_env = "musl", target_pointer_width = "32"),
    expect(
        deprecated,
        reason = "libc warns that musl's time_t is changing to 64 bits"
    )
)]
#[must_use]
pub fn utc_offset_secs(unix_secs: i64) -> i64 {
    let Some(time) = convert::<libc::time_t, _>(unix_secs) else {
        return 0;
    };
    let mut tm = std::mem::MaybeUninit::<libc::tm>::uninit();
    // SAFETY: both pointers are valid for the call; localtime_r fills `tm` or returns null.
    let filled = unsafe { libc::localtime_r(&raw const time, tm.as_mut_ptr()) };
    if filled.is_null() {
        return 0;
    }
    // SAFETY: a non-null result means `tm` was filled in.
    convert(unsafe { tm.assume_init() }.tm_gmtoff).unwrap_or(0)
}

/// Converts between integer types whose widths differ by target (`time_t`, `c_long`).
#[cfg(unix)]
fn convert<T: TryFrom<F>, F>(value: F) -> Option<T> {
    T::try_from(value).ok()
}

/// Not supported off Unix: times show in UTC.
#[cfg(not(unix))]
#[must_use]
pub const fn utc_offset_secs(_unix_secs: i64) -> i64 {
    0
}
