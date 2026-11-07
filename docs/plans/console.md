# kith Console and accounts

The core (`kith`: daemon, CLI, ledger, mesh) stays as it is and works alone. The Console is a plugin: the `kith-ui` binary grown into a control panel that can be reached from anywhere, with people and passkeys. Read with `docs/plans/kith-ui-design.md` (look and feel) and `docs/plans/kith-ui-api.md` (current HTTP API).

## Principles

- The ledger is the only authority for who is in the network and who may change it. The Console's database holds memory and history, never authority. Deleting it loses history and sessions, nothing else.
- Admins are people, not machines. Losing the server that runs the Console never loses control of the network.
- Three roles only: Owner, Admin, Member. No custom roles.
- Anything not needed in the first week stays out of sight.

## Identity in the ledger

New ledger concepts (new `OpBody` variants are appended at the end of the enum):

- **Account**: `AccountId` (BLAKE3 of the first credential's public key), a display name, a role (`owner`, `admin`, `member`) and one or more **passkey credentials**: `{credential_id, public_key (P-256, SEC1), rp_id}`.
- **Account-signed ops**: an op may carry, instead of a device's Ed25519 signature, a WebAuthn assertion by an account's passkey: `{account, credential_id, authenticator_data, client_data_json, signature}`. Valid when the ES256 signature checks against the stored key, `client_data_json.type` is `webauthn.get`, its `challenge` is the base64url op hash, the rp id hash matches, and the user-verified flag is set.
- **Authority ops must be account-signed** by an owner (or an admin, where noted): `AddAccount`, `RemoveAccount`, `SetRole` (owner only), `AuthorizeConsole {device}` and `RevokeConsole`. The first owner is the exception: in a network with no accounts yet, the genesis admin device may sign the first `AddAccount` with role `owner`; from then on the device-admin path for authority ops is closed.
- **Authorized consoles**: a device named by an `AuthorizeConsole` may sign everyday admin ops (devices, invites, records, policy, exits, relays, renames) with its own key, as admin devices do today. Each such op carries `actor: AccountId` saying which person asked; validity requires that account to hold the needed role at that point.
- **Device ownership**: `AssignDevice {device, account}`; removing an account removes its devices.
- **Recovery**: when the network is set up, the owner gets a recovery code (an Ed25519 key shown once as words); `AddAccount` and `AuthorizeConsole` signed by it are valid like an owner's. Store nothing about it but its public key.

Existing networks without accounts keep working exactly as now.

## Console data (SQLite, one file)

`rusqlite` with the bundled SQLite, in the Console's state directory, behind one storage trait so Postgres can be added later. Tables: `sessions`, `webauthn_challenges`, `device_codes`, `setup_tokens`, `journal` (every journal entry, kept 30 days), `traffic_minute` (bytes per peer per minute, kept 30 days). Migrations are numbered SQL files run at start.

## Modes

- **Local** (today): `kith ui` on 127.0.0.1 with the one-time link. Passkeys also work here (rp id `localhost`).
- **Public**: `kith-ui --public vpn.example.com` serves HTTPS on 443 with a Let's Encrypt certificate (ACME, renewed automatically), HSTS, `Secure` cookies, Host must be the domain, rp id = the domain. Only passkey login; no token links except the one-time setup link.

## Flows

1. **Setup**: `install.sh --console vpn.example.com` installs kith and kith-ui, runs `kith up`, creates a network if there is none, installs the Console as a service in public mode, and prints a one-time setup link (valid 30 minutes). Opening it: name the network, name yourself, create a passkey. The Console signs the first `AddAccount` (owner); the owner's passkey then signs `AuthorizeConsole` for this device. The page shows the recovery words once.
2. **Login**: passkey only. Session cookie for 12 hours, renewed while active.
3. **Add a person**: owner or admin enters a name and role; gets a one-time link the person opens to create their passkey; the person's `AddAccount` is signed by the inviting admin's passkey when the link is first used (the admin confirms with Touch ID when creating the link, signing an op that names the new credential once registered: implement as a pending account that the inviter's next passkey prompt completes, or sign at creation with the credential filled in at registration — choose the simpler correct one and document it).
4. **Add a device**: the Console shows a 6-character code (10 minutes) and a QR for phones. On a computer: `kith join https://vpn.example.com/j/ABC123` fetches an invite over HTTPS and joins; the device is assigned to the person who made the code. Phones: the existing WireGuard QR flow, assigned to the person.
5. **Everyday admin actions**: as today in the UI; the Console signs and names the actor.
6. **Authority actions** (people, roles, consoles): Touch ID or Face ID prompt for each.

## Interfaces

IPC (append to `kith-ipc`): `Accounts {network}`, `AddAccount`, `RemoveAccount`, `SetRole`, `AssignDevice`, `PrepareAuthority {network, change} -> {op_bytes, challenge}`, `SubmitAuthority {network, op_bytes, assertion}`, `RecoveryKey {network} -> words` (only while unset), and an optional `actor` on every admin request. HTTP (Console): `/setup/<token>` page; `POST /api/setup/begin|finish`; `POST /api/login/begin|finish`; `POST /api/logout`; `GET /api/people`; `POST /api/people` (pending person, returns link); `POST /api/people/{id}/role|remove`; `GET|POST /api/authority/begin|finish`; `POST /api/device-codes` (`{person}` -> `{code, expires_at_ms, qr_svg}`); public `GET /j/{code}` -> invite link for `kith join`. Everything else as in `kith-ui-api.md`, with the session now tied to a person.

## Not now

SSO, custom roles, teams, billing, the menu bar app's redesign, signed requests to individual devices, external databases.
