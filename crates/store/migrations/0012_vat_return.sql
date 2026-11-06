-- M10 / A3: the VAT treatment of a posting, and prepared returns.
--
-- The category and rate travel on the line because the return needs them after
-- the fact: two purchases can land on the same costs account and still belong in
-- different rubrieken, and a report that re-derived the treatment from the
-- account would have to guess. Both stay null where a line carries no VAT
-- meaning at all — a payment, an opening balance.
--
-- A return is stored, not recomputed on demand: filing is an event in the life of
-- a business, and the numbers that were filed have to stay readable even after
-- the postings behind them are corrected. The basis table names the entries the
-- numbers came from, so a prepared return can always be explained.

ALTER TABLE journal_line ADD COLUMN vat_category TEXT;
ALTER TABLE journal_line ADD COLUMN vat_rate_bp INTEGER;

CREATE TABLE vat_return (
    id                 TEXT PRIMARY KEY,
    seller_id          TEXT NOT NULL REFERENCES seller(id),
    year               INTEGER NOT NULL,
    period_no          INTEGER NOT NULL,
    status             TEXT NOT NULL CHECK (status IN ('prepared', 'superseded')),
    prepared_at        TEXT NOT NULL,
    -- Hash over the ordered (entry, content hash) list the numbers came from.
    basis_hash         TEXT NOT NULL,
    totals_json        TEXT NOT NULL,
    -- Recorded by hand when the user filed it elsewhere; v1.1 does not file.
    external_reference TEXT,
    notes              TEXT
);

-- One return per period is the current one; superseded ones stay as history.
CREATE UNIQUE INDEX vat_return_one_prepared ON vat_return (seller_id, year, period_no)
    WHERE status = 'prepared';

CREATE TABLE vat_return_line (
    return_id   TEXT NOT NULL REFERENCES vat_return(id),
    rubriek     TEXT NOT NULL,
    base_micros INTEGER NOT NULL,
    vat_micros  INTEGER NOT NULL,
    PRIMARY KEY (return_id, rubriek)
);

CREATE TABLE vat_basis_posting (
    return_id TEXT NOT NULL REFERENCES vat_return(id),
    entry_id  TEXT NOT NULL REFERENCES journal_entry(id),
    PRIMARY KEY (return_id, entry_id)
);

CREATE INDEX vat_basis_entry ON vat_basis_posting (entry_id);

CREATE TRIGGER vat_return_no_delete BEFORE DELETE ON vat_return
BEGIN
    SELECT RAISE(ABORT, 'a prepared VAT return is evidence and is not deleted');
END;

CREATE TRIGGER vat_return_line_no_delete BEFORE DELETE ON vat_return_line
BEGIN
    SELECT RAISE(ABORT, 'a prepared VAT return is evidence and is not deleted');
END;

CREATE TRIGGER vat_basis_no_delete BEFORE DELETE ON vat_basis_posting
BEGIN
    SELECT RAISE(ABORT, 'a prepared VAT return is evidence and is not deleted');
END;
