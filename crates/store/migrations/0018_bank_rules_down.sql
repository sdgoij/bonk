-- Rollback of 0018.
--
-- Nothing is refused. A rule is configuration, not evidence: a line it classified
-- already has its entry and its audit event, and reversing that entry is the
-- ordinary correction path whether or not the rule still exists. What is lost is
-- the instruction, which is a reason to write it down again rather than a reason to
-- refuse the rollback.
DROP TABLE bank_rule;
