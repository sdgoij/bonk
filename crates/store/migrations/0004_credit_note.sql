-- M2: credit notes (381) reference the invoice they correct (BG-3).
--
-- The number and date are snapshotted on both the draft and the issued row so
-- the correction keeps naming the document it was issued against, even if the
-- original is later archived or renamed in a future schema.
--
-- `issued_invoice.customer_id` lets a credit note copy the original's customer
-- without guessing from the party snapshot.

ALTER TABLE invoice_draft ADD COLUMN preceding_invoice_id TEXT;
ALTER TABLE invoice_draft ADD COLUMN preceding_number TEXT;
ALTER TABLE invoice_draft ADD COLUMN preceding_issue_date TEXT;

ALTER TABLE issued_invoice ADD COLUMN customer_id TEXT;
ALTER TABLE issued_invoice ADD COLUMN preceding_invoice_id TEXT;
ALTER TABLE issued_invoice ADD COLUMN preceding_number TEXT;
ALTER TABLE issued_invoice ADD COLUMN preceding_issue_date TEXT;

CREATE INDEX issued_invoice_number_idx ON issued_invoice (seller_id, number);
