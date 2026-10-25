//! Operating-system integration: routes, addresses, DNS configuration and firewall.
//!
//! This is the only crate in the workspace that may contain `unsafe` code.
//! Every `unsafe` block must carry a `// SAFETY:` comment (enforced by lint).
//!
//! Plane: platform. Platform layer. See `docs/architecture.md`.
