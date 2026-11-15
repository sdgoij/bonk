#![allow(clippy::unwrap_used)]
//! Rules that classify the bank lines no invoice will ever match: what a rule
//! takes, what it must never take, and how a person takes it back.

use crux::{
    AccountCode, Address, CountryCode, Currency, CustomerId, DraftLine, InvoiceDraft, LineId, LocalDate, Money, Party,
    Quantity, SellerId, TaxTreatment, UnitCode, VatRate,
};
use store::bank::{BankAccount, ParsedStatement, ParsedTransaction, import_statement, insert_bank_account};
use store::bank_rule::{self, Classification, RuleDefinition};
use store::open_items::{OpenItemKind, list};
use store::{
    CustomerRecord, Database, IssueRequest, KdfParams, KeyStore, SellerRecord, insert_customer, insert_draft,
    insert_seller, issue_draft, post_outbound_document, seed_starter_chart,
};
use tempfile::TempDir;

const NOW: &str = "2026-11-06T20:00:00.123456Z";
const ACCOUNT: &str = "acct-1";

fn date(raw: &str) -> LocalDate {
    LocalDate::parse_iso(raw).unwrap()
}

fn euros(units: i64) -> Money {
    Money::from_major_units(units, Currency::EUR).unwrap()
}

/// A rule with every matcher unset, for a test to fill in.
fn rule_named(name: &str, account: &str) -> RuleDefinition {
    RuleDefinition {
        name: name.to_owned(),
        direction: None,
        counterparty_iban: None,
        remittance_contains: None,
        counterparty_contains: None,
        account_code: account.to_owned(),
    }
}

struct Books {
    _dir: TempDir,
    database: Database,
    seller: SellerId,
    customer: CustomerId,
}

impl Books {
    fn open() -> Self {
        let dir = TempDir::new().unwrap();
        let keystore = KeyStore::create(dir.path().join("keystore.bin"), "passphrase", KdfParams::TEST).unwrap();
        let database = Database::create(dir.path().join("bonk.db"), &keystore).unwrap();
        seed_starter_chart(database.connection(), NOW).unwrap();

        let seller = SellerId::new();
        insert_seller(
            database.connection(),
            &SellerRecord {
                id: seller,
                party: Party::new("Boeken BV", Address::new("Utrecht", CountryCode::NL)),
                default_currency: Currency::EUR,
                default_locale: "nl-NL".to_owned(),
            },
            NOW,
        )
        .unwrap();
        let customer = CustomerId::new();
        insert_customer(
            database.connection(),
            &CustomerRecord {
                id: customer,
                party: Party::new("Klant BV", Address::new("Amsterdam", CountryCode::NL)),
            },
            NOW,
        )
        .unwrap();
        insert_bank_account(
            database.connection(),
            &BankAccount {
                id: ACCOUNT.to_owned(),
                seller_id: seller,
                name: "Zakelijke rekening".to_owned(),
                iban: Some("NL91ABNA0417164300".to_owned()),
                currency: Currency::EUR,
                ledger_account_code: AccountCode::parse("1100").unwrap(),
                import_profile: None,
            },
            NOW,
        )
        .unwrap();

        Self {
            _dir: dir,
            database,
            seller,
            customer,
        }
    }

    /// Issues and posts an invoice for `net` excluding VAT, returning its number.
    fn sell(&mut self, net: i64, on: &str) -> String {
        let mut draft = InvoiceDraft::new(self.seller, self.customer, Currency::EUR, date(on));
        draft.due_date = Some(date("2026-03-31"));
        draft.lines.push(DraftLine {
            id: LineId::new(),
            position: 1,
            name: "Consulting".to_owned(),
            description: None,
            quantity: Quantity::ONE,
            unit: UnitCode::PIECE,
            unit_price: euros(net),
            price_base_quantity: Quantity::ONE,
            treatment: TaxTreatment::standard(VatRate::STANDARD_NL),
        });
        insert_draft(self.database.connection(), &draft, NOW).unwrap();
        let issued = issue_draft(
            self.database.connection_mut(),
            IssueRequest {
                draft_id: draft.id,
                actor: "ada",
                device: "test",
                renderer_version: "test",
                now: NOW,
            },
            |_| Ok(b"<Invoice/>".to_vec()),
        )
        .unwrap();
        post_outbound_document(self.database.connection(), issued.id, "ada", NOW).unwrap();
        issued.number
    }

