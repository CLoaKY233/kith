//! Looking up local groups, to let members of an operator group run kith without root.

/// The id of the local group `name`, or `None` if it does not exist.
#[cfg(unix)]
#[expect(unsafe_code, reason = "getgrnam_r has no safe std wrapper")]
#[must_use]
pub fn group_id(name: &str) -> Option<u32> {
    let name = std::ffi::CString::new(name).ok()?;
    let mut group = std::mem::MaybeUninit::<libc::group>::uninit();
    let mut buffer = vec![0_u8; 16 * 1024];
    let mut found: *mut libc::group = std::ptr::null_mut();
    // SAFETY: every pointer is valid for the call; `buffer` outlives it and its length
    // is passed, and `found` is only read after the call returns.
    let status = unsafe {
        libc::getgrnam_r(
            name.as_ptr(),
            group.as_mut_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &raw mut found,
        )
    };
    if status != 0 || found.is_null() {
        return None;
    }
    // SAFETY: a non-null `found` means `group` was filled in.
    Some(unsafe { group.assume_init() }.gr_gid)
}

/// Not supported off Unix.
#[cfg(not(unix))]
#[must_use]
pub fn group_id(_name: &str) -> Option<u32> {
    None
}

/// Whether user `uid` belongs to group `gid`, as primary or supplementary group.
#[cfg(unix)]
#[expect(unsafe_code, reason = "getpwuid_r has no safe std wrapper")]
#[must_use]
pub fn user_in_group(uid: u32, gid: u32) -> bool {
    let mut user = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut buffer = vec![0_u8; 16 * 1024];
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: as in `group_id`.
    let status = unsafe {
        libc::getpwuid_r(
            uid,
            user.as_mut_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &raw mut found,
        )
    };
    if status != 0 || found.is_null() {
        return false;
    }
    // SAFETY: a non-null `found` means `user` was filled in; its name points into
    // `buffer`, which is still alive.
    let user = unsafe { user.assume_init() };
    if user.pw_gid == gid {
        return true;
    }
    groups_of(user.pw_name, user.pw_gid).contains(&gid)
}

/// Not supported off Unix.
#[cfg(not(unix))]
#[must_use]
pub fn user_in_group(_uid: u32, _gid: u32) -> bool {
    false
}

/// macOS lists groups as `c_int`, other systems as `gid_t`.
#[cfg(target_os = "macos")]
type GroupEntry = libc::c_int;
#[cfg(all(unix, not(target_os = "macos")))]
type GroupEntry = libc::gid_t;

#[cfg(target_os = "macos")]
const fn to_entry(gid: u32) -> GroupEntry {
    gid.cast_signed()
}
#[cfg(target_os = "macos")]
const fn from_entry(entry: GroupEntry) -> u32 {
    entry.cast_unsigned()
}
#[cfg(all(unix, not(target_os = "macos")))]
const fn to_entry(gid: u32) -> GroupEntry {
    gid
}
#[cfg(all(unix, not(target_os = "macos")))]
const fn from_entry(entry: GroupEntry) -> u32 {
    entry
}

#[cfg(unix)]
#[expect(unsafe_code, reason = "getgrouplist has no safe std wrapper")]
fn groups_of(name: *const libc::c_char, primary: libc::gid_t) -> Vec<u32> {
    let mut count: libc::c_int = 64;
    loop {
        let mut groups: Vec<GroupEntry> = vec![0; usize::try_from(count).unwrap_or(64)];
        let asked = count;
        // SAFETY: `name` is a valid C string, and `groups` holds `count` entries.
        let status = unsafe {
            libc::getgrouplist(name, to_entry(primary), groups.as_mut_ptr(), &raw mut count)
        };
        if status >= 0 {
            groups.truncate(usize::try_from(count).unwrap_or(0));
            return groups.into_iter().map(from_entry).collect();
        }
        if asked >= 4096 {
            return Vec::new();
        }
        count = asked.saturating_mul(4).max(count);
    }
}
