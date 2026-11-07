//! The page's TypeScript types, generated from the Rust ones so the two cannot drift.
//!
//! `cargo xtask ui-types` writes `kith-ui/web/src/lib/types.gen.ts`; the test below fails
//! when that file is not what this module would write.

use ts_rs::{Config, TS};

use crate::wire;

/// The first lines of the generated file.
const HEADER: &str = "// Generated from kith-ipc and kith-ui-core by `cargo xtask ui-types`. Do not edit.\n\
// Ids are lowercase hex. Times ending in `_ms` are Unix milliseconds.\n";

fn push<T: TS>(out: &mut String, config: &Config) {
    out.push('\n');
    if let Some(docs) = T::docs() {
        out.push_str(&docs);
    }
    out.push_str("export ");
    out.push_str(&T::decl(config));
    out.push('\n');
}

/// Every type the page reads or writes, as one TypeScript module.
///
/// 64-bit integers become `number`: JSON carries them as numbers and every value the
/// page sees (bytes, milliseconds, sequence numbers) stays below 2^53.
pub fn declarations() -> String {
    let config = Config::new().with_large_int("number");
    let mut out = String::from(HEADER);
    push::<kith_ipc::PathKind>(&mut out, &config);
    push::<kith_ipc::Problem>(&mut out, &config);
    push::<kith_ipc::Warning>(&mut out, &config);
    push::<kith_ipc::NetworkSummary>(&mut out, &config);
    push::<kith_ipc::ImportedInfo>(&mut out, &config);
    push::<kith_ipc::Status>(&mut out, &config);
    push::<kith_ipc::PeerInfo>(&mut out, &config);
    push::<kith_ipc::EntryKind>(&mut out, &config);
    push::<kith_ipc::Entry>(&mut out, &config);
    push::<kith_ipc::TrafficSample>(&mut out, &config);
    push::<kith_ipc::TrafficSeries>(&mut out, &config);
    push::<kith_ipc::LinkInfo>(&mut out, &config);
    push::<kith_ipc::LinkReport>(&mut out, &config);
    push::<kith_ipc::PendingJoin>(&mut out, &config);
    push::<kith_ipc::BlockedInfo>(&mut out, &config);
    push::<kith_ipc::AccountRole>(&mut out, &config);
    push::<kith_ipc::CredentialInfo>(&mut out, &config);
    push::<kith_ipc::AccountInfo>(&mut out, &config);
    push::<kith_ipc::AccountsInfo>(&mut out, &config);
    push::<wire::Versions>(&mut out, &config);
    push::<wire::Role>(&mut out, &config);
    push::<wire::Membership>(&mut out, &config);
    push::<wire::Host>(&mut out, &config);
    push::<wire::ExitState>(&mut out, &config);
    push::<wire::StatusView>(&mut out, &config);
    push::<wire::ExitView>(&mut out, &config);
    push::<wire::RateBand>(&mut out, &config);
    push::<wire::TrafficTick>(&mut out, &config);
    push::<wire::LinkTick>(&mut out, &config);
    push::<wire::Tick>(&mut out, &config);
    push::<wire::Reset>(&mut out, &config);
    push::<wire::Gap>(&mut out, &config);
    push::<wire::Claimed>(&mut out, &config);
    push::<wire::Seen>(&mut out, &config);
    push::<wire::JoinRequest>(&mut out, &config);
    push::<wire::BlockedGroup>(&mut out, &config);
    push::<wire::Pending>(&mut out, &config);
    push::<wire::Boot>(&mut out, &config);
    push::<wire::RecordType>(&mut out, &config);
    push::<wire::RecordView>(&mut out, &config);
    push::<wire::ApiErrorCode>(&mut out, &config);
    push::<wire::Resolution>(&mut out, &config);
    push::<wire::Conflict>(&mut out, &config);
    push::<wire::ApiErrorBody>(&mut out, &config);
    push::<wire::OutcomeState>(&mut out, &config);
    push::<wire::Outcome>(&mut out, &config);
    push::<wire::SessionIn>(&mut out, &config);
    push::<wire::CreateNetworkIn>(&mut out, &config);
    push::<wire::JoinIn>(&mut out, &config);
    push::<wire::RequestJoinIn>(&mut out, &config);
    push::<wire::JoinCode>(&mut out, &config);
    push::<wire::JoinRequestState>(&mut out, &config);
    push::<wire::InviteIn>(&mut out, &config);
    push::<wire::Invite>(&mut out, &config);
    push::<wire::ApproveIn>(&mut out, &config);
    push::<wire::OpenInvite>(&mut out, &config);
    push::<kith_ipc::SelectorInfo>(&mut out, &config);
    push::<kith_ipc::TargetInfo>(&mut out, &config);
    push::<kith_ipc::PolicyRuleInfo>(&mut out, &config);
    push::<kith_ipc::PolicyInfo>(&mut out, &config);
    push::<wire::PolicyIn>(&mut out, &config);
    push::<wire::RelayView>(&mut out, &config);
    push::<wire::RelayIn>(&mut out, &config);
    push::<wire::ServerView>(&mut out, &config);
    push::<wire::ImportIn>(&mut out, &config);
    push::<wire::Imported>(&mut out, &config);
    push::<wire::ImportUseIn>(&mut out, &config);
    push::<wire::PhoneIn>(&mut out, &config);
    push::<kith_ipc::GatewayInfo>(&mut out, &config);
    push::<wire::PhoneCreated>(&mut out, &config);
    push::<wire::PhoneConfigIn>(&mut out, &config);
    push::<wire::PhoneConfig>(&mut out, &config);
    push::<wire::RecordIn>(&mut out, &config);
    push::<wire::ExitIn>(&mut out, &config);
    push::<wire::GrantIn>(&mut out, &config);
    push::<wire::RenameIn>(&mut out, &config);
    push::<wire::RemoveIn>(&mut out, &config);
    push::<wire::AllowIn>(&mut out, &config);
    push::<wire::AllowPreview>(&mut out, &config);
    push::<wire::NameProblem>(&mut out, &config);
    push::<wire::NameCheck>(&mut out, &config);
    push::<wire::Fixture>(&mut out, &config);
    console(&mut out, &config);
    out
}

