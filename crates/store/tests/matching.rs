#![allow(clippy::unwrap_used)]
//! Matching payments to open items: what is proposed, what is matched without
//! asking, and what happens to the books when a match is undone.

use crux::{
    AccountCode, Address, CountryCode, Currency, CustomerId, DocType, DocumentId, DraftLine, InvoiceDraft, LineId,
    LocalDate, Money, Party, Quantity, SellerId, TaxTreatment, UnitCode, VatRate,
};
use store::bank::{
    BankAccount, MatchMethod, ParsedStatement, ParsedTransaction, allocate, auto_match, import_statement,
    insert_bank_account, proposals, unmatch,
};
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

    /// Imports a statement holding one payment.
    fn receive_payment(&self, cents: i64, remittance: &str, tag: &str) -> String {
        let statement = ParsedStatement {
            format: "camt053",
            source_name: Some(format!("{tag}.xml")),
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
                counterparty_name: Some("Klant BV".to_owned()),
                counterparty_iban: Some("NL02RABO0123456789".to_owned()),
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

    fn outstanding(&self, kind: OpenItemKind) -> i64 {
        list(self.database.connection(), self.seller, kind)
            .unwrap()
            .iter()
            .filter(|item| item.is_open())
            .map(|item| item.outstanding.micros())
            .sum()
    }

    fn count(&self, sql: &str) -> i64 {
        self.database.connection().query_row(sql, [], |row| row.get(0)).unwrap()
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
}

#[test]
fn a_payment_that_names_its_invoice_is_proposed_as_a_reference_match() {
    let mut books = Books::open();
    books.sell(1_000, "2026-02-10");
    let payment = books.receive_payment(1_210_000_000, "Betaling factuur 2026-0001", "feb");

    let proposals = proposals(books.database.connection(), books.seller, &payment).unwrap();
    assert_eq!(proposals.len(), 1);
    assert_eq!(proposals[0].method, MatchMethod::ExactReference);
    assert_eq!(proposals[0].number, "2026-0001");
    assert_eq!(proposals[0].applies, euros(1_210));
    assert!(
        proposals[0].reason.contains("named in the payment"),
        "{}",
        proposals[0].reason
    );
}

#[test]
fn auto_match_settles_the_item_and_posts_the_payment() {
    let mut books = Books::open();
    books.sell(1_000, "2026-02-10");
    assert_eq!(books.outstanding(OpenItemKind::Receivable), 1_210_000_000);
    books.receive_payment(1_210_000_000, "Betaling factuur 2026-0001", "feb");

    let matched = auto_match(books.database.connection(), books.seller, "ada", NOW).unwrap();
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0].settled, euros(-1_210), "settling is negative");

    // The subledger is clear and the books show the money.
    assert_eq!(books.outstanding(OpenItemKind::Receivable), 0);
    assert_eq!(books.balance("1100"), 1_210_000_000, "bank debited");
    assert_eq!(books.balance("1300"), 0, "debtors cleared");
    assert_eq!(
        books.count("SELECT count(*) FROM bank_transaction WHERE status = 'matched'"),
        1
    );
    assert_eq!(
        books.count("SELECT count(*) FROM audit_event WHERE kind = 'bank_transaction_matched'"),
        1
    );

    // The reconciliation agrees with itself afterwards.
    let ready = store::readiness(books.database.connection(), books.seller, 2026, 2).unwrap();
    assert!(ready.is_ready(), "{:?}", ready.blockers());
}

#[test]
fn matching_twice_does_not_pay_twice() {
    let mut books = Books::open();
    books.sell(1_000, "2026-02-10");
    let payment = books.receive_payment(1_210_000_000, "factuur 2026-0001", "feb");
    auto_match(books.database.connection(), books.seller, "ada", NOW).unwrap();

    // Nothing is left to propose, and a second run does nothing.
    assert!(
        proposals(books.database.connection(), books.seller, &payment)
            .unwrap()
            .is_empty()
    );
    assert!(
        auto_match(books.database.connection(), books.seller, "ada", NOW)
            .unwrap()
            .is_empty()
    );

    let item = list(books.database.connection(), books.seller, OpenItemKind::Receivable).unwrap();
    let error = allocate(
        books.database.connection(),
        books.seller,
        &payment,
        &item[0].id,
        MatchMethod::Manual,
        "ada",
        NOW,
    )
    .unwrap_err();
    assert!(error.to_string().contains("already settled"), "{error}");
    assert_eq!(books.balance("1100"), 1_210_000_000, "still one payment");
}

