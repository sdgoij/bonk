-- Rollback of 0022.
--
-- The two columns are a copy of facts the entry still carries, so dropping them
-- loses nothing. The index goes with them.
DROP INDEX journal_line_balance;
ALTER TABLE journal_line DROP COLUMN period_id;
ALTER TABLE journal_line DROP COLUMN entry_date_local;
