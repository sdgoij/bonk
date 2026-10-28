# Accounting Layer Design (v1.1)

**Project:** Bonk (working name; final name TBD)
**Status:** Draft 0.1 for review
**Date:** 2026-10-27
**Scope:** the bookkeeping layer scheduled for v1.1
**Companion to:** `technical-design.md` (v1.0), `implementation-plan.md`

---

## Contents

1. [Thesis and scope](#1-thesis-and-scope)
2. [Cut line](#2-cut-line)
3. [Accounting model](#3-accounting-model)
4. [Posting rules](#4-posting-rules)
5. [Data model](#5-data-model)
6. [VAT return preparation](#6-vat-return-preparation)
7. [Bank reconciliation](#7-bank-reconciliation)
8. [Reports and exports](#8-reports-and-exports)
9. [Immutability, audit, period close](#9-immutability-audit-period-close)
10. [Local-first and synchronisation](#10-local-first-and-synchronisation)
11. [Security and privacy](#11-security-and-privacy)
12. [Testing and acceptance](#12-testing-and-acceptance)
13. [Effort](#13-effort)
14. [Decisions](#14-decisions)
15. [Glossary](#15-glossary)

---

## 1. Thesis and scope

v1.0 stores invoices as immutable, hash-addressed facts with party snapshots,
resolved tax treatment, and business dates. A ledger is a deterministic
projection of exactly those facts. That makes the accounting layer the natural
next step, and it makes the choice of what v1.0 records today load-bearing: the
only thing v1.1 cannot reconstruct is a fact that was never recorded.

**Thesis:** the invoice archive is the source of truth; the ledger is derived,
versioned, and reprovable. The user never enters the same invoice twice, and the
books can always be explained by pointing at the documents that produced them.

This is also the commercial logic. An invoice tool is a utility; a bookkeeping
system is the thing a small business actually runs on, and it is the reason not
to leave for a SaaS once the archive is full.

### 1.1 What v1.1 adds

- Double-entry general ledger with journals and a chart of accounts.
- Automated postings from issued invoices, credit notes, and imported purchase
  invoices.
- Fiscal periods, opening balances, period locking.
- Dutch VAT return preparation (BTW-aangifte) with an immutable basis snapshot.
- Bank statement import and assisted reconciliation against open items.
- Trial balance, P&L, balance sheet, AR/AP aging, and an accountant handoff
  package.

### 1.2 What it does not add

See §2. The short version: no payroll, no inventory valuation, no fixed-asset
depreciation, no consolidation, no purchase-order matching, and no filing
through Digipoort.

---

## 2. Cut line

The cut line exists so the accounting layer does not become an ERP. Every item
below is excluded *by decision*, not by omission.

| Excluded | Why | Workaround in v1.1 |
|----------|-----|--------------------|
| Payroll | Separate legal and integration surface (loonheffing, UPA) | Manual journal entry |
| Inventory valuation | Requires costing method decisions and stock movements | Post purchases to an expense account |
| Fixed assets / depreciation schedules | Needs a register and disposal logic | Manual journal entry, or accountant export |
| Consolidation of multiple entities | Multi-entity is a different product | One seller per database |
| Purchase orders / three-way matching | Procurement workflow, not bookkeeping | None |
| Filing VAT via Digipoort | eHerkenning, certification, liability | Prepare + export + audit snapshot |
| OCR of receipts | Separate product surface | Bank transaction posted directly to an expense account |
| Unrealised FX revaluation | Period-end policy, requires FX rate policy | Realised differences only, documented |
| Multi-currency ledgers per account | Rarely needed at this scale | Single reporting currency, transaction currency on documents |

The line is drawn at: **can a competent bookkeeper produce a correct VAT return
and a balanced trial balance for a Dutch small business from this data, without
Excel?** If yes, the feature is in. If it only helps a different persona
(controller, auditor, CFO), it is out.

---

## 3. Accounting model

### 3.1 Double entry

Every economic event becomes one or more **journal entries**. An entry belongs to
a journal (sales, purchases, bank, cash, memorial, opening, tax) and consists of
lines that are either debit or credit. Invariants:

1. `sum(debits) == sum(credits)` per entry, in the entry currency.
2. Every entry references at least one immutable source fact, or is a manual
   entry with an author and a reason.
3. Entries are append-only. Corrections are reversing entries plus a new entry.
4. An entry can only be posted into an open fiscal period.

### 3.2 Chart of accounts

Accounts are rows, not an enum, because every business argues about its chart.

```
account {
    code        -- "1300", user-visible and stable
    name        -- "Handelsdebiteuren"
    kind        -- asset | liability | equity | revenue | expense
    rgs_code    -- optional mapping to the Referentie Grootboek Schema
    parent_code -- hierarchy for reporting
    is_control  -- AR/AP control accounts, reconciled to subledger
    currency    -- NULL for the reporting currency
}
```

**RGS alignment.** The Dutch *Referentie Grootboek Schema* is a shared reference
chart that accountants and software already speak. We do **not** force RGS codes
(that fights real practice), and we do **not** ignore them (that makes exports
useless to an accountant). The model is: user accounts are primary, each may
carry an RGS code, and a ~60-account starter chart ships pre-mapped. Reports can
be produced either per account or rolled up per RGS group.

### 3.3 Periods

- A fiscal year is configured per seller (start month, typically January).
- Periods are monthly, with an optional 13th adjustment period for year-end
  entries.
- Each period is `open` or `locked`. Locking is itself an immutable event; a
  posting into a locked period is rejected by a database trigger and by the
  application, belt and braces.
- Reopening a period requires an explicit action, a reason, and an audit event.
  It never deletes the lock event; it appends a `reopened` fact.
- Closing a year requires: trial balance balances, every open item either
  settled or explained, VAT returns prepared for every period, and no unposted
  documents in the period range. The close report lists what blocks it.

### 3.4 Open items and subledgers

Accounts receivable and payable are control accounts in the GL. Their detail
lives in open items:

```
open_item {
    id, kind: receivable | payable,
    party_id, source_kind: outbound_document | inbound_document | manual,
    source_id, amount, currency, due_date, original_date
}
```

The remaining balance of an open item is **derived** from allocations, never
stored. This removes the classic bug where a cached "paid" flag disagrees with
the journal.

AR/AP aging buckets (current, 1–30, 31–60, 61–90, 90+) are computed from open
items, and the control-account balance must equal the sum of open-item balances;
a report shows any discrepancy rather than hiding it.

### 3.5 Money, rounding, and currency

- Reuse `crux::Money` (integer micro-units). No new money type, no floats.
- The ledger posts in the document currency for AR/AP and in the reporting
  currency for the GL, with the exchange rate recorded on the posting.
- Rates come from a versioned, offline cache of ECB reference rates; a user may
  override a rate per document, and the override is part of the posting record.
- Realised FX differences post to a dedicated account on payment.
- Rounding is applied once, at the point the value is fixed (invoice calc, VAT
  return line), and the rounding residue is posted explicitly, never absorbed
  into a revenue account.

---

## 4. Posting rules

### 4.1 Derivation, not entry

When an invoice is issued in v1.0, it carries everything a posting needs: party
snapshot, tax code, per-category tax amounts, totals, dates. v1.1 attaches a
**posting rule set** that maps those facts to accounts:

```
sales invoice
  Dr  debtors_control            total_incl_tax
  Cr  revenue_account(tax_code)  net_total
  Cr  vat_output_account         tax_total        (per VAT category)

credit note  → mirror of the above (reversal entry)

payment received
  Dr  bank_account               amount
  Cr  debtors_control            amount

purchase invoice
  Dr  expense_account            net_total
  Dr  vat_input_account          tax_total
  Cr  creditors_control          total_incl_tax
```

Line-level accounts come from the item catalogue when present (M10 adds an
optional account code to catalogue items), otherwise from a per-supplier or
per-tax-code default, otherwise from an "unclassified" account that the UI nags
about. An invoice with unclassified lines can be posted, but it cannot be part
of a period close.

### 4.2 Rule sets are versioned

Accounts, tax-code mappings, and default-account rules change over time. Each
posting records `posting_profile_id` and the rule-set version used. Changing a
default tomorrow must not silently rewrite yesterday's entry, and re-running
posting for an old invoice must reproduce the original entry or explain the
difference.

The posting engine is deterministic: same fact + same rule version = same entry.
That property is tested, not assumed.

### 4.3 Scenario matrix

Illustrative accounts; the shipped starter chart maps them to RGS.

| Scenario | Debit | Credit | Rubriek effect |
|----------|-------|--------|----------------|
| Domestic sale, 21 % | 1300 Debtors 1 210 | 8000 Revenue 1 000; 1600 VAT payable 210 | 1a, 5a |
| Domestic sale, 9 % | 1300 Debtors 1 090 | 8000 Revenue 1 000; 1610 VAT payable 90 | 1b, 5a |
| EU B2B services, VAT shifted to the customer (supplier side) | 1300 Debtors 1 000 | 8000 Revenue 1 000 | 3a + ICP listing |
| Export outside the EU | 1300 Debtors 1 000 | 8000 Revenue 1 000 | 3b |
| Purchase, EU B2B reverse charge | 7000 Expense 1 000; 1500 VAT receivable 210 | 1400 Creditors 1 000; 1600 VAT payable 210 | 4a, 5a, 5b |
| Intra-EU goods acquisition | 7000 Expense 1 000; 1500 VAT receivable 210 | 1400 Creditors 1 000; 1600 VAT payable 210 | 2a, 5a, 5b |
| Payment of an invoice | 1100 Bank 1 210 | 1300 Debtors 1 210 | — |
| Bank fee | 7100 Bank costs 5 | 1100 Bank 5 | — |
| Credit note | 8000 Revenue 1 000; 1600 VAT payable 210 | 1300 Debtors 1 210 | negative in the same rubriek |

The matrix is a test fixture, not documentation drift: each row is an acceptance
test comparing generated entries to the expected entry.

### 4.4 Reversal and correction

- Wrong posting: reverse with a mirror entry referencing the original, then post
  the correct entry. Never edit.
- Reversal entries carry `reverses_entry_id`; a reversed entry is excluded from
  balances by a derived flag, not by deletion.
- Manual entries (memorial) require an author, a description, and ideally a
  supporting attachment. They are the accountant's escape hatch and are logged
  like everything else.

---

## 5. Data model

```sql
CREATE TABLE account (
    code               TEXT PRIMARY KEY,
    name               TEXT NOT NULL,
    kind               TEXT NOT NULL CHECK (kind IN
                         ('asset','liability','equity','revenue','expense')),
    rgs_code           TEXT,
    parent_code        TEXT REFERENCES account(code),
    is_control         INTEGER NOT NULL DEFAULT 0,
    currency           TEXT,
    archived           INTEGER NOT NULL DEFAULT 0,
    updated_at         TEXT NOT NULL
);

CREATE TABLE journal (
    code               TEXT PRIMARY KEY,   -- VK, IK, BNK, KAS, MEM, OPEN, BTW
    name               TEXT NOT NULL,
    kind               TEXT NOT NULL
);

CREATE TABLE fiscal_period (
    id                 TEXT PRIMARY KEY,
    seller_id          TEXT NOT NULL REFERENCES seller(id),
    year               INTEGER NOT NULL,
    period_no          INTEGER NOT NULL,    -- 1..12, 13 = adjustment
    starts_on          TEXT NOT NULL,
    ends_on            TEXT NOT NULL,
    status             TEXT NOT NULL CHECK (status IN ('open','locked')),
    UNIQUE (seller_id, year, period_no)
);

CREATE TABLE period_event (
    id                 TEXT PRIMARY KEY,
    period_id          TEXT NOT NULL REFERENCES fiscal_period(id),
    kind               TEXT NOT NULL,       -- locked | reopened | closed
    reason             TEXT,
    at                 TEXT NOT NULL,
    actor              TEXT NOT NULL
);

CREATE TABLE journal_entry (
    id                 TEXT PRIMARY KEY,
    journal_code       TEXT NOT NULL REFERENCES journal(code),
    period_id          TEXT NOT NULL REFERENCES fiscal_period(id),
    number             TEXT NOT NULL,       -- rendered per journal and year, e.g. 2026-0001
    entry_date_local   TEXT NOT NULL,
    description        TEXT NOT NULL,
    source_kind        TEXT NOT NULL,       -- outbound_document | inbound_document | manual | opening | bank
    source_id          TEXT,
    reverses_entry_id  TEXT REFERENCES journal_entry(id),
    posting_profile_id TEXT NOT NULL,
    rule_version       TEXT NOT NULL,
    currency           TEXT NOT NULL,
    created_at         TEXT NOT NULL,
    created_by         TEXT NOT NULL,
    content_hash       TEXT NOT NULL,
    UNIQUE (journal_code, number)
);

CREATE TABLE journal_line (
    id                 TEXT PRIMARY KEY,
    entry_id           TEXT NOT NULL REFERENCES journal_entry(id),
    line_no            INTEGER NOT NULL,
    account_code       TEXT NOT NULL REFERENCES account(code),
    side               TEXT NOT NULL CHECK (side IN ('debit','credit')),
    amount_micros      INTEGER NOT NULL CHECK (amount_micros > 0),
    currency           TEXT NOT NULL,
    fx_rate_micros     INTEGER,             -- reporting currency per unit
    base_amount_micros INTEGER NOT NULL,
    party_id           TEXT,
    tax_code_id        TEXT,
    description        TEXT,
    UNIQUE (entry_id, line_no)
);

CREATE TABLE open_item (
    id                 TEXT PRIMARY KEY,
    kind               TEXT NOT NULL CHECK (kind IN ('receivable','payable')),
    party_id           TEXT NOT NULL,
    source_kind        TEXT NOT NULL,
    source_id          TEXT NOT NULL,
    amount_micros      INTEGER NOT NULL,
    currency           TEXT NOT NULL,
    due_date_local     TEXT,
    original_date_local TEXT NOT NULL,
    UNIQUE (source_kind, source_id)
);

CREATE TABLE bank_account (
    id                 TEXT PRIMARY KEY,
    name               TEXT NOT NULL,
    iban               TEXT,
    currency           TEXT NOT NULL,
    ledger_account_code TEXT NOT NULL REFERENCES account(code),
    import_profile     TEXT
);

CREATE TABLE bank_statement (
    id                 TEXT PRIMARY KEY,
    bank_account_id    TEXT NOT NULL REFERENCES bank_account(id),
    format             TEXT NOT NULL,       -- camt053 | mt940 | csv
    source_name        TEXT,
    period_start       TEXT,
    period_end         TEXT,
    opening_balance_micros INTEGER,
    closing_balance_micros INTEGER,
    payload_hash       TEXT NOT NULL,
    imported_at        TEXT NOT NULL
);

CREATE TABLE bank_transaction (
    id                 TEXT PRIMARY KEY,
    statement_id       TEXT NOT NULL REFERENCES bank_statement(id),
    booking_date_local TEXT NOT NULL,
    value_date_local   TEXT,
    amount_micros      INTEGER NOT NULL,    -- signed
    currency           TEXT NOT NULL,
    counterparty_name  TEXT,
    counterparty_iban  TEXT,
    remittance_info    TEXT,
    external_ref       TEXT,
    status             TEXT NOT NULL,       -- new | matched | partially_matched | posted | ignored
    ignored_reason     TEXT,
    fingerprint        TEXT NOT NULL         -- dedupe across overlapping statements
);

CREATE TABLE allocation (
    id                 TEXT PRIMARY KEY,
    transaction_id     TEXT NOT NULL REFERENCES bank_transaction(id),
    open_item_id       TEXT REFERENCES open_item(id),
    entry_id           TEXT REFERENCES journal_entry(id),  -- direct posting case
    amount_micros      INTEGER NOT NULL,
    fx_difference_micros INTEGER NOT NULL DEFAULT 0,
    matched_at         TEXT NOT NULL,
    matched_by         TEXT NOT NULL,
    match_method       TEXT NOT NULL,       -- exact_ref | exact_amount | manual | group
    reversed           INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE tax_code_mapping (
    tax_code_id        TEXT NOT NULL REFERENCES tax_code(id),
    version            TEXT NOT NULL,
    jurisdiction       TEXT NOT NULL,
    vat_return_box     TEXT NOT NULL,       -- NL rubriek, e.g. 1a
    output_account     TEXT REFERENCES account(code),
    input_account      TEXT REFERENCES account(code),
    icp_relevant       INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (tax_code_id, version)
);

CREATE TABLE vat_return (
    id                 TEXT PRIMARY KEY,
    seller_id          TEXT NOT NULL REFERENCES seller(id),
    year               INTEGER NOT NULL,
    period_no          INTEGER NOT NULL,
    status             TEXT NOT NULL CHECK (status IN
                         ('draft','prepared','externally_filed','superseded')),
    prepared_at        TEXT,
    basis_hash         TEXT,
    totals_json        TEXT,
    external_reference TEXT,               -- portal receipt number, if recorded
    notes              TEXT
);

CREATE TABLE vat_return_line (
    return_id          TEXT NOT NULL REFERENCES vat_return(id),
    rubriek            TEXT NOT NULL,
    base_micros        INTEGER NOT NULL,
    vat_micros         INTEGER NOT NULL,
    PRIMARY KEY (return_id, rubriek)
);

CREATE TABLE vat_basis_posting (
    return_id          TEXT NOT NULL REFERENCES vat_return(id),
    entry_id           TEXT NOT NULL REFERENCES journal_entry(id),
    PRIMARY KEY (return_id, entry_id)
);
```

Notes:

- `open_item` is created at posting time and never updated; its balance is
  `amount - sum(allocation)`. Denormalising the balance is forbidden.
- `bank_transaction.fingerprint` is a hash of date, amount, counterparty, and
  remittance info, so overlapping statement ranges cannot double-import.
- `vat_basis_posting` is the evidence link: a prepared return can always name the
  exact journal entries it was computed from.
- No table stores an account balance. Balances are queries; there is one primary
  index on `journal_line(account_code, entry_id)` and one covering the period
  join.

---

## 6. VAT return preparation

### 6.1 What a return is here

A VAT return is a **projection** of journal lines for a period, grouped by
rubriek. It is not a set of numbers typed by the user. Editing is limited to
explicit adjustments, which are posted as journal entries in the same period so
the return and the ledger can never drift.

### 6.2 Rubriek mapping

Each tax code maps to a rubriek through `tax_code_mapping`, which is versioned by
year. Illustrative subset of the Dutch return:

| Rubriek | Meaning | Typical source |
|---------|---------|----------------|
| 1a | Supplies at the high rate (21 %) | Domestic sales |
| 1b | Supplies at the low rate (9 %) | Domestic sales |
| 1c | Supplies at other rates (not 0 %) | Special schemes |
| 1d | Private use | Corrections |
| 1e | Supplies at 0 % or not taxed with right to deduct | Zero-rated |
| 2a | Intra-EU acquisitions | Purchases from EU suppliers |
| 3a | Intra-EU supplies | ICP listing, B2B services/goods |
| 3b | Exports outside the EU | Non-EU sales |
| 4a | Reverse-charged services | Imports of services, subcontracting |
| 4b | Reverse-charged goods/other | Construction, scrap, etc. |
| 5a | VAT due | Sum of computed output VAT |
| 5b | Input VAT | Deductible VAT |
| 5c | Payable / refundable | 5a − 5b |

The shipped table is a **versioned artefact**, not hardcoded constants, because
rubriek definitions and thresholds change with the annual tax form. The mapping
table is verified against the current Belastingdienst specification before each
release, and a golden test asserts the full mapping for the supported years.

### 6.3 Periods, ICP, and corrections

- Return period configured per seller: monthly or quarterly. The ICP listing
  follows the return frequency (monthly filers file monthly, quarterly filers
  may file quarterly) and is generated from rubriek 3a lines regardless of the
  chosen period; the report blocks if a 3a line has no valid VAT ID.
- Corrections for a prior period become a supplementary posting in the current
  period with a reference, or an explicit adjustment line, depending on the
  user's choice. Both are audited.
- A superseded return stays in the database forever; the new return references
  it.

### 6.4 Basis snapshot and evidence

Preparing a return writes:

1. `vat_return` with totals and status `prepared`.
2. `vat_basis_posting` rows for every contributing entry.
3. `basis_hash` over the ordered `(entry_id, content_hash)` list.
4. An audit event `vat_return_prepared`.

Recomputing later must either reproduce the same totals or report exactly which
postings changed since — this is the accounting analogue of invoice artefact
reproduction, and it is the reason the ledger is append-only.

### 6.5 Reconciliation before filing

The "ready to file" screen shows three checks and refuses to mark a return
prepared if any is red:

| Check | Rule |
|-------|------|
| Ledger ↔ return | Every line in a VAT account is mapped to exactly one rubriek; unmapped lines are listed |
| Invoice ↔ ledger | Per VAT category, invoice tax totals equal posted VAT totals for the period |
| Balance sheet | Trial balance nets to zero; AR/AP control accounts equal subledger sums |

### 6.6 Filing boundary

v1.1 prepares, reconciles, and exports: a printable summary, a CSV, and (later)
the official SBR/XBRL payload. Recording that a return was filed is a manual
action with an external reference and an optional receipt document. Direct
Digipoort filing requires eHerkenning and a certified intermediary role; if it
ever ships, it ships as a partner connector, not as core.

---

## 7. Bank reconciliation

### 7.1 Formats

| Format | Notes |
|--------|-------|
| CAMT.053 (ISO 20022) | Preferred; structured remittance info, balances |
| MT940 (SWIFT) | Legacy but still common; parse `:61:`/`:86:` carefully |
| CSV | Per-bank profiles with column mapping, decimal/date conventions, delimiter, encoding |

Each import profile is a stored, versioned mapping (bank → column semantics →
normalisation rules), so a CSV bank cannot corrupt amounts through a locale
guess. Encoding detection is explicit; failure is an error, not mojibake.

### 7.2 Import pipeline

```
file → format sniff (extension + content) → parse → validate balances
     → dedupe by fingerprint → persist statement + transactions
     → classify non-invoice lines by rule → match the rest
```

Whole-file rejection on a parse error, with the failing line number. Partial
imports of a bank statement are worse than no import.

### 7.3 Matching ladder

Ordered, each step only runs on what the previous step left:

1. **Rule-based classification.** Interest, bank fees, VAT payments, payroll,
   internal transfers → post directly to the configured account. Never matched
   against invoices.
2. **Exact reference.** Amount equals an open item and the normalised remittance
   info contains the invoice number (or the structured CAMT reference) →
   auto-match.
3. **Exact amount, unique counterparty item.** One candidate → propose with a
   one-click accept.
4. **Group match.** The sum of two or three open items equals the amount and the
   remittance info names them → propose a group allocation.
5. **Ranked candidates.** Score by amount delta, counterparty IBAN, name
   similarity, date proximity; show the top three with the reason.
6. **Unmatched queue.** Everything else, with a "post directly" escape hatch.

Auto-match (step 2) is still a recorded, reversible `allocation` with
`match_method = exact_ref`. There is no silent state: the transaction status and
the allocation rows are the truth, and both are auditable.

Never auto-match when two candidates tie, and never absorb a difference into an
invoice. A short payment becomes a partial allocation plus an explicit write-off
or fee entry.

### 7.4 Allocations

- One transaction can settle many open items; one open item can be settled by
  many transactions.
- `sum(allocation.amount) <= abs(transaction.amount)`; the remainder must be
  resolved (posted to an account, or the transaction is marked partial).
- Allocations are immutable; "unmatching" appends a reversal allocation.
- FX differences on settlement are posted separately and never folded into the
  matched amount.

### 7.5 UX for the unmatched queue

The queue is the daily work surface: keyboard-first, one candidate list per
transaction, running totals, and a visible "difference" field. Bulk actions are
limited to accepting proposals above a confidence threshold; anything ambiguous
is a human decision.

---

## 8. Reports and exports

| Report | Basis | Notes |
|--------|-------|-------|
| Trial balance (proefbalans) | Sum of journal lines per account and period | Must net to zero |
| P&L (winst- en verliesrekening) | Revenue and expense accounts | Per period, YTD, comparative |
| Balance sheet (balans) | Asset, liability, equity accounts | Includes derived current result |
| AR/AP aging | Open items | Buckets, per counterparty |
| VAT summary | Return lines | Per rubriek, with drill-down to postings |
| Journal listing | Entries per journal/period | Numbered, hash column |
| Accountant package | All of the above + documents | Zip with CSV, PDFs, XMLs, VAT return, audit chain head |

Export formats: CSV (UTF-8, semicolon and comma profiles), PDF for statements,
and XBRL/SBR later. Every export records an audit event with the report
parameters and a hash of the produced file, so a number in an email can be traced
back to the exact query that produced it.

---

## 9. Immutability, audit, period close

- Journal entries and lines are write-once, enforced by triggers like v1.0's
  issued invoices.
- `journal_entry.content_hash` is chained into the same audit log as invoice
  events, so one verifier covers both domains.
- Posting is idempotent: posting an invoice twice is a no-op keyed by
  `(source_kind, source_id)`, with a UNIQUE constraint to make it impossible.
- Period close writes an event with the close report hash; subsequent postings
  into the period are rejected.
- Deletion policy: nothing in the ledger is deletable while its period exists.
  Retention deletion (v1.0 §13.5) removes blobs under a policy, never postings.
- Evidence bundle gains `ledger.jsonl`, `vat-return.json`, and the trial balance,
  and the same offline verifier checks all of it.

---

## 10. Local-first and synchronisation

- v1.1 assumes **one posting device**. That is a deliberate limit: offline
  double-entry with multiple writers creates duplicate journal numbers and
  double-booked periods, and correctness beats convenience.
- Journal numbers use the same lease mechanism as invoice numbers (v1.0 §6.5).
- When v2.0 syncs the ledger, the rules are already compatible: entries are
  immutable facts merged by set union; period locks are facts that win over
  concurrent postings; a device that posted into a period another device locked
  surfaces the entry for review rather than silently dropping it.
- The relay, if used, sees ciphertext only. Accounting data is more sensitive
  than invoice data, not less.

---

## 11. Security and privacy

- The ledger extends the same encrypted database; no separate store, no plaintext
  cache, no CSV left in temp directories after export (exports are written where
  the user chose, and the app says so).
- Bank statement files are copied into the encrypted blob store and the original
  path is recorded; nothing is re-read from disk later.
- Support bundles include account codes and counts, never amounts, names, or
  IBANs.
- Audit access: any user with the passphrase can read the books. There is no
  role separation in v1.1 — say so plainly rather than implying controls that do
  not exist.

---

## 12. Testing and acceptance

| Level | Test |
|-------|------|
| Invariant | Debits equal credits per entry and globally; trial balance nets to zero; control accounts equal subledger sums |
| Idempotency | Posting the same invoice twice produces one entry and one audit event |
| Property | Random invoice sets → trial balance zero, revenue + VAT = AR, aging sums to control account |
| Determinism | Same fact + same rule version → identical entry bytes and hash |
| Golden | 15 VAT scenarios with expected rubriek totals, including credit notes, reverse charge, ICP, exports |
| Reconciliation | CAMT/MT940/CSV corpus with partials, combined payments, fees, refunds, duplicates, overlapping ranges |
| Period | Posting into a locked period fails; reopening is audited; close report lists blockers |
| Performance | Trial balance over 500k lines < 200 ms; VAT return over a year < 300 ms; 10k bank lines < 2 s |
| Migration | Opening balances import balances and is reflected in the first trial balance |

Acceptance for the milestone: a pilot company's full quarter is reconstructed
from its invoices, a bank statement, and an opening balance, producing a VAT
return that matches what their accountant filed (within explained differences),
and a trial balance that an accountant accepts without a spreadsheet.

---

## 13. Effort

| Work package | Est. |
|--------------|------|
| A1 Chart of accounts, journals, posting engine, reversal | 3 w |
| A2 Periods, opening balances, trial balance / P&L / balance sheet | 2 w |
| A3 Tax-code mapping, VAT return, ICP, reconciliation checks | 3 w |
| A4 Bank formats, matching ladder, allocations, unmatched queue | 3 w |
| A5 Accountant package, hardening, pilot migration | 1 w |
| **Total** | **12 w** |

Confidence: medium for A1/A2, low for A3 (rubriek artefacts and edge cases) and
A4 (bank format variation). The bank formats are the classic source of
"one more edge case" — budget the fuzz corpus accordingly.

---

## 14. Decisions

| ADR | Decision | Alternatives rejected |
|-----|----------|------------------------|
| 013 | Append-only double-entry journal; corrections via reversals | Mutable ledger rows, single-entry cashbook |
| 014 | RGS-aligned optional mapping over user-defined accounts | Mandatory RGS, free-form only |
| 015 | VAT return preparation with basis snapshot; no direct filing | Digipoort filing in core |
| 016 | Assisted reconciliation with recorded, reversible matches | Silent auto-allocation |
| 017 | Ledger starts at a configured accounting start date with imported opening balances | Full-history migration, zero start with an unbalanced balance sheet |

---

## 15. Glossary

| Term | Meaning |
|------|---------|
| **Grootboek** | General ledger |
| **Proefbalans** | Trial balance |
| **Dagboek** | Journal (sales, purchase, bank, cash, memorial) |
| **Rubriek** | A numbered box on the Dutch VAT return |
| **Voorbelasting** | Deductible input VAT (rubriek 5b) |
| **Af te dragen btw** | Output VAT payable (rubriek 5a) |
| **ICP-opgaaf** | Intra-EU supplies listing, filed with the VAT return |
| **Openstaande posten** | Open items (unpaid invoices/credits) |
| **Beginbalans** | Opening balance |
| **RGS** | Referentie Grootboek Schema, the Dutch reference chart of accounts |
| **Verleggingsregeling** | Reverse-charge mechanism |
| **Aangifte** | VAT return |
