ALTER TABLE access_requests RENAME TO access_requests_legacy;

DROP INDEX idx_access_requests_status_created;
DROP INDEX idx_access_requests_username;

CREATE TABLE access_requests (
  id TEXT PRIMARY KEY,
  invite_id TEXT REFERENCES invites(id) UNIQUE,
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

INSERT INTO access_requests (
  id, invite_id, username, display_name, credential_hash, request_token_hash,
  status, user_id, created_at, reviewed_at, reviewed_by
)
SELECT
  id, invite_id, username, display_name, credential_hash, request_token_hash,
  status, user_id, created_at, reviewed_at, reviewed_by
FROM access_requests_legacy;

DROP TABLE access_requests_legacy;

CREATE INDEX idx_access_requests_status_created ON access_requests(status, created_at DESC);
CREATE INDEX idx_access_requests_username ON access_requests(username, status);
CREATE UNIQUE INDEX idx_access_requests_open_username ON access_requests(username)
  WHERE status IN ('pending', 'approved');

