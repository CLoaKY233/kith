//! Live address changes, each confirmed by re-listing the device's addresses
//! because tun-rs ignores the kernel's netlink reply for IPv4 on Linux.

pub(crate) use platform::{add, remove};

#[cfg(any(
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
))]
mod platform {
    use std::io;
    use std::net::IpAddr;

    use tun_rs::AsyncDevice;

    use crate::TunError;

    /// Adds `address` unless the device has it; returns whether it was added.
    pub(crate) fn add(
        device: &AsyncDevice,
        address: IpAddr,
        prefix_len: u8,
    ) -> Result<bool, TunError> {
        if has(device, address)? {
            return Ok(false);
        }
        match address {
            IpAddr::V4(v4) => device.add_address_v4(v4, prefix_len)?,
            IpAddr::V6(v6) => device.add_address_v6(v6, prefix_len)?,
        }
        confirm(device, address, true)
    }

    /// Removes `address` if the device has it; returns whether it was removed.
    pub(crate) fn remove(device: &AsyncDevice, address: IpAddr) -> Result<bool, TunError> {
        if !has(device, address)? {
            return Ok(false);
        }
        device.remove_address(address)?;
        confirm(device, address, false)
    }

    fn has(device: &AsyncDevice, address: IpAddr) -> io::Result<bool> {
        Ok(device.addresses()?.contains(&address))
    }

    fn confirm(device: &AsyncDevice, address: IpAddr, present: bool) -> Result<bool, TunError> {
        if has(device, address)? == present {
            return Ok(true);
        }
        let action = if present { "add" } else { "remove" };
        Err(TunError::Io(io::Error::other(format!(
            "the OS did not {action} {address}"
        ))))
    }
}

#[cfg(not(any(
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
)))]
mod platform {
    use std::net::IpAddr;

    use tun_rs::AsyncDevice;

    use crate::TunError;

    const UNSUPPORTED: &str = "tun-rs cannot change a live device's addresses here";

    pub(crate) fn add(
        _device: &AsyncDevice,
        _address: IpAddr,
        _prefix_len: u8,
    ) -> Result<bool, TunError> {
        Err(TunError::Unsupported(UNSUPPORTED))
    }

    pub(crate) fn remove(_device: &AsyncDevice, _address: IpAddr) -> Result<bool, TunError> {
        Err(TunError::Unsupported(UNSUPPORTED))
    }
}
