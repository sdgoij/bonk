-- M10 / A3: tax codes, the treatment a purchase is filed under.
--
-- v1.0 created a `tax_code` table as a hook (§1.5 of the technical design) before
-- the rubriek mapping was settled. Nothing ever wrote a row: no command, no UI,
-- no import. This migration reshapes it into the catalogue the ledger needs
-- rather than adding a second table about the same thing.
--
-- What changed and why:
--
-- - `seller_id` and `archived` go. The codes are Dutch treatments resolved by a
--   published mapping version, not a list a business curates: two sellers in one
--   archive file the same boxes. `journal` and `rubriek` say where a code lands,
--   which is what the ledger reads.
-- - `vat_rate_bp` and `reverse_charge` go. The rate belongs to the document, and
--   a code that repeated it could disagree with the invoice; the reverse-charge
--   flag is what the `AE` category already says. `exemption_reason` and its code
--   live on the document, where EN 16931 asks for them.
-- - `id` stays the stable token (e.g. `NL-AE-GOODS`), and `label` the name a
--   person reads, so `invoice_draft_line.tax_code_id` keeps meaning the same.
--
-- The table is rebuilt rather than altered so no column keeps a default nobody
-- would want for a row inserted later.

CREATE TABLE tax_code_new (
    id              TEXT PRIMARY KEY,
    label           TEXT NOT NULL,
    -- The EN 16931 category this code resolves.
    vat_category    TEXT NOT NULL,
    -- The journal whose postings it applies to. Sales need no code: the category
    -- and the rate already decide the box.
    journal         TEXT NOT NULL REFERENCES journal(code),
    -- The box the taxable base and its VAT land in.
    rubriek         TEXT NOT NULL,
    -- The release of the rubriek mapping that introduced the code, so a return
    -- can say which set of definitions produced it.
    mapping_version TEXT NOT NULL,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

INSERT INTO tax_code_new
    (id, label, vat_category, journal, rubriek, mapping_version, created_at, updated_at)
VALUES
    ('NL-AE-SERVICES', 'Btw verlegd, diensten', 'AE', 'IK', '4a', 'nl-2026.1',
     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
    ('NL-AE-GOODS', 'Btw verlegd, goederen', 'AE', 'IK', '4b', 'nl-2026.1',
     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
    ('NL-K-GOODS', 'Intracommunautaire verwerving', 'K', 'IK', '2a', 'nl-2026.1',
     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');

DROP INDEX tax_code_seller_idx;
DROP TABLE tax_code;
ALTER TABLE tax_code_new RENAME TO tax_code;

-- Which treatment a document is filed under, and who decided it. The document
-- itself stays immutable; this is a separate fact with an author, like the
-- supplier match. It is replaced rather than versioned, because correcting a
-- classification before posting is a change of mind about the same document.
CREATE TABLE document_tax_code (
    document_id TEXT PRIMARY KEY REFERENCES document(id),
    code        TEXT NOT NULL REFERENCES tax_code(id),
    decided_by  TEXT NOT NULL,
    decided_at  TEXT NOT NULL
);

CREATE INDEX document_tax_code_code ON document_tax_code (code);
