-- Rollback of 0017.
--
-- Nothing is refused here. A profile is configuration, not evidence: statements
-- already imported keep their own bytes and payload hash, and an entry posted
-- from a transaction is in the ledger regardless of how the file was read. What
-- is lost is the ability to import that bank's CSV again, which is a reason to
-- export the profiles first rather than a reason to refuse.
DROP TABLE import_profile;
