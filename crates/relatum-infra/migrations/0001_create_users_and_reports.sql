-- Initial schema for the relational storage adapters.
--
-- Mirrors the domain aggregates `User` (crates/relatum-domain/src/models/users.rs)
-- and `Report` (crates/relatum-domain/src/models/report.rs). Users are provisioned
-- from the LDAP directory sync, so a user carries the directory `marker` and the
-- locally-assigned `department` (NULL until an admin assigns one, leaving the user
-- inert). Reports flow into their author's `department` queue rather than to a
-- chosen reviewer. The enum variants are flattened into discriminant columns plus
-- their payloads:
--
--   users.marker        'instructor' | 'trainee' | 'regular' (directory group marker)
--   users.department    the assigned department, or NULL when the user is inert
--
--   reports.week        the ISO week the report covers, as `YYYY-Www` text (e.g.
--                       `2026-W24`); `UNIQUE (author, week)` enforces at most one
--                       report per trainee per week
--   reports.status      'draft'      -> status_at NULL,        signed_by/reject_reason NULL
--                       'submitted'  -> status_at = timestamp, signed_by/reject_reason NULL
--                       'signed'     -> status_at = timestamp, signed_by = the signer
--                       'rejected'   -> status_at = timestamp, reject_reason set
--
-- Timestamps are stored as RFC 3339 text because the domain models time with
-- `jiff::Timestamp`, which sqlx does not map natively; the adapter formats and
-- parses them losslessly.

CREATE TABLE users (
    id         TEXT PRIMARY KEY,
    username   TEXT NOT NULL,
    marker     TEXT NOT NULL CHECK (marker IN ('instructor', 'trainee', 'regular')),
    department TEXT
);

CREATE TABLE reports (
    id            TEXT PRIMARY KEY,
    author        TEXT NOT NULL,
    department    TEXT NOT NULL,
    week          TEXT NOT NULL,
    content       TEXT NOT NULL,
    status        TEXT NOT NULL CHECK (status IN ('draft', 'submitted', 'signed', 'rejected')),
    status_at     TEXT,
    signed_by     TEXT,
    reject_reason TEXT,
    -- At most one report per trainee per ISO week.
    UNIQUE (author, week)
);

-- `list_by_author` filters on `author` (served by the leading column of the
-- `UNIQUE (author, week)` index); `list_by_department` (a signer's queue) on
-- `department`.
CREATE INDEX reports_department_idx ON reports (department);