#[test]
fn an_amount_alone_is_a_candidate_but_never_automatic() {
    let mut books = Books::open();
    books.sell(1_000, "2026-02-10");
    let payment = books.receive_payment(1_210_000_000, "Losse storting", "feb");

    let candidates = proposals(books.database.connection(), books.seller, &payment).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].method, MatchMethod::ExactAmount);
    assert!(
        auto_match(books.database.connection(), books.seller, "ada", NOW)
            .unwrap()
            .is_empty(),
        "an amount without a reference is a question for a person"
    );

    // A person can still match it.
    let report = allocate(
        books.database.connection(),
        books.seller,
        &payment,
        &candidates[0].open_item_id,
        MatchMethod::Manual,
        "ada",
        NOW,
    )
    .unwrap();
    assert_eq!(report.settled, euros(-1_210));
}

#[test]
fn two_equal_amounts_are_not_chosen_between() {
    let mut books = Books::open();
    books.sell(1_000, "2026-02-10");
    books.sell(1_000, "2026-02-11");
    books.receive_payment(1_210_000_000, "Losse storting", "feb");

    let candidates = proposals(books.database.connection(), books.seller, "missing").unwrap_err();
    assert!(candidates.to_string().contains("not found"), "{candidates}");

    let payment: String = books
        .database
        .connection()
        .query_row("SELECT id FROM bank_transaction", [], |row| row.get(0))
        .unwrap();
    let candidates = proposals(books.database.connection(), books.seller, &payment).unwrap();
    assert_eq!(candidates.len(), 2, "both invoices could be the one");
    assert!(
        auto_match(books.database.connection(), books.seller, "ada", NOW)
            .unwrap()
            .is_empty(),
        "a tie is never resolved by guessing"
    );
}

#[test]
fn unmatching_puts_the_item_back_and_keeps_the_history() {
    let mut books = Books::open();
    books.sell(1_000, "2026-02-10");
    books.receive_payment(1_210_000_000, "factuur 2026-0001", "feb");
    let matched = auto_match(books.database.connection(), books.seller, "ada", NOW).unwrap();
    assert_eq!(books.outstanding(OpenItemKind::Receivable), 0);

    unmatch(
        books.database.connection(),
        books.seller,
        &matched[0].allocation_id,
        "ada",
        NOW,
    )
    .unwrap();

    assert_eq!(
        books.outstanding(OpenItemKind::Receivable),
        1_210_000_000,
        "the invoice is open again"
    );
    assert_eq!(
        books.count("SELECT count(*) FROM allocation"),
        2,
        "the match and its undo"
    );
    assert_eq!(
        books.count("SELECT count(*) FROM bank_transaction WHERE status = 'new'"),
        1
    );
    assert_eq!(
        books.count("SELECT count(*) FROM audit_event WHERE kind = 'bank_transaction_unmatched'"),
        1
    );

    // Undoing the same allocation twice is refused.
    let again = unmatch(
        books.database.connection(),
        books.seller,
        &matched[0].allocation_id,
        "ada",
        NOW,
    )
    .unwrap_err();
    assert!(again.to_string().contains("already been undone"), "{again}");
}

#[test]
fn an_invoice_that_is_not_there_cannot_be_matched() {
    let mut books = Books::open();
    books.sell(1_000, "2026-02-10");
    let payment = books.receive_payment(1_210_000_000, "factuur 2026-0001", "feb");

    let error = allocate(
        books.database.connection(),
        books.seller,
        &payment,
        "no-such-item",
        MatchMethod::Manual,
        "ada",
        NOW,
    )
    .unwrap_err();
    assert!(error.to_string().contains("not found"), "{error}");
    let _ = DocumentId::new();
    let _ = DocType::Invoice;
}
