-- M10: the report's date and period on the line itself.
--
-- The trial balance joined every line to its entry and its period to filter by the
-- seller and the window, which made the report a random lookup per line: 500 000
-- lines took seconds. These two columns are a denormalised copy of facts the entry
-- already holds, written by the posting and backfilled here. The covering index
-- below then lets the report read the lines and group by account without touching
-- the entry at all.
--
-- Reversible: dropping the columns restores the old query's inputs, and nothing is
-- lost that the entry does not still carry.

ALTER TABLE journal_line ADD COLUMN entry_date_local TEXT NOT NULL DEFAULT '';
ALTER TABLE journal_line ADD COLUMN period_id TEXT NOT NULL DEFAULT '';

-- The append-only trigger would refuse the backfill, which is the point of the
-- trigger. Drop it for the one statement that copies facts already in the entry,
-- then put it back exactly as 0011 wrote it.
DROP TRIGGER journal_line_no_update;
UPDATE journal_line
SET entry_date_local = e.entry_date_local,
    period_id = e.period_id
FROM journal_entry e
WHERE e.id = journal_line.entry_id;
CREATE TRIGGER journal_line_no_update BEFORE UPDATE ON journal_line
BEGIN
    SELECT RAISE(ABORT, 'journal lines are append-only');
END;

-- The report reads account, date, period, side and amount in one pass and groups
-- by account; this index is in that order, so the grouping needs no temporary
-- B-tree and the line table is never fetched.
CREATE INDEX journal_line_balance
    ON journal_line (account_code, entry_date_local, period_id, side, base_amount_micros);
