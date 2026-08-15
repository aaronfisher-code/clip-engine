ALTER TABLE users ADD COLUMN username TEXT COLLATE NOCASE;
ALTER TABLE users ADD COLUMN password_hash TEXT;
ALTER TABLE users ADD COLUMN password_salt TEXT;
ALTER TABLE users ADD COLUMN password_iterations INTEGER;

UPDATE users SET email = id || '@no-email.invalid';

CREATE UNIQUE INDEX idx_users_username ON users(username) WHERE username IS NOT NULL;

ALTER TABLE invites ADD COLUMN username TEXT COLLATE NOCASE;
ALTER TABLE invites ADD COLUMN purpose TEXT NOT NULL DEFAULT 'enroll'
  CHECK (purpose IN ('enroll', 'password_reset'));
ALTER TABLE invites ADD COLUMN target_user_id TEXT REFERENCES users(id);

UPDATE invites SET email = id || '@no-email.invalid';

CREATE INDEX idx_invites_username ON invites(username, redeemed_at, expires_at);

CREATE TABLE auth_attempts (
  bucket TEXT PRIMARY KEY,
  failures INTEGER NOT NULL,
  window_started_at TEXT NOT NULL,
  blocked_until TEXT,
  updated_at TEXT NOT NULL
);
