//! Argument grammar. Networks and nodes stay as text here; [`crate::exec`] resolves names.

use std::path::PathBuf;

use bpaf::Bpaf;

use crate::duration;

const DEFAULT_EXPIRY_SECS: u64 = 24 * 60 * 60;

/// Top-level options.
#[derive(Debug, Clone, PartialEq, Eq, Bpaf)]
#[bpaf(options, version)]
pub(crate) struct Opts {
    /// Daemon socket (default: from the config file)
    #[bpaf(long, env("KITH_SOCKET"), argument("PATH"))]
    pub socket: Option<PathBuf>,
    #[bpaf(external(cmd))]
    pub cmd: Cmd,
}

/// A subcommand.
#[derive(Debug, Clone, PartialEq, Eq, Bpaf)]
pub(crate) enum Cmd {
    /// Turn kith on, now and at every boot
    #[bpaf(command)]
    Up,
    /// Turn kith off, and keep it off after a restart
    #[bpaf(command)]
    Down,
    /// Show networks, this device and peers
    #[bpaf(command)]
    Status {
        /// Print JSON
        json: bool,
    },
    /// Create a network with this device as its first member
    #[bpaf(command)]
    Create {
        /// This device's name (default: hostname)
        #[bpaf(argument("NAME"))]
        device_name: Option<String>,
        #[bpaf(positional("NETWORK-NAME"))]
        name: String,
    },
    /// Print a join link for a network
    #[bpaf(command)]
    Invite {
        /// How long the link works, like 30m, 24h or 7d
        #[bpaf(argument::<String>("DURATION"), parse(duration::parse_secs), fallback(DEFAULT_EXPIRY_SECS))]
        expires: u64,
        /// How many devices may join, or `unlimited`
        #[bpaf(argument::<String>("N"), parse(parse_uses), fallback(Some(1)))]
        uses: Option<u32>,
        #[bpaf(positional("NETWORK"))]
        network: String,
    },
    /// Join a network with an invite link
    #[bpaf(command)]
    Join {
        /// This device's name (default: hostname)
        #[bpaf(argument("NAME"))]
        device_name: Option<String>,
        #[bpaf(positional("LINK"))]
        link: String,
    },
    /// Remove another device from a network (admins only)
    #[bpaf(command)]
    Remove {
        #[bpaf(positional("NETWORK"))]
        network: String,
        /// The device's name or id
        #[bpaf(positional("DEVICE"))]
        device: String,
    },
    /// Leave a network
    #[bpaf(command)]
    Leave {
        #[bpaf(positional("NETWORK"))]
        network: String,
    },
    /// List a network's peers
    #[bpaf(command)]
    Peers {
        /// Print JSON
        json: bool,
        #[bpaf(positional("NETWORK"))]
        network: String,
    },
    /// Manage DNS records
    #[bpaf(command)]
    Dns(#[bpaf(external(dns_cmd))] DnsCmd),
    /// Use, allow or deny exit nodes
    #[bpaf(command)]
    Exit(#[bpaf(external(exit_cmd))] ExitCmd),
    /// Advertise or remove relay servers
    #[bpaf(command)]
    Relay(#[bpaf(external(relay_cmd))] RelayCmd),
    /// Offer this device as an exit node and/or relay
    #[bpaf(command)]
    Server {
        /// Act as an exit node
        exit: bool,
        /// Act as a relay
        relay: bool,
    },
    /// Import or export plain WireGuard configurations
    #[bpaf(command)]
    Wg(#[bpaf(external(wg_cmd))] WgCmd),
    /// Stream daemon events
    #[bpaf(command)]
    Watch,
    /// Check what is wrong and say how to fix it
    #[bpaf(command)]
    Doctor,
    /// Check or show the configuration file
    #[bpaf(command)]
    Config(#[bpaf(external(config_cmd))] ConfigCmd),
    /// Run the background daemon
    #[bpaf(command)]
    Daemon {
        /// Configuration file
        #[bpaf(argument("PATH"))]
        config: Option<PathBuf>,
        /// Stay attached to the terminal (accepted for service managers; always true)
        foreground: bool,
        /// Log more than the configured level: -v for debug, -vv for trace
        #[bpaf(short, long, req_flag(()), count)]
        verbose: usize,
    },
}

/// `kith dns …`.
#[derive(Debug, Clone, PartialEq, Eq, Bpaf)]
pub(crate) enum DnsCmd {
    /// Create or replace a record
    #[bpaf(command)]
    Set {
        #[bpaf(positional("NETWORK"))]
        network: String,
        #[bpaf(positional("NAME"))]
        name: String,
        #[bpaf(positional("TYPE"))]
        kind: String,
        #[bpaf(positional("VALUE"))]
        value: String,
    },
    /// Delete a record
    #[bpaf(command)]
    Rm {
        #[bpaf(positional("NETWORK"))]
        network: String,
        #[bpaf(positional("NAME"))]
        name: String,
    },
}

/// `kith exit …`.
#[derive(Debug, Clone, PartialEq, Eq, Bpaf)]
pub(crate) enum ExitCmd {
    /// Use a peer as exit node
    #[bpaf(command)]
    Use {
        #[bpaf(positional("NETWORK"))]
        network: String,
        #[bpaf(positional("NODE"))]
        node: String,
    },
    /// Stop using an exit node
    #[bpaf(command)]
    Off {
        /// Needed only when this device is in several networks
        #[bpaf(positional("NETWORK"))]
        network: Option<String>,
    },
    /// Let a node act as an exit node (admins only)
    #[bpaf(command)]
    Allow {
        #[bpaf(positional("NETWORK"))]
        network: String,
        #[bpaf(positional("NODE"))]
        node: String,
    },
    /// Stop a node acting as an exit node (admins only)
    #[bpaf(command)]
    Deny {
        #[bpaf(positional("NETWORK"))]
        network: String,
        #[bpaf(positional("NODE"))]
        node: String,
    },
}

/// `kith relay …`.
#[derive(Debug, Clone, PartialEq, Eq, Bpaf)]
pub(crate) enum RelayCmd {
    /// Advertise a relay to a network's members (admins only)
    #[bpaf(command)]
    Advertise {
        /// The node running the relay (default: this device)
        #[bpaf(argument("NODE"))]
        node: Option<String>,
        #[bpaf(positional("NETWORK"))]
        network: String,
        #[bpaf(positional("NAME"))]
        name: String,
        #[bpaf(positional("HOST:PORT"))]
        address: String,
    },
    /// Stop advertising a relay (admins only)
    #[bpaf(command)]
    Remove {
        #[bpaf(positional("NETWORK"))]
        network: String,
        #[bpaf(positional("NAME"))]
        name: String,
    },
}

/// `kith wg …`.
#[derive(Debug, Clone, PartialEq, Eq, Bpaf)]
pub(crate) enum WgCmd {
    /// Import a wg-quick file as a network
    #[bpaf(command)]
    Import {
        #[bpaf(positional("NAME"))]
        name: String,
        #[bpaf(positional("FILE"))]
        file: PathBuf,
    },
    /// Add a device that runs the WireGuard app, such as a phone, and show its QR codes
    #[bpaf(command)]
    Export {
        /// Give it a policy tag; repeat for more
        #[bpaf(long("tag"), argument("TAG"))]
        tags: Vec<String>,
        /// Print the config files instead of QR codes
        #[bpaf(long)]
        text: bool,
        #[bpaf(positional("NETWORK"))]
        network: String,
        #[bpaf(positional("DEVICE-NAME"))]
        device_name: String,
    },
}

/// `kith config …`.
#[derive(Debug, Clone, PartialEq, Eq, Bpaf)]
pub(crate) enum ConfigCmd {
    /// Validate a configuration file
    #[bpaf(command)]
    Check {
        #[bpaf(positional("PATH"))]
        path: Option<PathBuf>,
    },
    /// Print the effective configuration
    #[bpaf(command)]
    Show {
        #[bpaf(positional("PATH"))]
        path: Option<PathBuf>,
    },
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "bpaf parse hooks take ownership"
)]
fn parse_uses(text: String) -> Result<Option<u32>, String> {
    if text == "unlimited" {
        return Ok(None);
    }
    match text.parse::<u32>() {
        Ok(0) | Err(_) => Err(format!("`{text}` is not a positive count or `unlimited`")),
        Ok(n) => Ok(Some(n)),
    }
}
