-- Rollback of 0014.
--
-- Refused while any statement is stored: a statement is the bank's assertion
-- about a balance, and the allocations below it are the record of what was
-- matched. Losing them would make a reconciled quarter unexplainable.
CREATE TEMP TABLE bank_rollback_guard (
    statements INTEGER NOT NULL CONSTRAINT no_bank_statements CHECK (statements = 0)
);
INSERT INTO bank_rollback_guard (statements) SELECT count(*) FROM bank_statement;
DROP TABLE bank_rollback_guard;

DROP TRIGGER allocation_no_delete;
DROP TRIGGER allocation_no_update;
DROP TRIGGER bank_transaction_no_delete;

DROP TABLE allocation;
DROP TABLE bank_transaction;
DROP TABLE bank_statement;
DROP TABLE bank_account;
