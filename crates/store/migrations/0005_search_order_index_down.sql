-- Dropping an index loses no evidence: every row it pointed at is still there.
-- The list falls back to sorting its matches, which is slower but correct.
DROP INDEX IF EXISTS document_owner_order;
