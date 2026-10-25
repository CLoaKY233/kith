use std::fmt;
use std::path::PathBuf;

use crate::wgquick::WgQuickError;

/// Why a configuration could not be loaded.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// The file could not be read.
    #[error("cannot read {}: {source}", path.display())]
    Read {
        /// The file.
        path: PathBuf,
        /// The OS error.
        source: std::io::Error,
    },
    /// The file exceeds the size limit.
    #[error("{} is larger than {limit} bytes", path.display())]
    TooLarge {
        /// The file.
        path: PathBuf,
        /// The limit in bytes.
        limit: u64,
    },
    /// The TOML is malformed or has unknown keys; the message carries line and column.
    #[error("{0}")]
    Parse(Box<toml::de::Error>),
    /// The TOML parsed but violates the schema's rules.
    #[error("{0}")]
    Invalid(ValidationErrors),
    /// The configuration could not be rendered as TOML.
    #[error("cannot render configuration: {0}")]
    Render(#[from] toml::ser::Error),
    /// An imported wg-quick file is malformed.
    #[error("{}: {source}", path.display())]
    WgQuick {
        /// The file.
        path: PathBuf,
        /// The parse error.
        source: WgQuickError,
    },
}

/// One semantic problem, located by its dotted key path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    /// Dotted key path, e.g. `network.mtu`.
    pub field: String,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

/// Every semantic problem found in one configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationErrors(pub Vec<ValidationError>);

impl fmt::Display for ValidationErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid configuration:")?;
        for error in &self.0 {
            write!(f, "\n  {error}")?;
        }
        Ok(())
    }
}

impl core::error::Error for ValidationErrors {}