    /// Imports a statement holding one line, and returns the transaction.
    fn receive(&self, cents: i64, name: &str, iban: &str, remittance: &str, tag: &str) -> String {
        let statement = ParsedStatement {
            format: "csv",
            source_name: Some(format!("{tag}.csv")),
            currency: Currency::EUR,
            period_start: Some(date("2026-02-01")),
            period_end: Some(date("2026-02-28")),
            opening_balance_micros: None,
            closing_balance_micros: None,
            transactions: vec![ParsedTransaction {
                booking_date: date("2026-02-20"),
                value_date: None,
                amount_micros: cents,
                currency: Currency::EUR,
                counterparty_name: Some(name.to_owned()),
                counterparty_iban: Some(iban.to_owned()),
                remittance_info: Some(remittance.to_owned()),
                external_ref: None,
            }],
        };
        let report = import_statement(
            self.database.connection(),
            ACCOUNT,
            &statement,
            &store::audit::sha256_hex(tag.as_bytes()),
            NOW,
        )
        .unwrap();
        self.database
            .connection()
            .query_row(
                "SELECT id FROM bank_transaction WHERE statement_id = ?1",
                [&report.statement_id],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn add(&self, rule: &RuleDefinition) -> store::bank_rule::BankRule {
        bank_rule::add(self.database.connection(), self.seller, rule, "ada", NOW).unwrap()
    }

    fn classify(&self, dry_run: bool) -> Vec<Classification> {
        bank_rule::classify(self.database.connection(), self.seller, dry_run, "ada", NOW).unwrap()
    }

    fn balance(&self, account: &str) -> i64 {
        self.database
            .connection()
            .query_row(
                "SELECT COALESCE(sum(CASE side WHEN 'debit' THEN amount_micros ELSE -amount_micros END), 0)
                 FROM journal_line WHERE account_code = ?1",
                [account],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn status(&self, transaction: &str) -> String {
        self.database
            .connection()
            .query_row(
                "SELECT status FROM bank_transaction WHERE id = ?1",
                [transaction],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn count(&self, sql: &str) -> i64 {
        self.database.connection().query_row(sql, [], |row| row.get(0)).unwrap()
    }
}

#[test]
fn a_rule_books_a_fee_and_empties_the_queue() {
    let books = Books::open();
    let line = books.receive(
        -1_250_000,
        "Rabobank",
        "NL91ABNA0417164300",
        "Bankkosten februari",
        "feb",
    );
    let mut rule = rule_named("bankkosten", "7100");
    rule.remittance_contains = Some("kosten".to_owned());
    rule.direction = Some(-1);
    books.add(&rule);

    let outcomes = books.classify(false);
    assert_eq!(outcomes.len(), 1);
    match &outcomes[0] {
        Classification::Posted(classified) => {
            assert_eq!(classified.rule, "bankkosten");
            assert_eq!(classified.account_code, AccountCode::parse("7100").unwrap());
            assert_eq!(classified.amount.micros(), 1_250_000, "1.25 EUR out");
            assert!(classified.entry_id.is_some(), "a real run writes the entry");
        }
        other => panic!("expected a posting, got {other:?}"),
    }

    assert_eq!(books.status(&line), "posted");
    assert_eq!(books.balance("7100"), 1_250_000, "the cost is debited");
    assert_eq!(books.balance("1100"), -1_250_000, "the bank is credited");
    assert_eq!(
        books.count("SELECT count(*) FROM journal_entry WHERE source_kind = 'bank'"),
        1
    );
    assert_eq!(
        books.count("SELECT count(*) FROM audit_event WHERE kind = 'bank_transaction_classified'"),
        1
    );

    // Nothing is left for the queue, and a second sweep does nothing.
    assert!(books.classify(false).is_empty());
}

#[test]
fn a_rule_never_takes_a_payment_proposed_for_an_invoice() {
    let mut books = Books::open();
    let number = books.sell(1_000, "2026-02-10");
    let line = books.receive(
        1_210_000_000,
        "Klant BV",
        "NL02RABO0123456789",
        &format!("Betaling factuur {number}"),
        "feb",
    );
    // A rule broad enough to match it: any payment from this customer.
    let mut rule = rule_named("binnenkomend", "7000");
    rule.direction = Some(1);
    rule.counterparty_contains = Some("klant".to_owned());
    books.add(&rule);

    let outcomes = books.classify(false);
    match &outcomes[0] {
        Classification::Reserved {
            transaction_id,
            number: named,
        } => {
            assert_eq!(transaction_id, &line);
            assert_eq!(named, &number);
        }
        other => panic!("a rule may not take a payment, got {other:?}"),
    }
    assert_eq!(books.status(&line), "new", "the line stays where a person can match it");
    assert_eq!(
        books.count("SELECT count(*) FROM journal_entry WHERE source_kind = 'bank'"),
        0
    );
}

#[test]
fn two_rules_that_fit_leave_the_line_in_the_queue() {
    let books = Books::open();
    let line = books.receive(-1_250_000, "Rabobank", "", "Kosten februari", "feb");
    let mut first = rule_named("bankkosten", "7100");
    first.remittance_contains = Some("kosten".to_owned());
    books.add(&first);
    let mut second = rule_named("alle kosten", "7000");
    second.remittance_contains = Some("kosten".to_owned());
    books.add(&second);

    match &books.classify(false)[0] {
        Classification::Ambiguous { rules, .. } => {
            assert_eq!(rules, &vec!["bankkosten".to_owned(), "alle kosten".to_owned()]);
        }
        other => panic!("expected ambiguity, got {other:?}"),
    }
    assert_eq!(books.status(&line), "new");
    assert_eq!(books.count("SELECT count(*) FROM journal_entry"), 0);
}

#[test]
fn a_dry_run_reports_without_posting() {
    let books = Books::open();
    let line = books.receive(-1_250_000, "Rabobank", "", "Bankkosten februari", "feb");
    let mut rule = rule_named("bankkosten", "7100");
    rule.remittance_contains = Some("bankkosten".to_owned());
    books.add(&rule);

    match &books.classify(true)[0] {
        Classification::Posted(classified) => {
            assert!(classified.entry_id.is_none(), "a dry run writes nothing");
        }
        other => panic!("expected a would-be posting, got {other:?}"),
    }
    assert_eq!(books.status(&line), "new");
    assert_eq!(books.count("SELECT count(*) FROM journal_entry"), 0);
    assert_eq!(
        books.count("SELECT count(*) FROM audit_event WHERE kind = 'bank_transaction_classified'"),
        0
    );
}

#[test]
fn an_iban_matches_through_its_grouping_and_case() {
    let books = Books::open();
    books.receive(-500_000, "Energie BV", "NL02 RABO 0123 4567 89", "Voorschot", "feb");
    let mut rule = rule_named("energie", "7000");
    rule.counterparty_iban = Some("nl02rabo0123456789".to_owned());
    books.add(&rule);

    let outcomes = books.classify(false);
    assert!(matches!(outcomes[0], Classification::Posted(_)), "{outcomes:?}");
    assert_eq!(books.balance("7000"), 500_000);
}

#[test]
fn direction_keeps_a_rule_off_the_other_side() {
    let books = Books::open();
    books.receive(500_000, "Rabobank", "", "Rente", "in");
    let mut rule = rule_named("rente ontvangen", "7000");
    rule.remittance_contains = Some("rente".to_owned());
    rule.direction = Some(-1);
    books.add(&rule);

    assert!(matches!(books.classify(false)[0], Classification::NoRule { .. }));
    assert_eq!(books.balance("7000"), 0);

    let mut other = rule_named("rente ontvangen", "7000");
    other.remittance_contains = Some("rente".to_owned());
    other.direction = Some(1);
    // The name is taken, so remove the wrong one first.
    bank_rule::remove(books.database.connection(), books.seller, "rente ontvangen", "ada", NOW).unwrap();
    books.add(&other);

    assert!(matches!(books.classify(false)[0], Classification::Posted(_)));
    assert_eq!(books.balance("7000"), -500_000, "interest is credited");
}

#[test]
fn unclassifying_reverses_the_entry_and_returns_the_line() {
    let books = Books::open();
    let line = books.receive(-1_250_000, "Rabobank", "", "Bankkosten februari", "feb");
    let mut rule = rule_named("bankkosten", "7100");
    rule.remittance_contains = Some("bankkosten".to_owned());
    books.add(&rule);
    books.classify(false);
    assert_eq!(books.status(&line), "posted");

    let undone = bank_rule::unclassify(
        books.database.connection(),
        books.seller,
        &line,
        "verkeerde rekening",
        "ada",
        NOW,
    )
    .unwrap();

    assert_eq!(books.status(&line), "new");
    assert_eq!(books.balance("7100"), 0, "the cost is gone");
    assert_eq!(books.balance("1100"), 0, "the bank is back");
    assert_eq!(
        books.count("SELECT count(*) FROM audit_event WHERE kind = 'bank_transaction_unclassified'"),
        1
    );
    let reversed: Option<String> = books
        .database
        .connection()
        .query_row(
            "SELECT reverses_entry_id FROM journal_entry WHERE id = ?1",
            [undone.reversal_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(reversed, Some(undone.entry_id.to_string()));

    // The line is in the queue again and the rule would take it again, which is
    // why the correction starts by removing the rule.
    assert!(matches!(books.classify(true)[0], Classification::Posted(_)));
}

#[test]
fn a_matched_line_is_corrected_by_unmatching() {
    let mut books = Books::open();
    let number = books.sell(1_000, "2026-02-10");
    let line = books.receive(
        1_210_000_000,
        "Klant BV",
        "NL02RABO0123456789",
        &format!("factuur {number}"),
        "feb",
    );
    store::bank::auto_match(books.database.connection(), books.seller, "ada", NOW).unwrap();

    let error = bank_rule::unclassify(books.database.connection(), books.seller, &line, "x", "ada", NOW).unwrap_err();
    assert!(error.to_string().contains("bank unmatch"), "{error}");
}

#[test]
fn a_rule_without_a_matcher_is_refused() {
    let books = Books::open();
    let error = bank_rule::add(
        books.database.connection(),
        books.seller,
        &rule_named("leeg", "7100"),
        "ada",
        NOW,
    )
    .unwrap_err();
    assert!(error.to_string().contains("at least one matcher"), "{error}");
}

#[test]
fn a_second_rule_of_the_same_name_is_refused() {
    let books = Books::open();
    let mut rule = rule_named("bankkosten", "7100");
    rule.remittance_contains = Some("kosten".to_owned());
    books.add(&rule);

    let error = bank_rule::add(books.database.connection(), books.seller, &rule, "ada", NOW).unwrap_err();
    assert!(error.to_string().contains("already exists"), "{error}");
}

#[test]
fn a_rule_naming_an_unknown_account_is_refused() {
    let books = Books::open();
    let mut rule = rule_named("bankkosten", "1234");
    rule.remittance_contains = Some("kosten".to_owned());
    let error = bank_rule::add(books.database.connection(), books.seller, &rule, "ada", NOW).unwrap_err();
    assert!(error.to_string().contains("1234"), "{error}");
}

#[test]
fn a_line_no_rule_fits_stays_in_the_queue() {
    let books = Books::open();
    let line = books.receive(-1_250_000, "Rabobank", "", "Iets anders", "feb");
    let mut rule = rule_named("bankkosten", "7100");
    rule.remittance_contains = Some("kosten".to_owned());
    books.add(&rule);

    assert!(matches!(books.classify(false)[0], Classification::NoRule { .. }));
    assert_eq!(books.status(&line), "new");
    assert_eq!(books.count("SELECT count(*) FROM journal_entry"), 0);
}

#[test]
fn a_rule_does_not_touch_a_line_that_is_already_posted() {
    let books = Books::open();
    let line = books.receive(-1_250_000, "Rabobank", "", "Bankkosten februari", "feb");
    let mut rule = rule_named("bankkosten", "7100");
    rule.remittance_contains = Some("bankkosten".to_owned());
    books.add(&rule);
    books.classify(false);

    let error =
        bank_rule::classify_one(books.database.connection(), books.seller, &line, false, "ada", NOW).unwrap_err();
    assert!(error.to_string().contains("only a line waiting"), "{error}");
    assert_eq!(books.balance("7100"), 1_250_000, "still one posting");
}

#[test]
fn the_rules_are_listed_in_the_order_they_were_written() {
    let books = Books::open();
    for (name, account) in [("eerste", "7100"), ("tweede", "7000"), ("derde", "9999")] {
        let mut rule = rule_named(name, account);
        rule.remittance_contains = Some(name.to_owned());
        books.add(&rule);
    }

    let rules = bank_rule::list(books.database.connection(), books.seller).unwrap();
    let names: Vec<&str> = rules.iter().map(|rule| rule.name.as_str()).collect();
    assert_eq!(names, vec!["eerste", "tweede", "derde"]);
    assert_eq!(rules[0].position, 1);
}

#[test]
fn the_subledger_is_untouched_by_a_classification() {
    let mut books = Books::open();
    books.sell(1_000, "2026-02-10");
    let before = list(books.database.connection(), books.seller, OpenItemKind::Receivable)
        .unwrap()
        .len();
    books.receive(-1_250_000, "Rabobank", "", "Bankkosten februari", "feb");
    let mut rule = rule_named("bankkosten", "7100");
    rule.remittance_contains = Some("bankkosten".to_owned());
    books.add(&rule);
    books.classify(false);

    let after = list(books.database.connection(), books.seller, OpenItemKind::Receivable)
        .unwrap()
        .len();
    assert_eq!(before, after, "a classified line never creates or settles an item");
    assert_eq!(
        books.count("SELECT count(*) FROM allocation"),
        0,
        "and never writes an allocation"
    );
}
