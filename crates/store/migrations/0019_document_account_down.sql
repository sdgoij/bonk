-- Rollback of 0019.
--
-- Refused while a posted document carries a choice. The account itself is on the
-- journal line, so dropping the table would not move a euro — but it would erase
-- who decided that a purchase belongs on 7100 and when, and a figure in a filed
-- return would no longer be explainable from the archive.
CREATE TEMP TABLE document_account_rollback_guard (
    decided INTEGER NOT NULL CONSTRAINT no_posted_choices CHECK (decided = 0)
);
INSERT INTO document_account_rollback_guard (decided)
SELECT count(*)
FROM document_account a
JOIN journal_entry e
  ON e.source_id = a.document_id AND e.source_kind = 'inbound_document';
DROP TABLE document_account_rollback_guard;

DROP TABLE document_account;
