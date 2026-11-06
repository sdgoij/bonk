-- M10 / A4: allocations carry a sign.
--
-- Settling an open item is a negative allocation, and unmatching appends its
-- positive opposite. That keeps one rule for receivables and payables, makes the
-- history legible without a flag anyone can flip, and keeps every allocation
-- append-only: what happened, happened.

CREATE TABLE allocation_v2 (
    id                   TEXT PRIMARY KEY,
    transaction_id       TEXT NOT NULL REFERENCES bank_transaction(id),
    open_item_id         TEXT REFERENCES open_item(id),
    entry_id             TEXT REFERENCES journal_entry(id),
    -- Negative settles, positive undoes. Zero is not an allocation.
    amount_micros        INTEGER NOT NULL CHECK (amount_micros <> 0),
    fx_difference_micros INTEGER NOT NULL DEFAULT 0,
    matched_at           TEXT NOT NULL,
    matched_by           TEXT NOT NULL,
    match_method         TEXT NOT NULL CHECK (match_method IN ('exact_ref', 'exact_amount', 'group', 'manual')),
    reversed             INTEGER NOT NULL DEFAULT 0 CHECK (reversed IN (0, 1)),
    CHECK (open_item_id IS NOT NULL OR entry_id IS NOT NULL)
);

INSERT INTO allocation_v2
    (id, transaction_id, open_item_id, entry_id, amount_micros, fx_difference_micros,
     matched_at, matched_by, match_method, reversed)
SELECT id, transaction_id, open_item_id, entry_id, amount_micros, fx_difference_micros,
       matched_at, matched_by, match_method, reversed
FROM allocation;

DROP TRIGGER allocation_no_delete;
DROP TRIGGER allocation_no_update;
DROP TABLE allocation;
ALTER TABLE allocation_v2 RENAME TO allocation;

CREATE INDEX allocation_transaction ON allocation (transaction_id);
CREATE INDEX allocation_item ON allocation (open_item_id);

CREATE TRIGGER allocation_no_update BEFORE UPDATE ON allocation
BEGIN
    SELECT RAISE(ABORT, 'an allocation is never edited; unmatching appends a reversal');
END;

CREATE TRIGGER allocation_no_delete BEFORE DELETE ON allocation
BEGIN
    SELECT RAISE(ABORT, 'an allocation is never edited; unmatching appends a reversal');
END;
