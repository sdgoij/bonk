-- The find list and the export both read documents in issue-date order, newest
-- first, and only ever draw a page of them. Without an index to walk, SQLite
-- sorts every match before it can hand back the first row: 20 000 documents took
-- 770 ms to draw fifty. With it the read is an index scan and stops at the limit.
CREATE INDEX IF NOT EXISTS document_owner_date
    ON document (owner_id, issue_date_local DESC, number DESC);
