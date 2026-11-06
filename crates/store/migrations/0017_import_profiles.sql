-- M10 / A4: stored CSV import profiles.
--
-- CAMT and MT940 describe themselves; a CSV export does not. The delimiter, the
-- column order, the decimal comma, and the date layout are whatever that bank
-- decided, and reading amounts through a guess is how a locale turns 1.234 into
-- one euro and change. So the mapping is data a person states, kept in the
-- archive, and re-read every time a file is imported.
--
-- The text is TOML because a profile is edited by hand: comments survive, and a
-- reader can see what a bank's export looks like without a JSON decoder. It is
-- parsed and validated before it is stored, so a row here is always readable —
-- and stored verbatim, so what `profile show` prints is what the import runs.
--
-- `bank_account.import_profile`, which 0014 already has, names the profile an
-- account uses when nothing more specific is given. It is not a foreign key: the
-- column predates the table, and a profile may be removed while an account still
-- names it. Reading an account's profile therefore has to cope with a dangling
-- name, and does: it is reported, not silently ignored.
--
-- `sha256` is the digest of `toml`, so a statement read months ago can be tied to
-- the mapping that was current when it was read, even after the mapping changed.

CREATE TABLE import_profile (
    name       TEXT PRIMARY KEY,
    toml       TEXT NOT NULL,
    sha256     TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
