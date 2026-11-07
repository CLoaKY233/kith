-- The Console's memory: sessions, ceremonies, one-time links and history. Never authority:
-- the ledger decides who is in the network and who may change it.

CREATE TABLE sessions (
    -- BLAKE3 of the cookie's secret, hex; the secret itself is never stored.
    id TEXT PRIMARY KEY,
    account TEXT NOT NULL,
    created_ms INTEGER NOT NULL,
    last_seen_ms INTEGER NOT NULL,
    expires_ms INTEGER NOT NULL
);
CREATE INDEX sessions_account ON sessions (account);
CREATE INDEX sessions_expires ON sessions (expires_ms);

CREATE TABLE webauthn_challenges (
    -- The challenge as base64url, as the browser echoes it in clientDataJSON.
    challenge TEXT PRIMARY KEY,
    purpose TEXT NOT NULL,
    -- The person the ceremony belongs to, when one is signed in.
    account TEXT,
    -- What finishing the ceremony needs, as JSON.
    payload TEXT NOT NULL,
    expires_ms INTEGER NOT NULL
);
CREATE INDEX webauthn_challenges_expires ON webauthn_challenges (expires_ms);

CREATE TABLE device_codes (
    code TEXT PRIMARY KEY,
    network TEXT NOT NULL,
    -- Whose device it will be.
    account TEXT NOT NULL,
    created_by TEXT NOT NULL,
    created_ms INTEGER NOT NULL,
    expires_ms INTEGER NOT NULL,
    used_ms INTEGER,
    -- The invite handed out for the code, and the device that joined with it.
    invite TEXT,
    node TEXT
);
CREATE INDEX device_codes_expires ON device_codes (expires_ms);

CREATE TABLE setup_tokens (
    -- BLAKE3 of the token, hex.
    token TEXT PRIMARY KEY,
    created_ms INTEGER NOT NULL,
    expires_ms INTEGER NOT NULL,
    used_ms INTEGER
);

-- People invited but not yet in the ledger: their one-time registration link, and the
-- passkey once they made one, until an owner or admin signs their AddAccount.
CREATE TABLE pending_people (
    id TEXT PRIMARY KEY,
    -- BLAKE3 of the link's token, hex.
    token TEXT NOT NULL UNIQUE,
    network TEXT NOT NULL,
    name TEXT NOT NULL,
    role TEXT NOT NULL,
    invited_by TEXT NOT NULL,
    created_ms INTEGER NOT NULL,
    expires_ms INTEGER NOT NULL,
    registered_ms INTEGER,
    credential_id BLOB,
    public_key BLOB,
    rp_id TEXT,
    -- When the inviter's passkey confirmed the invite; owners and admins need it.
    confirmed_ms INTEGER
);

CREATE TABLE journal (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    boot TEXT NOT NULL,
    seq INTEGER NOT NULL,
    at_ms INTEGER NOT NULL,
    kind TEXT NOT NULL,
    network TEXT,
    -- The entry flattened as the event stream sends it.
    entry TEXT NOT NULL,
    -- Not (boot, seq): a restarted kith-ui names the daemon's run anew and reads its
    -- history again.
    UNIQUE (seq, at_ms, kind)
);
CREATE INDEX journal_at ON journal (at_ms);

CREATE TABLE traffic_minute (
    network TEXT NOT NULL,
    node TEXT NOT NULL,
    minute_ms INTEGER NOT NULL,
    rx_bytes INTEGER NOT NULL,
    tx_bytes INTEGER NOT NULL,
    PRIMARY KEY (network, node, minute_ms)
);
CREATE INDEX traffic_minute_at ON traffic_minute (minute_ms);

CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
