-- Rollback for 0004: drops the credit-note reference columns and the number
-- index. Issued documents lose their BG-3 link; the documents themselves and
-- the audit chain are untouched.

DROP INDEX issued_invoice_number_idx;

ALTER TABLE issued_invoice DROP COLUMN preceding_issue_date;
ALTER TABLE issued_invoice DROP COLUMN preceding_number;
ALTER TABLE issued_invoice DROP COLUMN preceding_invoice_id;
ALTER TABLE issued_invoice DROP COLUMN customer_id;

ALTER TABLE invoice_draft DROP COLUMN preceding_issue_date;
ALTER TABLE invoice_draft DROP COLUMN preceding_number;
ALTER TABLE invoice_draft DROP COLUMN preceding_invoice_id;
