//! Split DNS: which upstream group answers which domain.
//!
//! The core only decides *where* a query goes; the driver owns the upstream sockets. A
//! route covers its suffix and every name below it, and the longest matching suffix
//! wins, so `corp.example.com → A` and `example.com → B` send `x.corp.example.com` to
//! A and `www.example.com` to B.

use alloc::collections::BTreeMap;

use crate::name::{DomainName, self_and_ancestors};

/// Most routes in one [`SplitDns`] table.
pub const MAX_ROUTES: usize = 256;

/// Opaque handle for a set of upstream resolvers, chosen and interpreted by the driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UpstreamGroupId(pub u32);

/// Why a route was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SplitDnsError {
    /// The suffix already has a route.
    #[error("duplicate split-DNS route for {0}")]
    DuplicateRoute(DomainName),
    /// Routes are suffixes, so a wildcard label is meaningless.
    #[error("split-DNS route {0} is a wildcard")]
    WildcardRoute(DomainName),
    /// More than [`MAX_ROUTES`] routes.
    #[error("more than {} split-DNS routes", MAX_ROUTES)]
    TooManyRoutes,
}

/// Suffix → upstream group table. Immutable once handed to the answer engine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SplitDns {
    routes: BTreeMap<DomainName, UpstreamGroupId>,
}

impl SplitDns {
    /// An empty table: every name outside the zone goes to the default upstream.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sends `suffix` and every name below it to `group`.
    pub fn add_route(
        &mut self,
        suffix: DomainName,
        group: UpstreamGroupId,
    ) -> Result<(), SplitDnsError> {
        if suffix.is_wildcard() {
            return Err(SplitDnsError::WildcardRoute(suffix));
        }
        if self.routes.contains_key(&suffix) {
            return Err(SplitDnsError::DuplicateRoute(suffix));
        }
        if self.routes.len() >= MAX_ROUTES {
            return Err(SplitDnsError::TooManyRoutes);
        }
        self.routes.insert(suffix, group);
        Ok(())
    }

    /// The group for the longest route covering `name`, if any.
    pub fn route_for(&self, name: &DomainName) -> Option<UpstreamGroupId> {
        self_and_ancestors(name.as_str()).find_map(|suffix| self.exact(suffix))
    }

    /// The group routed at exactly `suffix`.
    pub(crate) fn exact(&self, suffix: &str) -> Option<UpstreamGroupId> {
        self.routes.get(suffix).copied()
    }
}
