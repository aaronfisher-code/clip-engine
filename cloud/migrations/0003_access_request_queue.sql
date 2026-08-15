ALTER TABLE users ADD COLUMN password_scheme TEXT NOT NULL DEFAULT 'server-pbkdf2-v1'
  CHECK (password_scheme IN ('server-pbkdf2-v1', 'client-pbkdf2-v1'));

CREATE TABLE access_requests (
  id TEXT PRIMARY KEY,
  invite_id TEXT NOT NULL REFERENCES invites(id) UNIQUE,
  username TEXT NOT NULL COLLATE NOCASE,
  display_name TEXT NOT NULL,
  credential_hash TEXT NOT NULL,
  request_token_hash TEXT NOT NULL UNIQUE,
  status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'approved', 'denied')),
  user_id TEXT REFERENCES users(id),
  created_at TEXT NOT NULL,
  reviewed_at TEXT,
  reviewed_by TEXT REFERENCES users(id)
);

CREATE INDEX idx_access_requests_status_created ON access_requests(status, created_at DESC);
CREATE INDEX idx_access_requests_username ON access_requests(username, status);

