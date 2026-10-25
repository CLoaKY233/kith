//! Idle timeouts per flow state.
//!
//! Based on the Linux conntrack defaults (`Documentation/networking/nf_conntrack-sysctl.rst`
//! and `net/netfilter/nf_conntrack_proto_{tcp,udp,icmp}.c`). Only the established TCP
//! timeout is scaled down, see [`TCP_ESTABLISHED`].

use core::time::Duration;

/// A SYN we sent that has not been answered.
///
/// Linux `nf_conntrack_tcp_timeout_syn_sent` = 120 s.
pub const TCP_SYN_SENT: Duration = Duration::from_secs(120);

/// An answered TCP connection with no traffic.
///
/// Linux uses 5 days (`nf_conntrack_tcp_timeout_established` = 432000 s), which pins
/// router memory for connections long gone. Two hours equals the default TCP
/// keepalive interval (`tcp_keepalive_time` = 7200 s, RFC 1122 §4.2.3.6), so any
/// connection that uses keepalives survives.
pub const TCP_ESTABLISHED: Duration = Duration::from_hours(2);

/// A connection after a FIN in either direction.
///
/// Linux `nf_conntrack_tcp_timeout_fin_wait` and `_time_wait` = 120 s (2 MSL).
pub const TCP_CLOSING: Duration = Duration::from_secs(120);

/// A connection after an RST in either direction.
///
/// Linux `nf_conntrack_tcp_timeout_close` = 10 s.
pub const TCP_CLOSE: Duration = Duration::from_secs(10);

/// UDP we sent with no reply yet.
///
/// Linux `nf_conntrack_udp_timeout` = 30 s.
pub const UDP_UNREPLIED: Duration = Duration::from_secs(30);

/// UDP seen in both directions.
///
/// Linux `nf_conntrack_udp_timeout_stream` = 120 s (180 s since Linux 5.17).
pub const UDP_STREAM: Duration = Duration::from_secs(120);

/// An ICMP echo request we sent.
///
/// Linux `nf_conntrack_icmp_timeout` and `nf_conntrack_icmpv6_timeout` = 30 s.
pub const ICMP_ECHO: Duration = Duration::from_secs(30);
