-- M10 / A4: bank statements, their transactions, and how they settle open items.
--
-- A statement is stored with the hash of the bytes it came from, so importing the
-- same file twice is a no-op rather than a second copy of the same money. A
-- transaction carries a fingerprint of what identifies it — date, amount,
-- counterparty, remittance — because banks send overlapping ranges: the same
-- payment appears in last month's file and this month's, and it is one payment.
--
-- Allocations are the other direction from postings: an open item and a bank
-- transaction, with the amount that settles between them. The sum below an
-- allocation can never exceed what is open, and an allocation is never edited —
-- unmatching appends a reversal, so the record shows that a human changed their
-- mind rather than pretending they never matched it.

CREATE TABLE bank_account (
    id                  TEXT PRIMARY KEY,
    seller_id           TEXT NOT NULL REFERENCES seller(id),
    name                TEXT NOT NULL,
    iban                TEXT,
    currency            TEXT NOT NULL CHECK (length(currency) = 3),
    -- The ledger account the bank balance lives on, e.g. 1100.
    ledger_account_code TEXT NOT NULL REFERENCES account(code),
    import_profile      TEXT
);

CREATE TABLE bank_statement (
    id                     TEXT PRIMARY KEY,
    bank_account_id        TEXT NOT NULL REFERENCES bank_account(id),
    format                 TEXT NOT NULL CHECK (format IN ('camt053', 'mt940', 'csv')),
    source_name            TEXT,
    period_start           TEXT,
    period_end             TEXT,
    opening_balance_micros INTEGER,
    closing_balance_micros INTEGER,
    payload_hash           TEXT NOT NULL,
    imported_at            TEXT NOT NULL,
    UNIQUE (bank_account_id, payload_hash)
);

CREATE TABLE bank_transaction (
    id                 TEXT PRIMARY KEY,
    statement_id       TEXT NOT NULL REFERENCES bank_statement(id),
    booking_date_local TEXT NOT NULL,
    value_date_local   TEXT,
    -- Signed: a debit on the account is negative.
    amount_micros      INTEGER NOT NULL CHECK (amount_micros <> 0),
    currency           TEXT NOT NULL CHECK (length(currency) = 3),
    counterparty_name  TEXT,
    counterparty_iban  TEXT,
    remittance_info    TEXT,
    external_ref       TEXT,
    status             TEXT NOT NULL CHECK (status IN ('new', 'matched', 'partially_matched', 'posted', 'ignored')),
    ignored_reason     TEXT,
    fingerprint        TEXT NOT NULL
);

-- One payment exists once, whatever range the bank sent it in.
CREATE UNIQUE INDEX bank_transaction_fingerprint ON bank_transaction (fingerprint);
CREATE INDEX bank_transaction_statement ON bank_transaction (statement_id, booking_date_local);
CREATE INDEX bank_transaction_status ON bank_transaction (status, booking_date_local);

CREATE TABLE allocation (
    id                   TEXT PRIMARY KEY,
    transaction_id       TEXT NOT NULL REFERENCES bank_transaction(id),
    open_item_id         TEXT REFERENCES open_item(id),
    entry_id             TEXT REFERENCES journal_entry(id),
    -- Always positive; the direction comes from the transaction's sign.
    amount_micros        INTEGER NOT NULL CHECK (amount_micros > 0),
    fx_difference_micros INTEGER NOT NULL DEFAULT 0,
    matched_at           TEXT NOT NULL,
    matched_by           TEXT NOT NULL,
    match_method         TEXT NOT NULL CHECK (match_method IN ('exact_ref', 'exact_amount', 'group', 'manual')),
    reversed             INTEGER NOT NULL DEFAULT 0 CHECK (reversed IN (0, 1)),
    CHECK (open_item_id IS NOT NULL OR entry_id IS NOT NULL)
);

CREATE INDEX allocation_transaction ON allocation (transaction_id);
CREATE INDEX allocation_item ON allocation (open_item_id);

CREATE TRIGGER bank_transaction_no_delete BEFORE DELETE ON bank_transaction
BEGIN
    SELECT RAISE(ABORT, 'a bank transaction is evidence of what the bank said');
END;

CREATE TRIGGER allocation_no_update BEFORE UPDATE ON allocation
BEGIN
    SELECT RAISE(ABORT, 'an allocation is never edited; unmatching appends a reversal');
END;

CREATE TRIGGER allocation_no_delete BEFORE DELETE ON allocation
BEGIN
    SELECT RAISE(ABORT, 'an allocation is never edited; unmatching appends a reversal');
END;
