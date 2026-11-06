-- M10 / A1: the ledger.
--
-- Marked irreversible, unlike the migrations before it: dropping these tables
-- would delete postings, and a posting is evidence for a filed VAT return. A
-- schema that can be rolled back only while nobody depends on it is a schema
-- that invites a rollback after somebody does.
--
-- Amounts are micro-units in the entry currency; `base_amount_micros` is the
-- same value in the reporting currency, equal to `amount_micros` until a foreign
-- currency posting exists. `fx_rate_micros` is reporting currency per unit and
-- stays null for reporting-currency entries, so "no rate needed" and "rate of 1"
-- remain distinguishable.

CREATE TABLE account (
    code        TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    kind        TEXT NOT NULL CHECK (kind IN ('asset', 'liability', 'equity', 'revenue', 'expense')),
    -- Optional mapping into the Referentie Grootboek Schema. Null means the
    -- account is outside the reference chart; it never means "not yet filled in".
    rgs_code    TEXT,
    parent_code TEXT REFERENCES account(code),
    -- AR/AP control accounts: their balance is reconciled against the open items.
    is_control  INTEGER NOT NULL DEFAULT 0 CHECK (is_control IN (0, 1)),
    -- Null means the reporting currency of the books.
    currency    TEXT,
    archived    INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
    updated_at  TEXT NOT NULL
);

CREATE INDEX account_kind ON account (kind, code);

-- The seven journals are fixed: numbering, reporting, and the audit trail are
-- per journal, and a user-invented journal would need its own numbering policy.
-- The names are the conventional Dutch labels, which is what an accountant reads
-- on an export; the interface localises by code.
CREATE TABLE journal (
    code TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL
);

INSERT INTO journal (code, name, kind) VALUES
    ('VK', 'Verkoop', 'sales'),
    ('IK', 'Inkoop', 'purchase'),
    ('BNK', 'Bank', 'bank'),
    ('KAS', 'Kas', 'cash'),
    ('MEM', 'Memoriaal', 'memorial'),
    ('OPEN', 'Beginbalans', 'opening'),
    ('BTW', 'Btw', 'vat');

CREATE TRIGGER journal_no_update BEFORE UPDATE ON journal
BEGIN
    SELECT RAISE(ABORT, 'journals are fixed');
END;

CREATE TRIGGER journal_no_delete BEFORE DELETE ON journal
BEGIN
    SELECT RAISE(ABORT, 'journals are fixed');
END;

CREATE TABLE fiscal_period (
    id        TEXT PRIMARY KEY,
    seller_id TEXT NOT NULL REFERENCES seller(id),
    year      INTEGER NOT NULL,
    period_no INTEGER NOT NULL CHECK (period_no BETWEEN 1 AND 13),
    starts_on TEXT NOT NULL,
    ends_on   TEXT NOT NULL,
    status    TEXT NOT NULL CHECK (status IN ('open', 'locked')),
    UNIQUE (seller_id, year, period_no)
);

CREATE INDEX fiscal_period_seller_year ON fiscal_period (seller_id, year, period_no);

-- Locking, reopening, and closing are facts about a period, not edits to it.
CREATE TABLE period_event (
    id        TEXT PRIMARY KEY,
    period_id TEXT NOT NULL REFERENCES fiscal_period(id),
    kind      TEXT NOT NULL CHECK (kind IN ('locked', 'reopened', 'closed')),
    reason    TEXT,
    at        TEXT NOT NULL,
    actor     TEXT NOT NULL
);

CREATE INDEX period_event_period ON period_event (period_id, at);

