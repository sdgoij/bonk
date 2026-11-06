-- Rollback of 0016.
--
-- Refused while a posting carries a code: that code is what put the line's VAT
-- in a box, so dropping the catalogue would leave a filed return unexplained.
CREATE TEMP TABLE tax_code_rollback_guard (
    coded INTEGER NOT NULL CONSTRAINT no_coded_lines CHECK (coded = 0)
);
INSERT INTO tax_code_rollback_guard (coded) SELECT count(*) FROM journal_line WHERE tax_code_id IS NOT NULL;
DROP TABLE tax_code_rollback_guard;

DROP TABLE document_tax_code;

-- Restore the v1.0 hook table, empty, so a rolled-back archive opens with the
-- schema its version number promises.
CREATE TABLE tax_code_hook (
    id                    TEXT PRIMARY KEY,
    seller_id             TEXT NOT NULL REFERENCES seller(id),
    jurisdiction          TEXT NOT NULL DEFAULT 'NL',
    label                 TEXT NOT NULL,
    vat_category          TEXT NOT NULL,
    vat_rate_bp           INTEGER NOT NULL CHECK (vat_rate_bp BETWEEN 0 AND 10000),
    reverse_charge        INTEGER NOT NULL DEFAULT 0,
    exemption_reason      TEXT,
    exemption_reason_code TEXT,
    archived              INTEGER NOT NULL DEFAULT 0,
    created_at            TEXT NOT NULL,
    updated_at            TEXT NOT NULL
);

DROP TABLE tax_code;
ALTER TABLE tax_code_hook RENAME TO tax_code;
CREATE INDEX tax_code_seller_idx ON tax_code (seller_id, archived);
