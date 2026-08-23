# Security model

## Trust boundaries

- Public: share pages, thumbnails, and published MP4 files. Their URLs are intended to
  be shared without login.
- Authenticated: team-library metadata, upload creation/completion, and clip retention.
- Owner-only: licence generation, licence listing, and licence revocation or restoration.
- Server-only: the parent R2 access-key ID and secret, token pepper, and owner bootstrap
  token.
- Device-only: the opaque device token in the OS credential vault and local recordings.
  The desktop is a native process (egui + libmpv). There is no embedded webview, so
  renderer CSP does not apply; cloud calls still leave the device only as HTTPS to the
  Worker and scoped R2 uploads. Desktop updates are downloaded from public GitHub
  Release assets over HTTPS and installed by the NSIS/AppImage/deb package already
  attached to that release.

## Licence authentication

Member licences contain 192 random bits and use a recognizable `CE-` prefix. The Worker
stores only each key's peppered SHA-256 hash and a short non-secret hint; plaintext keys
are returned once at creation and cannot be recovered from D1. Activation sends the key,
the member's chosen public display name, and a device label over TLS. The first activation
creates the member identity; later activations of the same active key add device sessions
without changing that identity. Attempts are throttled by source IP.

Successful activation returns a separate 256-bit opaque device token. The desktop stores
that token in Windows Credential Manager or the Linux Secret Service keyring, while D1
stores only its peppered hash. Licence keys are therefore not sent on routine API calls.
The owner uses the high-entropy `BOOTSTRAP_TOKEN` through the same activation screen; it
is compared against the server secret and is never stored in D1.

Migration `0005_licence_key_auth.sql` revokes device sessions belonging to the retired
username/password member flow. Owners keep their existing device sessions so they can
issue replacement licences. Legacy login, registration, invitation, access-request, and
password-reset endpoints are no longer routed.

## Upload authorization

The Worker authenticates a hashed device token, generates unpredictable object keys,
records the expected byte counts in D1, and locally signs temporary credentials with the
server-only parent R2 secret. The JWT restricts access to those exact keys. Completion
succeeds only when R2 HEAD metadata matches both expected sizes. The credential expires
after one hour and cannot access unrelated objects or another bucket.

## Revocation

Revoking a licence marks the linked user and all devices revoked in a single D1 batch.
Restoring the licence does not restore those old sessions; the member enters the key
again. New API requests fail immediately. Temporary R2 credentials already issued to that device may
remain usable for their remaining one-hour lifetime, but only for the two keys assigned
to that upload. Revoke the parent R2 token if those outstanding credentials must be
invalidated immediately during an incident.

## Retention

D1 is authoritative for link availability. The Worker returns HTTP 410 at the exact
expiry and its hourly task marks records expired. The R2 rule removes bytes after 30
days from their latest write, typically within 24 hours. Extending retention rewrites
both objects and advances the D1 time together.

## Incident checklist

1. Revoke the affected licence in **Manage access**.
2. Rotate the parent R2 token and update `R2_PARENT_SECRET_ACCESS_KEY` plus
   `R2_PARENT_ACCESS_KEY_ID` for suspected storage compromise.
3. Rotate `TOKEN_PEPPER` only with a planned forced logout of every device.
4. Delete affected clips through the app or R2, then inspect Worker audit logs/D1.
5. Revoke the GitHub Release and rotate any installer-signing certificates if a desktop
   build is compromised.