/// The Console's types: people, passkeys, authority prompts, codes and history.
fn console(out: &mut String, config: &Config) {
    push::<wire::PersonRole>(out, config);
    push::<wire::PersonState>(out, config);
    push::<wire::PersonView>(out, config);
    push::<wire::ConsoleMode>(out, config);
    push::<wire::Me>(out, config);
    push::<wire::NetworkRef>(out, config);
    push::<wire::MeView>(out, config);
    push::<wire::CeremonyOptions>(out, config);
    push::<wire::CredentialIn>(out, config);
    push::<wire::SetupInfo>(out, config);
    push::<wire::SetupBeginIn>(out, config);
    push::<wire::TokenCredentialIn>(out, config);
    push::<wire::AuthorityKind>(out, config);
    push::<wire::AuthorityChallenge>(out, config);
    push::<wire::SetupFinished>(out, config);
    push::<wire::RecoveryWords>(out, config);
    push::<wire::RegisterInfo>(out, config);
    push::<wire::TokenIn>(out, config);
    push::<wire::PersonIn>(out, config);
    push::<wire::PersonInvite>(out, config);
    push::<wire::RoleIn>(out, config);
    push::<wire::AuthorityChangeIn>(out, config);
    push::<wire::AuthorityBeginIn>(out, config);
    push::<wire::DeviceCodeIn>(out, config);
    push::<wire::DeviceCodeView>(out, config);
    push::<wire::JoinLink>(out, config);
    push::<wire::StoredEntry>(out, config);
    push::<wire::JournalPage>(out, config);
    push::<wire::TrafficMinute>(out, config);
    push::<wire::TrafficHistory>(out, config);
}

#[cfg(test)]
#[expect(
    clippy::panic_in_result_fn,
    reason = "assertions fail the test; ? keeps the setup short"
)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// Set to write the file instead of checking it.
    const WRITE: &str = "KITH_UI_WRITE_TYPES";

    #[test]
    fn types_gen_ts_is_current() -> std::io::Result<()> {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../kith-ui/web/src/lib/types.gen.ts");
        let fresh = declarations();
        if std::env::var_os(WRITE).is_some() {
            return std::fs::write(&path, fresh);
        }
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            current == fresh,
            "{} is stale; run `cargo xtask ui-types`",
            path.display()
        );
        Ok(())
    }
}
