-- M10 / A1: the account a purchase's cost lands on.
--
-- An invoice we receive says what it cost, not where it belongs in our chart.
-- EN 16931 has no field whose meaning is "office costs" for every business, so
-- the account is a decision a person makes between recording the document and
-- posting it — the same shape as the tax code in 0016, and for the same reason:
-- the document is immutable, the decision is a fact with an author and a time,
-- and the posting snapshots it on the lines it wrote.
--
-- Until 0019 every purchase posted its base to `9999 Nog te rubriceren` with no
-- way out of it that a person would recognise. That account's doc comment has
-- claimed since the ledger landed that "a period close refuses to happen while
-- this account has a balance"; the close now does exactly that, which is what
-- makes this table load-bearing rather than decorative.
--
-- One account per document, not per line. A purchase that mixes kinds of cost
-- still needs a memorial correction to split it, because splitting belongs with an
-- item catalogue and supplier defaults, which do not exist yet.
--
-- No down-guard beyond the migration itself: the account a posted line landed on
-- is on the line, so the books stay right if this table goes. What is lost is why,
-- which is why the down migration refuses once a posted document carries a choice.

CREATE TABLE document_account (
    document_id  TEXT PRIMARY KEY REFERENCES document(id),
    account_code TEXT NOT NULL REFERENCES account(code),
    decided_by   TEXT NOT NULL,
    decided_at   TEXT NOT NULL
);
