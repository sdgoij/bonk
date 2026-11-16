-- M10: foreign-currency postings.
--
-- A rate is stored as reporting-currency micro-units per one unit of the source
-- currency, the shape `journal_line.fx_rate_micros` already reserves. The cache
-- holds rates already crossed into the books' reporting currency: the ECB quotes
-- every currency per euro, and importing crosses the two quotes so a lookup is
-- one row rather than a join of two.
--
-- The source is versioned by the hash of the file it was imported from. An
-- override is a decision about one document, so it lives in its own table with
-- an author and a time, exactly like the tax code and the cost account beside it.
--
-- Irreversible: dropping the reporting-currency amounts after a foreign posting
-- exists would leave the control accounts explaining themselves with numbers the
-- subledger no longer holds. The journal lines keep the rate and the base amount,
-- so the postings would still be readable, but open items and reports would
-- silently disagree. A rollback is cheaper than a wrong reconciliation only
-- before the first foreign posting.

CREATE TABLE fx_rate (
    id               TEXT PRIMARY KEY,
    seller_id        TEXT NOT NULL REFERENCES seller(id),
    currency         TEXT NOT NULL CHECK (length(currency) = 3),
    base_currency    TEXT NOT NULL CHECK (length(base_currency) = 3),
    -- The date the rate applies from. A posting takes the latest row at or
    -- before its own date, so a weekend uses Friday's reference rate.
    as_of            TEXT NOT NULL,
    rate_micros      INTEGER NOT NULL CHECK (rate_micros > 0),
    source           TEXT NOT NULL CHECK (source IN ('ecb', 'manual', 'demo')),
    provider         TEXT NOT NULL,
    provider_version TEXT NOT NULL,
    recorded_at      TEXT NOT NULL,
    recorded_by      TEXT NOT NULL,
    UNIQUE (seller_id, currency, base_currency, as_of, source)
);

CREATE INDEX fx_rate_lookup ON fx_rate (seller_id, currency, base_currency, as_of);

CREATE TABLE document_fx_rate (
    document_id   TEXT PRIMARY KEY REFERENCES document(id),
    base_currency TEXT NOT NULL CHECK (length(base_currency) = 3),
    rate_micros   INTEGER NOT NULL CHECK (rate_micros > 0),
    -- The date the chosen rate belongs to, which is the document's issue date
    -- unless the person overriding it says otherwise.
    as_of         TEXT NOT NULL,
    decided_by    TEXT NOT NULL,
    decided_at    TEXT NOT NULL
);

-- The reporting currency a converted entry was posted into. Null means the entry
-- is in the reporting currency already, which is the shape every posting before
-- this migration has.
ALTER TABLE journal_entry ADD COLUMN base_currency TEXT
    CHECK (base_currency IS NULL OR length(base_currency) = 3);

-- An open item keeps the rate it was posted at so its carrying value in the
-- reporting currency can be released on settlement without re-reading the ledger.
ALTER TABLE open_item ADD COLUMN base_amount_micros INTEGER NOT NULL DEFAULT 0;
ALTER TABLE open_item ADD COLUMN base_currency TEXT
    CHECK (base_currency IS NULL OR length(base_currency) = 3);
ALTER TABLE open_item ADD COLUMN fx_rate_micros INTEGER
    CHECK (fx_rate_micros IS NULL OR fx_rate_micros > 0);

-- Items written before this migration were in the reporting currency, or were
-- posted without conversion; either way their stored amount is what the ledger
-- recorded as the carrying value, and the ledger carried it in the document's
-- own currency.
UPDATE open_item SET base_amount_micros = amount_micros, base_currency = currency;

-- The same carrying value on the settlement side. An allocation written before
-- this migration had the same currency as its item and the ledger carried it at
-- the item's amount; new allocations of a converted item are refused until the
-- settlement slice can clear the carrying value and post the realised difference.
ALTER TABLE allocation ADD COLUMN base_amount_micros INTEGER NOT NULL DEFAULT 0;
UPDATE allocation SET base_amount_micros = amount_micros;
