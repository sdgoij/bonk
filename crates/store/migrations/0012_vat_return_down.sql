-- Rollback of 0012.
--
-- Refused while a return exists: a prepared return is the record of what was
-- filed, and dropping it would erase the evidence rather than undo a schema.
CREATE TEMP TABLE vat_rollback_guard (
    returns INTEGER NOT NULL CONSTRAINT no_vat_returns CHECK (returns = 0)
);
INSERT INTO vat_rollback_guard (returns) SELECT count(*) FROM vat_return;
DROP TABLE vat_rollback_guard;

DROP TRIGGER vat_basis_no_delete;
DROP TRIGGER vat_return_line_no_delete;
DROP TRIGGER vat_return_no_delete;

DROP TABLE vat_basis_posting;
DROP TABLE vat_return_line;
DROP TABLE vat_return;

ALTER TABLE journal_line DROP COLUMN vat_rate_bp;
ALTER TABLE journal_line DROP COLUMN vat_category;
