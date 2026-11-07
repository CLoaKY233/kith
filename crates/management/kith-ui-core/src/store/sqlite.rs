//! [`Storage`] on SQLite, compiled in: one file in the Console's state directory, or
//! memory for tests and the simulator.

use core::time::Duration;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use rusqlite::{Connection, OptionalExtension as _, Row, params};

use super::{
    ChallengeRow, DeviceCodeRow, JournalRow, PendingPerson, RETENTION_MS, Redeem, SessionRow,
    Storage, StoreError, StoredCredential, TrafficRow,
};

/// The numbered migrations, run in order at open. The database's `user_version` is the
/// number of the last one applied.
const MIGRATIONS: &[(u32, &str)] = &[(1, include_str!("migrations/0001_console.sql"))];

/// How long a writer waits for another one, such as a second `kith-ui`.
const BUSY: Duration = Duration::from_secs(5);

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::new(error.to_string())
    }
}

/// SQLite behind a mutex: every call is short.
#[derive(Debug)]
pub struct SqliteStore {
    db: Mutex<Connection>,
}

/// SQLite keeps signed 64-bit integers; times and byte counts never reach the top bit.
fn int(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn uint(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn opt_uint(value: Option<i64>) -> Option<u64> {
    value.map(uint)
}

impl SqliteStore {
    /// Opens or creates the database at `path` and brings it up to date. On Unix a new
    /// file is readable by its owner only.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let db = Connection::open(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .map_err(|error| StoreError::new(error.to_string()))?;
        }
        db.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
        Self::ready(db)
    }

    /// A database in memory, gone when dropped.
    pub fn memory() -> Result<Self, StoreError> {
        Self::ready(Connection::open_in_memory()?)
    }

    fn ready(mut db: Connection) -> Result<Self, StoreError> {
        db.busy_timeout(BUSY)?;
        db.execute_batch("PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")?;
        migrate(&mut db)?;
        Ok(Self { db: Mutex::new(db) })
    }

    fn db(&self) -> MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Runs every migration newer than the database, each in its own transaction.
fn migrate(db: &mut Connection) -> Result<(), StoreError> {
    let current: u32 = db.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    for (number, sql) in MIGRATIONS {
        if *number <= current {
            continue;
        }
        let transaction = db.transaction()?;
        transaction.execute_batch(sql)?;
        transaction.execute_batch(&format!("PRAGMA user_version = {number}"))?;
        transaction.commit()?;
    }
    Ok(())
}

fn session_row(row: &Row<'_>) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        id: row.get(0)?,
        account: row.get(1)?,
        created_ms: uint(row.get(2)?),
        last_seen_ms: uint(row.get(3)?),
        expires_ms: uint(row.get(4)?),
    })
}

const CODE_COLUMNS: &str =
    "code, network, account, created_by, created_ms, expires_ms, used_ms, invite, node";

fn code_row(row: &Row<'_>) -> rusqlite::Result<DeviceCodeRow> {
    Ok(DeviceCodeRow {
        code: row.get(0)?,
        network: row.get(1)?,
        account: row.get(2)?,
        created_by: row.get(3)?,
        created_ms: uint(row.get(4)?),
        expires_ms: uint(row.get(5)?),
        used_ms: opt_uint(row.get(6)?),
        invite: row.get(7)?,
        node: row.get(8)?,
    })
}

const PENDING_COLUMNS: &str = "id, token, network, name, role, invited_by, created_ms, \
expires_ms, registered_ms, credential_id, public_key, rp_id, confirmed_ms";

fn pending_row(row: &Row<'_>) -> rusqlite::Result<PendingPerson> {
    let credential_id: Option<Vec<u8>> = row.get(9)?;
    let public_key: Option<Vec<u8>> = row.get(10)?;
    let rp_id: Option<String> = row.get(11)?;
    Ok(PendingPerson {
        id: row.get(0)?,
        token: row.get(1)?,
        network: row.get(2)?,
        name: row.get(3)?,
        role: row.get(4)?,
        invited_by: row.get(5)?,
        created_ms: uint(row.get(6)?),
        expires_ms: uint(row.get(7)?),
        registered_ms: opt_uint(row.get(8)?),
        confirmed_ms: opt_uint(row.get(12)?),
        credential: match (credential_id, public_key, rp_id) {
            (Some(credential_id), Some(public_key), Some(rp_id)) => Some(StoredCredential {
                credential_id,
                public_key,
                rp_id,
            }),
            _ => None,
        },
    })
}

