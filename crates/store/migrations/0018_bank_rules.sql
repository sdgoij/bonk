-- M10 / A4: the rules that classify a bank line no invoice will ever match.
--
-- Bank fees, interest, VAT payments, payroll, and transfers between the business's
-- own accounts arrive as plain bank lines: nobody will ever match them to an
-- invoice, and until now they sat in the queue forever. A rule is a standing
-- instruction about that shape of line, written by the operator, naming the one
-- account the money belongs on.
--
-- A rule is deliberately not a pattern language. Four matchers — the counterparty
-- IBAN exactly, a case-insensitive substring of the remittance text, the same of
-- the counterparty name, and the direction — and every matcher that is set has to
-- agree. No amount ranges, no regular expressions, no negation: a program that
-- posts money should not be something an operator writes by accident.
--
-- `position` orders the list a person reads. It does not break ties: two rules that
-- both fit a line are a question, not a coin toss, and the line stays in the queue
-- with both names reported. That is the same rule the matcher already follows when
-- two invoices could be paid by one transfer.
--
-- `account_code` is the account the bank line is booked *against*; the bank account
-- itself comes from the statement the line arrived in. A rule carries no VAT
-- category, rate, or tax code, because a bank line that carries VAT is a purchase
-- invoice, and letting a rule guess a rate would under-declare input tax silently.

CREATE TABLE bank_rule (
    id                    TEXT PRIMARY KEY,
    seller_id             TEXT NOT NULL REFERENCES seller(id),
    -- What the operator calls it. The audit event and the entry's description name
    -- it, so a reader of the journal can see which instruction fired.
    name                  TEXT NOT NULL,
    -- The order rules are listed in. Lower first.
    position              INTEGER NOT NULL,
    enabled               INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    -- Which way the money must go: 1 in, -1 out, NULL either.
    direction             INTEGER CHECK (direction IN (1, -1)),
    -- The matchers. At least one has to be set, or the rule would match every line.
    counterparty_iban     TEXT,
    remittance_contains   TEXT,
    counterparty_contains TEXT,
    -- Where the line is booked: the bank against this account.
    account_code          TEXT NOT NULL REFERENCES account(code),
    created_at            TEXT NOT NULL,
    updated_at            TEXT NOT NULL,
    UNIQUE (seller_id, name),
    CHECK (counterparty_iban IS NOT NULL OR remittance_contains IS NOT NULL
           OR counterparty_contains IS NOT NULL)
);

CREATE INDEX bank_rule_order ON bank_rule (seller_id, position);
