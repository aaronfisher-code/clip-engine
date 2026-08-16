# Security model

## Trust boundaries

- Public: share pages, thumbnails, and published MP4 files. Their URLs are intended to
  be shared without login.
- Authenticated: team-library metadata, upload creation/completion, and clip retention.
- Owner-only: account approval, password-reset tokens, user listing, and account revocation.
- Server-only: the parent R2 access-key ID and secret, token pepper, and owner bootstrap
  token.
- Device-only: the opaque device token in the OS credential vault and local recordings.
  The desktop is a native process (egui + libmpv). There is no embedded webview, so
  renderer CSP does not apply; cloud calls still leave the device only as HTTPS to the
  Worker and scoped R2 uploads. Desktop updates are downloaded from public GitHub
  Release assets over HTTPS and installed by the NSIS/AppImage/deb package already
  attached to that release.

## Account authentication

Accounts use a non-personal username and password; Clip Engine does not request an email
address. The desktop derives a 256-bit credential with PBKDF2-HMAC-SHA256 at 600,000
iterations using a username-scoped salt. Only that derived credential crosses TLS for
member accounts. D1 stores a peppered SHA-256 hash of it, so neither D1 nor the Worker
receives or stores a member's human password. The `admin` login additionally sends the
high-entropy owner token over TLS so the Worker can compare it with its
`BOOTSTRAP_TOKEN` secret; the token is never stored in D1. The migration replaces legacy
email values with generated `.invalid` compatibility identifiers. Login errors do not
reveal whether a username exists, and failed attempts are throttled independently by
username and source IP.

Legacy server-derived credentials are not accepted by the lightweight login path.
Owners migrate by signing in as `admin` with the owner token, and members migrate
through an owner-issued reset token; existing opaque device sessions remain valid until
replaced or revoked.

Account creation creates only a pending request. The public endpoint is limited to five
successful requests per source IP per 24 hours, reserves the `admin` username, and grants
no authenticated or upload access. The owner must independently approve a request before
an active user is created. Pending apps authenticate status checks with a separate opaque
token stored in the OS credential vault and do not poll automatically.

Password-reset tokens contain 256 bits of randomness, are stored only as peppered hashes,
expire after one day, and can be redeemed once. A reset token and matching username must
be validated before the UI accepts a new password. Redeeming one revokes the user's older
device sessions. Owner recovery through the server-held bootstrap token likewise revokes
all older owner sessions.

## Upload authorization

The Worker authenticates a hashed device token, generates unpredictable object keys,
records the expected byte counts in D1, and locally signs temporary credentials with the
server-only parent R2 secret. The JWT restricts access to those exact keys. Completion
succeeds only when R2 HEAD metadata matches both expected sizes. The credential expires
after one hour and cannot access unrelated objects or another bucket.

## Revocation

Revoking a user marks the user and all devices revoked in a single D1 batch. Restoring
the account does not restore those old sessions; the member signs in again. New API
requests fail immediately. Temporary R2 credentials already issued to that device may
remain usable for their remaining one-hour lifetime, but only for the two keys assigned
to that upload. Revoke the parent R2 token if those outstanding credentials must be
invalidated immediately during an incident.

## Retention

D1 is authoritative for link availability. The Worker returns HTTP 410 at the exact
expiry and its hourly task marks records expired. The R2 rule removes bytes after 30
days from their latest write, typically within 24 hours. Extending retention rewrites
both objects and advances the D1 time together.

## Incident checklist

1. Revoke the affected member in **Manage access**.
2. Rotate the parent R2 token and update `R2_PARENT_SECRET_ACCESS_KEY` plus
   `R2_PARENT_ACCESS_KEY_ID` for suspected storage compromise.
3. Rotate `TOKEN_PEPPER` only with a planned forced logout of every device.
4. Delete affected clips through the app or R2, then inspect Worker audit logs/D1.
5. Revoke the GitHub Release and rotate any installer-signing certificates if a desktop
   build is compromised.