fn journal_row(row: &Row<'_>) -> rusqlite::Result<JournalRow> {
    Ok(JournalRow {
        id: row.get(0)?,
        boot: row.get(1)?,
        seq: uint(row.get(2)?),
        at_ms: uint(row.get(3)?),
        kind: row.get(4)?,
        network: row.get(5)?,
        entry: row.get(6)?,
    })
}

fn traffic_row(row: &Row<'_>) -> rusqlite::Result<TrafficRow> {
    Ok(TrafficRow {
        network: row.get(0)?,
        node: row.get(1)?,
        minute_ms: uint(row.get(2)?),
        rx_bytes: uint(row.get(3)?),
        tx_bytes: uint(row.get(4)?),
    })
}

/// Whether a one-time thing that was `used` and lapses at `expires_ms` is still good.
fn state<T>(used: Option<i64>, expires_ms: i64, now_ms: u64, good: T) -> Redeem<T> {
    if used.is_some() {
        Redeem::Used
    } else if uint(expires_ms) <= now_ms {
        Redeem::Expired
    } else {
        Redeem::Ok(good)
    }
}

impl Storage for SqliteStore {
    fn put_session(&self, session: &SessionRow) -> Result<(), StoreError> {
        self.db().execute(
            "INSERT INTO sessions (id, account, created_ms, last_seen_ms, expires_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                session.id,
                session.account,
                int(session.created_ms),
                int(session.last_seen_ms),
                int(session.expires_ms)
            ],
        )?;
        Ok(())
    }

    fn session(&self, id: &str) -> Result<Option<SessionRow>, StoreError> {
        Ok(self
            .db()
            .query_row(
                "SELECT id, account, created_ms, last_seen_ms, expires_ms FROM sessions \
                 WHERE id = ?1",
                params![id],
                session_row,
            )
            .optional()?)
    }

    fn touch_session(
        &self,
        id: &str,
        last_seen_ms: u64,
        expires_ms: u64,
    ) -> Result<(), StoreError> {
        self.db().execute(
            "UPDATE sessions SET last_seen_ms = ?2, expires_ms = ?3 WHERE id = ?1",
            params![id, int(last_seen_ms), int(expires_ms)],
        )?;
        Ok(())
    }

    fn delete_session(&self, id: &str) -> Result<(), StoreError> {
        self.db()
            .execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
        Ok(())
    }

    fn delete_sessions_of(&self, account: &str) -> Result<(), StoreError> {
        self.db()
            .execute("DELETE FROM sessions WHERE account = ?1", params![account])?;
        Ok(())
    }

    fn put_challenge(&self, challenge: &ChallengeRow) -> Result<(), StoreError> {
        self.db().execute(
            "INSERT OR REPLACE INTO webauthn_challenges \
             (challenge, purpose, account, payload, expires_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                challenge.challenge,
                challenge.purpose,
                challenge.account,
                challenge.payload,
                int(challenge.expires_ms)
            ],
        )?;
        Ok(())
    }

    fn take_challenge(&self, challenge: &str) -> Result<Option<ChallengeRow>, StoreError> {
        let mut db = self.db();
        let transaction = db.transaction()?;
        let row = transaction
            .query_row(
                "SELECT challenge, purpose, account, payload, expires_ms \
                 FROM webauthn_challenges WHERE challenge = ?1",
                params![challenge],
                |row| {
                    Ok(ChallengeRow {
                        challenge: row.get(0)?,
                        purpose: row.get(1)?,
                        account: row.get(2)?,
                        payload: row.get(3)?,
                        expires_ms: uint(row.get(4)?),
                    })
                },
            )
            .optional()?;
        transaction.execute(
            "DELETE FROM webauthn_challenges WHERE challenge = ?1",
            params![challenge],
        )?;
        transaction.commit()?;
        Ok(row)
    }

    fn put_device_code(&self, code: &DeviceCodeRow) -> Result<(), StoreError> {
        self.db().execute(
            &format!("INSERT INTO device_codes ({CODE_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"),
            params![
                code.code,
                code.network,
                code.account,
                code.created_by,
                int(code.created_ms),
                int(code.expires_ms),
                code.used_ms.map(int),
                code.invite,
                code.node
            ],
        )?;
        Ok(())
    }

    fn peek_device_code(
        &self,
        code: &str,
        now_ms: u64,
    ) -> Result<Redeem<DeviceCodeRow>, StoreError> {
        let db = self.db();
        let Some(row) = db
            .query_row(
                &format!("SELECT {CODE_COLUMNS} FROM device_codes WHERE code = ?1"),
                params![code],
                code_row,
            )
            .optional()?
        else {
            return Ok(Redeem::Unknown);
        };
        Ok(state(
            row.used_ms.map(int),
            int(row.expires_ms),
            now_ms,
            row,
        ))
    }

    fn redeem_device_code(
        &self,
        code: &str,
        now_ms: u64,
        invite: &str,
    ) -> Result<Redeem<DeviceCodeRow>, StoreError> {
        let mut db = self.db();
        let transaction = db.transaction()?;
        let Some(row) = transaction
            .query_row(
                &format!("SELECT {CODE_COLUMNS} FROM device_codes WHERE code = ?1"),
                params![code],
                code_row,
            )
            .optional()?
        else {
            return Ok(Redeem::Unknown);
        };
        let found = state(
            row.used_ms.map(int),
            int(row.expires_ms),
            now_ms,
            row.clone(),
        );
        if let Redeem::Ok(_) = found {
            transaction.execute(
                "UPDATE device_codes SET used_ms = ?2, invite = ?3 WHERE code = ?1",
                params![code, int(now_ms), invite],
            )?;
            transaction.commit()?;
            return Ok(Redeem::Ok(DeviceCodeRow {
                used_ms: Some(now_ms),
                invite: Some(invite.to_owned()),
                ..row
            }));
        }
        Ok(found)
    }

    fn unassigned_codes(
        &self,
        network: &str,
        since_ms: u64,
    ) -> Result<Vec<DeviceCodeRow>, StoreError> {
        let db = self.db();
        let mut statement = db.prepare(&format!(
            "SELECT {CODE_COLUMNS} FROM device_codes WHERE network = ?1 AND used_ms >= ?2 \
             AND invite IS NOT NULL AND node IS NULL ORDER BY used_ms"
        ))?;
        let rows = statement
            .query_map(params![network, int(since_ms)], code_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn set_device_code_node(&self, code: &str, node: &str) -> Result<(), StoreError> {
        self.db().execute(
            "UPDATE device_codes SET node = ?2 WHERE code = ?1",
            params![code, node],
        )?;
        Ok(())
    }

    fn put_setup_token(
        &self,
        token: &str,
        created_ms: u64,
        expires_ms: u64,
    ) -> Result<(), StoreError> {
        let mut db = self.db();
        let transaction = db.transaction()?;
        transaction.execute(
            "UPDATE setup_tokens SET used_ms = ?1 WHERE used_ms IS NULL",
            params![int(created_ms)],
        )?;
        transaction.execute(
            "INSERT INTO setup_tokens (token, created_ms, expires_ms) VALUES (?1, ?2, ?3)",
            params![token, int(created_ms), int(expires_ms)],
        )?;
        transaction.commit()?;
        Ok(())
    }

    fn check_setup_token(&self, token: &str, now_ms: u64) -> Result<Redeem<u64>, StoreError> {
        let row: Option<(Option<i64>, i64)> = self
            .db()
            .query_row(
                "SELECT used_ms, expires_ms FROM setup_tokens WHERE token = ?1",
                params![token],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        Ok(match row {
            None => Redeem::Unknown,
            Some((used, expires_ms)) => state(used, expires_ms, now_ms, uint(expires_ms)),
        })
    }

    fn use_setup_token(&self, token: &str, now_ms: u64) -> Result<Redeem<u64>, StoreError> {
        let mut db = self.db();
        let transaction = db.transaction()?;
        let row: Option<(Option<i64>, i64)> = transaction
            .query_row(
                "SELECT used_ms, expires_ms FROM setup_tokens WHERE token = ?1",
                params![token],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((used, expires_ms)) = row else {
            return Ok(Redeem::Unknown);
        };
        let found = state(used, expires_ms, now_ms, uint(expires_ms));
        if matches!(found, Redeem::Ok(_) | Redeem::Expired) {
            transaction.execute(
                "UPDATE setup_tokens SET used_ms = ?2 WHERE token = ?1",
                params![token, int(now_ms)],
            )?;
            transaction.commit()?;
        }
        Ok(found)
    }

    fn put_pending(&self, person: &PendingPerson) -> Result<(), StoreError> {
        let credential = person.credential.as_ref();
        self.db().execute(
            &format!(
                "INSERT INTO pending_people ({PENDING_COLUMNS}) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"
            ),
            params![
                person.id,
                person.token,
                person.network,
                person.name,
                person.role,
                person.invited_by,
                int(person.created_ms),
                int(person.expires_ms),
                person.registered_ms.map(int),
                credential.map(|c| c.credential_id.clone()),
                credential.map(|c| c.public_key.clone()),
                credential.map(|c| c.rp_id.clone()),
                person.confirmed_ms.map(int),
            ],
        )?;
        Ok(())
    }

    fn pending(&self, network: &str) -> Result<Vec<PendingPerson>, StoreError> {
        let db = self.db();
        let mut statement = db.prepare(&format!(
            "SELECT {PENDING_COLUMNS} FROM pending_people WHERE network = ?1 ORDER BY created_ms"
        ))?;
        let rows = statement
            .query_map(params![network], pending_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn pending_by_token(&self, token: &str) -> Result<Option<PendingPerson>, StoreError> {
        Ok(self
            .db()
            .query_row(
                &format!("SELECT {PENDING_COLUMNS} FROM pending_people WHERE token = ?1"),
                params![token],
                pending_row,
            )
            .optional()?)
    }

    fn register_pending(
        &self,
        id: &str,
        credential: &StoredCredential,
        now_ms: u64,
    ) -> Result<Redeem<()>, StoreError> {
        let mut db = self.db();
        let transaction = db.transaction()?;
        let row: Option<(Option<i64>, i64)> = transaction
            .query_row(
                "SELECT registered_ms, expires_ms FROM pending_people WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((registered, expires_ms)) = row else {
            return Ok(Redeem::Unknown);
        };
        let found = state(registered, expires_ms, now_ms, ());
        if found == Redeem::Ok(()) {
            transaction.execute(
                "UPDATE pending_people SET registered_ms = ?2, credential_id = ?3, \
                 public_key = ?4, rp_id = ?5 WHERE id = ?1",
                params![
                    id,
                    int(now_ms),
                    credential.credential_id,
                    credential.public_key,
                    credential.rp_id
                ],
            )?;
            transaction.commit()?;
        }
        Ok(found)
    }

    fn confirm_pending(&self, id: &str, now_ms: u64) -> Result<bool, StoreError> {
        Ok(self.db().execute(
            "UPDATE pending_people SET confirmed_ms = ?2 WHERE id = ?1",
            params![id, int(now_ms)],
        )? > 0)
    }

    fn delete_pending(&self, id: &str) -> Result<bool, StoreError> {
        Ok(self
            .db()
            .execute("DELETE FROM pending_people WHERE id = ?1", params![id])?
            > 0)
    }

    fn add_journal(&self, entries: &[JournalRow]) -> Result<(), StoreError> {
        let mut db = self.db();
        let transaction = db.transaction()?;
        {
            let mut insert = transaction.prepare(
                "INSERT OR IGNORE INTO journal (boot, seq, at_ms, kind, network, entry) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for entry in entries {
                insert.execute(params![
                    entry.boot,
                    int(entry.seq),
                    int(entry.at_ms),
                    entry.kind,
                    entry.network,
                    entry.entry
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    fn journal(
        &self,
        before: Option<i64>,
        network: Option<&str>,
        limit: u32,
    ) -> Result<Vec<JournalRow>, StoreError> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT id, boot, seq, at_ms, kind, network, entry FROM journal \
             WHERE id < ?1 AND (?2 IS NULL OR network = ?2) ORDER BY id DESC LIMIT ?3",
        )?;
        let rows = statement
            .query_map(
                params![before.unwrap_or(i64::MAX), network, limit],
                journal_row,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn add_traffic(&self, rows: &[TrafficRow]) -> Result<(), StoreError> {
        let mut db = self.db();
        let transaction = db.transaction()?;
        {
            let mut upsert = transaction.prepare(
                "INSERT INTO traffic_minute (network, node, minute_ms, rx_bytes, tx_bytes) \
                 VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT (network, node, minute_ms) DO UPDATE \
                 SET rx_bytes = rx_bytes + excluded.rx_bytes, \
                 tx_bytes = tx_bytes + excluded.tx_bytes",
            )?;
            for row in rows {
                upsert.execute(params![
                    row.network,
                    row.node,
                    int(row.minute_ms),
                    int(row.rx_bytes),
                    int(row.tx_bytes)
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    fn traffic(
        &self,
        network: Option<&str>,
        node: Option<&str>,
        since_ms: u64,
        until_ms: u64,
    ) -> Result<Vec<TrafficRow>, StoreError> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT network, node, minute_ms, rx_bytes, tx_bytes FROM traffic_minute \
             WHERE minute_ms >= ?1 AND minute_ms < ?2 AND (?3 IS NULL OR network = ?3) \
             AND (?4 IS NULL OR node = ?4) ORDER BY network, node, minute_ms",
        )?;
        let rows = statement
            .query_map(
                params![int(since_ms), int(until_ms), network, node],
                traffic_row,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .db()
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?)
    }

    fn set_setting(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.db().execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) \
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    fn prune(&self, now_ms: u64) -> Result<(), StoreError> {
        let history = int(now_ms.saturating_sub(RETENTION_MS));
        let now = int(now_ms);
        let mut db = self.db();
        let transaction = db.transaction()?;
        transaction.execute("DELETE FROM journal WHERE at_ms < ?1", params![history])?;
        transaction.execute(
            "DELETE FROM traffic_minute WHERE minute_ms < ?1",
            params![history],
        )?;
        transaction.execute("DELETE FROM sessions WHERE expires_ms <= ?1", params![now])?;
        transaction.execute(
            "DELETE FROM webauthn_challenges WHERE expires_ms <= ?1",
            params![now],
        )?;
        // Used codes stay a day, so a device that joins late can still be assigned.
        transaction.execute(
            "DELETE FROM device_codes WHERE expires_ms <= ?1",
            params![int(now_ms.saturating_sub(24 * 60 * 60 * 1000))],
        )?;
        transaction.execute(
            "DELETE FROM setup_tokens WHERE expires_ms <= ?1",
            params![history],
        )?;
        transaction.execute(
            "DELETE FROM pending_people WHERE registered_ms IS NULL AND expires_ms <= ?1",
            params![now],
        )?;
        transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
#[expect(
    clippy::panic_in_result_fn,
    reason = "assertions fail the test; ? keeps the setup short"
)]
mod tests {
    use super::*;

    const DAY: u64 = 24 * 60 * 60 * 1000;

    fn entry(seq: u64, at_ms: u64) -> JournalRow {
        JournalRow {
            id: 0,
            boot: format!("b{at_ms}"),
            seq,
            at_ms,
            kind: "warning".to_owned(),
            network: None,
            entry: "{}".to_owned(),
        }
    }

    fn minute(minute_ms: u64, rx_bytes: u64) -> TrafficRow {
        TrafficRow {
            network: "n".to_owned(),
            node: "d".to_owned(),
            minute_ms,
            rx_bytes,
            tx_bytes: 1,
        }
    }

    #[test]
    fn history_is_kept_thirty_days() -> Result<(), StoreError> {
        let store = SqliteStore::memory()?;
        let now = 100 * DAY;
        store.add_journal(&[entry(0, now - 31 * DAY), entry(1, now - 29 * DAY)])?;
        store.add_journal(&[entry(1, now - 29 * DAY)])?;
        store.add_traffic(&[minute(now - 31 * DAY, 5), minute(now - DAY, 5)])?;
        store.add_traffic(&[minute(now - DAY, 7)])?;
        store.prune(now)?;
        let kept: Vec<u64> = store
            .journal(None, None, 10)?
            .iter()
            .map(|row| row.seq)
            .collect();
        assert_eq!(kept, [1], "the old entry went, the duplicate was skipped");
        let minutes = store.traffic(None, None, 0, u64::MAX)?;
        assert_eq!(minutes.len(), 1);
        assert_eq!(minutes.first().map(|row| row.rx_bytes), Some(12));
        Ok(())
    }

    #[test]
    fn migrations_run_once() -> Result<(), StoreError> {
        let mut db = Connection::open_in_memory()?;
        migrate(&mut db)?;
        migrate(&mut db)?;
        let version: u32 = db.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        assert_eq!(Some(version), MIGRATIONS.last().map(|(number, _)| *number));
        Ok(())
    }

    #[test]
    fn a_setup_token_is_used_once_and_lapses() -> Result<(), StoreError> {
        let store = SqliteStore::memory()?;
        store.put_setup_token("a", 0, 10)?;
        store.put_setup_token("b", 0, 10)?;
        assert_eq!(store.use_setup_token("a", 1)?, Redeem::Used, "retired");
        assert_eq!(store.check_setup_token("b", 1)?, Redeem::Ok(10));
        assert_eq!(store.use_setup_token("b", 1)?, Redeem::Ok(10));
        assert_eq!(store.use_setup_token("b", 1)?, Redeem::Used);
        store.put_setup_token("c", 0, 10)?;
        assert_eq!(store.use_setup_token("c", 10)?, Redeem::Expired);
        assert_eq!(store.use_setup_token("z", 1)?, Redeem::Unknown);
        Ok(())
    }
}
