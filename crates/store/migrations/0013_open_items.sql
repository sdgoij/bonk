-- M10 / A2: open items, the detail behind the AR and AP control accounts.
--
-- An open item is created when a document is posted and is never updated: its
-- remaining balance is `amount - sum(allocations)`, derived, because a stored
-- "paid" flag is the classic way for a subledger to disagree with the journal it
-- is supposed to explain.
--
-- The amount is signed. A credit note is a negative open item until it is
-- allocated against the invoice it corrects, and that sign is what keeps the
-- control account equal to the sum of the subledger after a correction.

CREATE TABLE open_item (
    id                  TEXT PRIMARY KEY,
    seller_id           TEXT NOT NULL REFERENCES seller(id),
    kind                TEXT NOT NULL CHECK (kind IN ('receivable', 'payable')),
    party_id            TEXT NOT NULL,
    source_kind         TEXT NOT NULL,
    source_id           TEXT NOT NULL,
    amount_micros       INTEGER NOT NULL CHECK (amount_micros <> 0),
    currency            TEXT NOT NULL CHECK (length(currency) = 3),
    due_date_local      TEXT,
    original_date_local TEXT NOT NULL,
    created_at          TEXT NOT NULL,
    UNIQUE (source_kind, source_id)
);

CREATE INDEX open_item_seller ON open_item (seller_id, kind, due_date_local);
CREATE INDEX open_item_party ON open_item (party_id, kind);

CREATE TRIGGER open_item_no_update BEFORE UPDATE ON open_item
BEGIN
    SELECT RAISE(ABORT, 'open items are created once and settled by allocation');
END;

CREATE TRIGGER open_item_no_delete BEFORE DELETE ON open_item
BEGIN
    SELECT RAISE(ABORT, 'open items are created once and settled by allocation');
END;