CREATE TABLE journal_entry (
    id                 TEXT PRIMARY KEY,
    journal_code       TEXT NOT NULL REFERENCES journal(code),
    period_id          TEXT NOT NULL REFERENCES fiscal_period(id),
    -- Rendered per journal and year, e.g. 2026-0001.
    number             TEXT NOT NULL,
    entry_date_local   TEXT NOT NULL,
    description        TEXT NOT NULL,
    source_kind        TEXT NOT NULL CHECK (source_kind IN
                         ('outbound_document', 'inbound_document', 'manual', 'opening', 'bank')),
    source_id          TEXT,
    reverses_entry_id  TEXT REFERENCES journal_entry(id),
    posting_profile_id TEXT NOT NULL,
    rule_version       TEXT NOT NULL,
    currency           TEXT NOT NULL CHECK (length(currency) = 3),
    created_at         TEXT NOT NULL,
    created_by         TEXT NOT NULL,
    content_hash       TEXT NOT NULL,
    UNIQUE (journal_code, number)
);

-- Posting is idempotent per source fact: one document, one entry. Manual and
-- opening entries carry no source id, and several of those are allowed, so the
-- index is partial rather than a table constraint.
CREATE UNIQUE INDEX journal_entry_source ON journal_entry (source_kind, source_id)
    WHERE source_id IS NOT NULL;

CREATE INDEX journal_entry_period ON journal_entry (period_id, journal_code, number);
CREATE INDEX journal_entry_reverses ON journal_entry (reverses_entry_id)
    WHERE reverses_entry_id IS NOT NULL;

CREATE TABLE journal_line (
    id                 TEXT PRIMARY KEY,
    entry_id           TEXT NOT NULL REFERENCES journal_entry(id),
    line_no            INTEGER NOT NULL CHECK (line_no >= 1),
    account_code       TEXT NOT NULL REFERENCES account(code),
    side               TEXT NOT NULL CHECK (side IN ('debit', 'credit')),
    amount_micros      INTEGER NOT NULL CHECK (amount_micros > 0),
    currency           TEXT NOT NULL CHECK (length(currency) = 3),
    fx_rate_micros     INTEGER CHECK (fx_rate_micros IS NULL OR fx_rate_micros > 0),
    base_amount_micros INTEGER NOT NULL CHECK (base_amount_micros >= 0),
    party_id           TEXT,
    tax_code_id        TEXT,
    description        TEXT,
    UNIQUE (entry_id, line_no)
);

-- One index for account balances (the trial balance walks it by account) and one
-- for reading a single entry back in line order.
CREATE INDEX journal_line_account ON journal_line (account_code, entry_id);
CREATE INDEX journal_line_entry ON journal_line (entry_id, line_no);

CREATE TRIGGER journal_entry_no_update BEFORE UPDATE ON journal_entry
BEGIN
    SELECT RAISE(ABORT, 'journal entries are append-only');
END;

CREATE TRIGGER journal_entry_no_delete BEFORE DELETE ON journal_entry
BEGIN
    SELECT RAISE(ABORT, 'journal entries are append-only');
END;

CREATE TRIGGER journal_line_no_update BEFORE UPDATE ON journal_line
BEGIN
    SELECT RAISE(ABORT, 'journal lines are append-only');
END;

CREATE TRIGGER journal_line_no_delete BEFORE DELETE ON journal_line
BEGIN
    SELECT RAISE(ABORT, 'journal lines are append-only');
END;

CREATE TRIGGER period_event_no_update BEFORE UPDATE ON period_event
BEGIN
    SELECT RAISE(ABORT, 'period events are append-only');
END;

CREATE TRIGGER period_event_no_delete BEFORE DELETE ON period_event
BEGIN
    SELECT RAISE(ABORT, 'period events are append-only');
END;

-- A posting into a locked period is refused by the database as well as by the
-- application. A missing period counts as locked: an entry must name a period
-- that exists and is open, and the foreign key alone would let a race through.
CREATE TRIGGER journal_entry_open_period BEFORE INSERT ON journal_entry
WHEN COALESCE((SELECT status FROM fiscal_period WHERE id = NEW.period_id), 'locked') <> 'open'
BEGIN
    SELECT RAISE(ABORT, 'the fiscal period is locked');
END;
