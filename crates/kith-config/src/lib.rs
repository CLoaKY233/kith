//! Configuration schema, loading and validation.
//!
//! Plane: management. Library. See `docs/architecture.md`.
//!
//! The file is optional: every field has a default, and `Config::default()` is the zero-config
//! daemon. Parsing rejects unknown keys so a typo is an error with a line and column, and
//! [`Config::validate`] reports every semantic problem at once rather than the first.

#![forbid(unsafe_code)]

mod error;
mod schema;
mod validate;
mod wgquick;

#[cfg(test)]
mod tests;

pub use error::{ConfigError, ValidationError, ValidationErrors};
pub use schema::{
    AmneziaParams, Config, DaemonConfig, DnsConfig, DnsOverride, ExitConfig, ExitSelector,
    ImportWireguard, LogLevel, NetworkConfig, Obfuscation, ServerConfig, StealthConfig,
    TransportKind, TransportsConfig, Upstream,
};
pub use wgquick::{
    Endpoint, InterfaceAddress, WgInterface, WgPeer, WgPublicKey, WgQuickConfig, WgQuickError,
    WgQuickErrorKind, WgSecret,
};
