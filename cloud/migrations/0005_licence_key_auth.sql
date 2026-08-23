CREATE TABLE licence_keys (
  id TEXT PRIMARY KEY,
  token_hash TEXT NOT NULL UNIQUE,
  key_hint TEXT NOT NULL,
  label TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'revoked')),
  user_id TEXT UNIQUE REFERENCES users(id),
  created_by TEXT NOT NULL REFERENCES users(id),
  created_at TEXT NOT NULL,
  last_activated_at TEXT,
  revoked_at TEXT
);

CREATE INDEX idx_licence_keys_status_created ON licence_keys(status, created_at DESC);
CREATE INDEX idx_licence_keys_user ON licence_keys(user_id);

-- Member sessions issued by the retired password/account-request flow must not
-- remain valid after this migration. Owners retain access so they can create keys.
UPDATE devices SET status = 'revoked'
WHERE user_id IN (SELECT id FROM users WHERE role = 'member');
