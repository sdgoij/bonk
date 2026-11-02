-- 0004 meant to add an index for the find list's order, but it reused the name
-- `document_owner_date`, which 0002 had already given to
-- `document (owner_id, issue_date_local)`. `CREATE INDEX IF NOT EXISTS` matched
-- that name and created nothing, so the list still sorted every match: 20 000
-- documents took 764 ms to draw fifty.
--
-- A distinct name is what makes the index exist. With `owner_id` fixed by the
-- WHERE, the remaining columns are exactly the ORDER BY, so the read is an index
-- scan that stops at the page instead of a sort of everything.
CREATE INDEX IF NOT EXISTS document_owner_order
    ON document (owner_id, issue_date_local DESC, number DESC);
