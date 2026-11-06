-- Rollback of 0011.
--
-- Only safe while the ledger is empty: a posting is the evidence behind a filed
-- VAT return, so removing one is not a schema change but a loss of records. The
-- guard below turns that into a refusal the caller can report, instead of a
-- script that quietly deletes the books.
CREATE TEMP TABLE ledger_rollback_guard (
    postings INTEGER NOT NULL CONSTRAINT no_postings CHECK (postings = 0)
);
INSERT INTO ledger_rollback_guard (postings) SELECT count(*) FROM journal_entry;
DROP TABLE ledger_rollback_guard;

DROP TRIGGER journal_entry_open_period;
DROP TRIGGER period_event_no_delete;
DROP TRIGGER period_event_no_update;
DROP TRIGGER journal_line_no_delete;
DROP TRIGGER journal_line_no_update;
DROP TRIGGER journal_entry_no_delete;
DROP TRIGGER journal_entry_no_update;

DROP TABLE journal_line;
DROP TABLE journal_entry;
DROP TABLE period_event;
DROP TABLE fiscal_period;
DROP TABLE journal;
DROP TABLE account;
