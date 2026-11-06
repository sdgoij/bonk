-- Rollback of 0015.
--
-- The original shape cannot hold a settlement, so a rollback refuses while any
-- allocation exists rather than discarding matches that were made.
CREATE TEMP TABLE allocation_rollback_guard (
    allocations INTEGER NOT NULL CONSTRAINT no_allocations CHECK (allocations = 0)
);
INSERT INTO allocation_rollback_guard (allocations) SELECT count(*) FROM allocation;
DROP TABLE allocation_rollback_guard;

CREATE TABLE allocation_v1 (
    id                   TEXT PRIMARY KEY,
    transaction_id       TEXT NOT NULL REFERENCES bank_transaction(id),
    open_item_id         TEXT REFERENCES open_item(id),
    entry_id             TEXT REFERENCES journal_entry(id),
    amount_micros        INTEGER NOT NULL CHECK (amount_micros > 0),
    fx_difference_micros INTEGER NOT NULL DEFAULT 0,
    matched_at           TEXT NOT NULL,
    matched_by           TEXT NOT NULL,
    match_method         TEXT NOT NULL CHECK (match_method IN ('exact_ref', 'exact_amount', 'group', 'manual')),
    reversed             INTEGER NOT NULL DEFAULT 0 CHECK (reversed IN (0, 1)),
    CHECK (open_item_id IS NOT NULL OR entry_id IS NOT NULL)
);

DROP TRIGGER IF EXISTS allocation_no_delete;
DROP TRIGGER IF EXISTS allocation_no_update;
DROP TABLE allocation;
ALTER TABLE allocation_v1 RENAME TO allocation;

CREATE INDEX allocation_transaction ON allocation (transaction_id);
CREATE INDEX allocation_item ON allocation (open_item_id);
