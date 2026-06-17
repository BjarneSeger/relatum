-- Per-user signature images.
--
-- Mirrors the domain value object `Signature`
-- (crates/relatum-domain/src/models/signature.rs). A signature is a per-user asset
-- — at most one per user, set once and replaced in place — that a future PDF export
-- stamps onto reports: the trainee's mark next to their authored report, the
-- signer's next to their sign-off.
--
-- It lives in its own table rather than on `users` on purpose: the LDAP directory
-- sync rewrites the `users` row from directory fields, and a signature column there
-- would be clobbered on every resync. Keying on `user_id` here keeps the
-- directory-owned row untouched.
--
--   signatures.image       the raw image bytes — the first BYTEA column in the
--                          schema (every other column is TEXT)
--   signatures.format      the image format discriminant ('png' today)
--   signatures.updated_at  when the signature was last set, as RFC 3339 text, the
--                          same way the rest of the schema stores `jiff::Timestamp`

CREATE TABLE signatures (
    user_id    TEXT PRIMARY KEY,
    format     TEXT NOT NULL CHECK (format IN ('png')),
    image      BYTEA NOT NULL,
    updated_at TEXT NOT NULL
);
