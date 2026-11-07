//! People in a network: listing them, the first owner, approvals from passkeys and the
//! recovery key, device ownership, and acting on someone's behalf.
//!
//! Changes to people are drafted here ([`Action::PrepareAuthority`]), approved elsewhere
//! (a browser's passkey prompt signs the draft's id), and published here only if the
//! network accepts them, so a refused approval never reaches other devices.

use std::sync::Arc;

use kith_crypto::webauthn::encode_challenge;
use kith_identity::RecoveryCode;
use kith_ipc::{AccountInfo, AccountRole, AccountsInfo, CredentialInfo, Response};
use kith_ledger::{Approval, Credential, Ledger, OpBody, Role, SignedOp, UnsignedOp};
use kith_types::{AccountId, NetworkId, NodeId};

use super::Daemon;
use crate::channel::now;
use crate::types::{Action, NodeError};

impl Daemon {
    /// Performs one of this module's actions.
    pub(super) async fn people(&mut self, action: Action) -> Result<Response, NodeError> {
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "execute passes only this module's actions"
        )]
        match action {
            Action::Accounts { network } => self.accounts(network),
            Action::AddAccount {
                network,
                name,
                credential,
            } => self.add_first_owner(network, name, credential).await,
            Action::AssignDevice {
                network,
                device,
                account,
            } => self.assign_device(network, device, account).await,
            Action::PrepareAuthority { network, change } => self.prepare(network, *change),
            Action::SubmitAuthority {
                network,
                draft,
                approval,
            } => self.submit(network, *draft, *approval).await,
            Action::RecoveryKey { network } => self.recovery_key(network).await,
            Action::AsActor { actor, action } => {
                self.networks.set_actor(Some(actor));
                let result = Box::pin(self.execute(*action)).await;
                self.networks.set_actor(None);
                result
            }
            _ => Err(NodeError::InvalidArgument("not a people action".to_owned())),
        }
    }

    fn accounts(&self, network: NetworkId) -> Result<Response, NodeError> {
        self.member_map(network)?;
        let info = self.ledger_of(network, |ledger| {
            let view = ledger.view();
            AccountsInfo {
                accounts: view
                    .accounts()
                    .iter()
                    .map(|(id, account)| AccountInfo {
                        id: *id,
                        name: account.name.clone(),
                        role: match account.role {
                            Role::Owner => AccountRole::Owner,
                            Role::Admin => AccountRole::Admin,
                            Role::Member => AccountRole::Member,
                        },
                        credentials: account
                            .credentials
                            .iter()
                            .map(|credential| CredentialInfo {
                                credential_id: credential.id.clone(),
                                public_key: credential.public_key.clone(),
                                rp_id: credential.rp_id.clone(),
                            })
                            .collect(),
                        devices: view.devices_of(id).collect(),
                    })
                    .collect(),
                consoles: view.consoles().iter().copied().collect(),
                recovery_set: view.recovery_key().is_some(),
            }
        })?;
        Ok(Response::Accounts(info))
    }

    /// Adds the first owner, signed by this device as an admin.
    async fn add_first_owner(
        &mut self,
        network: NetworkId,
        name: String,
        credential: Credential,
    ) -> Result<Response, NodeError> {
        self.member_map(network)?;
        self.founding_admin(network, "add the first owner")?;
        let account = credential.account_id();
        let body = OpBody::AddAccount {
            name,
            role: Role::Owner,
            credential,
        };
        let draft = self.draft(network, body)?;
        let op = draft.sign(self.identity.signing_key())?;
        self.publish(network, op).await?;
        Ok(Response::AccountAdded { account })
    }

    /// Creates the recovery key and publishes its public half; the words are returned once.
    async fn recovery_key(&mut self, network: NetworkId) -> Result<Response, NodeError> {
        self.member_map(network)?;
        if self.ledger_of(network, |ledger| ledger.view().recovery_key().is_some())? {
            return Err(NodeError::InvalidArgument(
                "this network already has a recovery key".to_owned(),
            ));
        }
        self.founding_admin(network, "create the recovery key")?;
        let code = RecoveryCode::generate()
            .map_err(|error| NodeError::InvalidArgument(error.to_string()))?;
        let body = OpBody::SetRecoveryKey {
            key: code.signing_key().verifying_key(),
        };
        let op = self
            .draft(network, body)?
            .sign(self.identity.signing_key())?;
        self.publish(network, op).await?;
        Ok(Response::RecoveryKey {
            words: code.to_words().to_string(),
        })
    }

    /// Before a network has people, its admin devices found them; afterwards they cannot.
    fn founding_admin(&self, network: NetworkId, what: &str) -> Result<(), NodeError> {
        let local = self.identity.node_id();
        let (admin, people) = self.ledger_of(network, |ledger| {
            let view = ledger.view();
            (view.admins().contains(&local), !view.accounts().is_empty())
        })?;
        if people {
            return Err(NodeError::InvalidArgument(format!(
                "this network already has people; only before the first one can a device {what}"
            )));
        }
        if !admin {
            return Err(NodeError::InvalidArgument(format!(
                "only an admin device of this network can {what}"
            )));
        }
        Ok(())
    }

    async fn assign_device(
        &mut self,
        network: NetworkId,
        device: NodeId,
        account: AccountId,
    ) -> Result<Response, NodeError> {
        self.member_map(network)?;
        let (member, known) = self.ledger_of(network, |ledger| {
            let view = ledger.view();
            (view.is_member(&device), view.account(&account).is_some())
        })?;
        if !member {
            return Err(NodeError::NotFound(format!(
                "{device} is not in this network"
            )));
        }
        if !known {
            return Err(NodeError::NotFound(format!("no person {account}")));
        }
        if !self.is_admin(network)? {
            return Err(NodeError::InvalidArgument(
                "only an admin of this network can assign devices".to_owned(),
            ));
        }
        let identity = Arc::clone(&self.identity);
        self.runtime_mut(network)?.append(
            &identity,
            OpBody::AssignDevice { device, account },
            now(),
        )?;
        self.commit(network).await?;
        Ok(Response::DeviceAssigned)
    }

    /// Drafts `change` by this device: the op bytes to send back, and the challenge.
    fn prepare(&self, network: NetworkId, change: OpBody) -> Result<Response, NodeError> {
        self.member_map(network)?;
        let draft = self.draft(network, change)?;
        let op_bytes = draft.encode().map_err(invalid)?;
        let challenge = draft.id().map_err(invalid)?;
        Ok(Response::AuthorityPrepared {
            op_bytes,
            challenge: encode_challenge(challenge.as_bytes()),
        })
    }

    async fn submit(
        &mut self,
        network: NetworkId,
        draft: UnsignedOp,
        approval: Approval,
    ) -> Result<Response, NodeError> {
        self.member_map(network)?;
        if draft.network != Some(network) || draft.author != self.identity.node_id() {
            return Err(NodeError::InvalidArgument(
                "this change was not prepared by this device for this network".to_owned(),
            ));
        }
        let challenge = draft.id().map_err(invalid)?;
        self.check_approval(network, &approval, &challenge)?;
        let op = draft
            .approve(approval)
            .map_err(invalid)?
            .sign(self.identity.signing_key())?;
        self.publish(network, op).await?;
        Ok(Response::AuthorityApplied)
    }

    /// Explains a bad signature before the ledger silently ignores it.
    fn check_approval(
        &self,
        network: NetworkId,
        approval: &Approval,
        challenge: &kith_types::OpId,
    ) -> Result<(), NodeError> {
        self.ledger_of(network, |ledger| {
            let view = ledger.view();
            match approval {
                Approval::Passkey(assertion) => {
                    let credential = view
                        .account(&assertion.account)
                        .and_then(|account| {
                            account
                                .credentials
                                .iter()
                                .find(|credential| credential.id == assertion.credential_id)
                        })
                        .ok_or_else(|| {
                            NodeError::InvalidArgument(
                                "that passkey belongs to nobody in this network".to_owned(),
                            )
                        })?;
                    assertion.verify(credential, challenge).map_err(|error| {
                        NodeError::InvalidArgument(format!(
                            "the passkey's approval failed: {error}"
                        ))
                    })
                }
                Approval::Recovery(_) => {
                    if view.recovery_key().is_some() {
                        Ok(())
                    } else {
                        Err(NodeError::InvalidArgument(
                            "this network has no recovery key".to_owned(),
                        ))
                    }
                }
            }
        })?
    }

    /// Drafts an op by this device on the network's current heads.
    fn draft(&self, network: NetworkId, body: OpBody) -> Result<UnsignedOp, NodeError> {
        let local = self.identity.node_id();
        let at = now();
        let draft = self.ledger_of(network, |ledger| ledger.draft(local, body, at))?;
        draft.encode().map_err(invalid)?;
        Ok(draft)
    }

    /// Inserts an account op only if the network accepts it, then shares it.
    async fn publish(&mut self, network: NetworkId, op: SignedOp) -> Result<(), NodeError> {
        let id = op.id();
        self.runtime_mut(network)?.with_ledger(|ledger| {
            let mut trial = ledger.clone();
            trial.insert(op.clone())?;
            if !trial.view().account_changes().contains(&id) {
                return Err(NodeError::InvalidArgument(
                    "the network did not accept this change: the approver's role does not \
                     allow it, or people changed since it was prepared (prepare it again)"
                        .to_owned(),
                ));
            }
            *ledger = trial;
            Ok(())
        })??;
        self.commit(network).await
    }

    fn ledger_of<R>(
        &self,
        network: NetworkId,
        f: impl FnOnce(&Ledger) -> R,
    ) -> Result<R, NodeError> {
        self.networks
            .get(&network)
            .ok_or_else(|| NodeError::NotFound(network.to_string()))?
            .with_ledger(|ledger| f(ledger))
    }
}

fn invalid(error: impl core::fmt::Display) -> NodeError {
    NodeError::InvalidArgument(error.to_string())
}
