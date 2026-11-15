#![allow(clippy::unwrap_used)]
//! The account a purchase's cost lands on: choosing it, and the close that refuses
//! to happen while costs are still unplaced.

use crux::{
    AccountCode, Address, CountryCode, Currency, CustomerId, DocType, DocumentId, LocalDate, Money, Party, SellerId,
    TaxTreatment, VatRate,
};
use store::{
    CustomerRecord, Database, KdfParams, KeyStore, SellerRecord, insert_customer, insert_seller, seed_starter_chart,
};
use tempfile::TempDir;

const NOW: &str = "2026-11-06T19:00:00.123456Z";

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
                party: Party::new("Leverancier BV", Address::new("Rotterdam", CountryCode::NL)),
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

    /// Records a received invoice, unposted, so an account can be chosen for it.
    fn buy(&self, net_units: i64, on: &str, number: &str) -> DocumentId {
        let treatment = TaxTreatment::standard(VatRate::STANDARD_NL);
        let net = euros(net_units);
        let tax = crux::tax_amount(net, treatment.rate).unwrap();
        let totals = crux::InvoiceTotals {
            net,
            tax,
            gross: net.checked_add(tax).unwrap(),
            breakdown: vec![crux::TaxBreakdown {
                category: treatment.category,
                rate_bp: treatment.rate.basis_points(),
                taxable_base: net,
                tax,
            }],
        };
        let issuer = Party::new("Leverancier BV", Address::new("Rotterdam", CountryCode::NL));
        let seller = store::load_seller(self.database.connection(), self.seller).unwrap();
        let received = store::ReceivedDocument {
            number,
            doc_type: DocType::Invoice,
            currency: Currency::EUR,
            issue_date: date(on),
            due_date: None,
            delivery_date: None,
            buyer_reference: None,
            order_reference: None,
            note: None,
            issuer: &issuer,
            recipient: &seller.party,
            lines: &[],
            totals: &totals,
            preceding_number: None,
            preceding_issue_date: None,
        };
        let payload = format!("<Invoice>{number}</Invoice>");
        store::record_received(
            self.database.connection(),
            self.seller,
            Some(self.customer),
            &received,
            &store::ImportSource {
                kind: "ubl",
                path: None,
                payload_hash: &store::audit::sha256_hex(payload.as_bytes()),
                payload: payload.as_bytes(),
            },
            None,
            NOW,
        )
        .unwrap();
        let id: String = self
            .database
            .connection()
            .query_row(
                "SELECT id FROM document WHERE direction = 'inbound' AND number = ?1",
                [number],
                |row| row.get(0),
            )
            .unwrap();
        DocumentId::parse_str(&id).unwrap()
    }

    /// The balance of an account, debit positive.
    fn balance(&self, code: &str) -> i64 {
        self.database
            .connection()
            .query_row(
                "SELECT COALESCE(sum(CASE side WHEN 'debit' THEN amount_micros ELSE -amount_micros END), 0)
                 FROM journal_line WHERE account_code = ?1",
                [code],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn period(&self, month: u8) -> crux::FiscalPeriodId {
        let id: String = self
            .database
            .connection()
            .query_row(
                "SELECT id FROM fiscal_period WHERE year = 2026 AND period_no = ?1",
                [month],
                |row| row.get(0),
            )
            .unwrap();
        crux::FiscalPeriodId::parse_str(&id).unwrap()
    }
}

#[test]
fn the_cost_of_a_purchase_lands_on_the_account_that_was_chosen() {
    let books = Books::open();
    let document = books.buy(1_000, "2026-10-10", "L-1");
    store::expense::assign(books.database.connection(), document, "7100", "ada", NOW).unwrap();
    store::post_document(books.database.connection(), document, "ada", NOW).unwrap();

    assert_eq!(books.balance("7100"), 1_000_000_000, "the cost is on the account");
    assert_eq!(books.balance("9999"), 0, "nothing landed on the unclassified account");
    assert_eq!(
        books.balance("1400"),
        -1_210_000_000,
        "and the creditor still owes the lot"
    );
    assert_eq!(
        store::expense::chosen(books.database.connection(), document).unwrap(),
        Some(AccountCode::parse("7100").unwrap())
    );
}

#[test]
fn without_a_choice_the_cost_lands_on_the_unclassified_account() {
    let books = Books::open();
    let document = books.buy(1_000, "2026-10-10", "L-1");
    store::post_document(books.database.connection(), document, "ada", NOW).unwrap();

    assert_eq!(books.balance("9999"), 1_000_000_000);
    assert_eq!(books.balance("7100"), 0);
    assert_eq!(
        store::expense::chosen(books.database.connection(), document).unwrap(),
        None
    );
}

#[test]
fn a_posted_document_keeps_the_account_it_was_posted_with() {
    let books = Books::open();
    let document = books.buy(1_000, "2026-10-10", "L-1");
    store::post_document(books.database.connection(), document, "ada", NOW).unwrap();

    let error = store::expense::assign(books.database.connection(), document, "7100", "ada", NOW).unwrap_err();
    assert!(error.to_string().contains("already posted"), "{error}");
    assert_eq!(books.balance("9999"), 1_000_000_000, "nothing moved");
}

#[test]
fn only_an_expense_account_that_takes_postings_can_be_chosen() {
    let books = Books::open();
    let document = books.buy(1_000, "2026-10-10", "L-1");

    let refused = |code: &str| {
        store::expense::assign(books.database.connection(), document, code, "ada", NOW)
            .map(|_| ())
            .unwrap_err()
            .to_string()
    };
    assert!(refused("8000").contains("revenue"), "{}", refused("8000"));
    assert!(refused("1300").contains("subledger"), "{}", refused("1300"));
    assert!(refused("9999").contains("clear the choice"), "{}", refused("9999"));
    assert!(refused("1234").contains("1234"), "the account is named");

    // An expense account is the whole point, though.
    store::expense::assign(books.database.connection(), document, "7000", "ada", NOW)
        .expect("an expense account takes the choice");
}

#[test]
fn clearing_returns_the_document_to_the_default() {
    let books = Books::open();
    let document = books.buy(1_000, "2026-10-10", "L-1");
    store::expense::assign(books.database.connection(), document, "7100", "ada", NOW).unwrap();
    store::expense::clear(books.database.connection(), document, "ada", NOW).unwrap();
    assert_eq!(
        store::expense::chosen(books.database.connection(), document).unwrap(),
        None
    );

    store::post_document(books.database.connection(), document, "ada", NOW).unwrap();
    assert_eq!(books.balance("9999"), 1_000_000_000);
}

#[test]
fn the_period_cannot_close_while_the_unclassified_account_holds_a_balance() {
    let books = Books::open();
    let document = books.buy(1_000, "2026-10-10", "L-1");
    store::post_document(books.database.connection(), document, "ada", NOW).unwrap();

    let error = store::lock_period(books.database.connection(), books.period(10), "ada", NOW).unwrap_err();
    let said = error.to_string();
    assert!(said.contains("9999"), "{said}");
    assert!(said.contains("1000 EUR"), "the base that landed there is named: {said}");
    assert!(
        said.contains("memorial entry"),
        "and the only remedy that exists is named: {said}"
    );
}

#[test]
fn a_chosen_account_lets_the_period_close() {
    let books = Books::open();
    let document = books.buy(1_000, "2026-10-10", "L-1");
    store::expense::assign(books.database.connection(), document, "7100", "ada", NOW).unwrap();
    store::post_document(books.database.connection(), document, "ada", NOW).unwrap();

    store::lock_period(books.database.connection(), books.period(10), "ada", NOW).unwrap();
}

/// The escape for a purchase that was already posted unclassified.
///
/// The document's own entry is append-only, so the cost is moved with a memorial
/// entry dated in the period — which is what the refusal tells a person to do, so
/// it has to work.
#[test]
fn a_memorial_correction_lets_a_period_with_unclassified_costs_close() {
    let books = Books::open();
    let document = books.buy(1_000, "2026-10-10", "L-1");
    store::post_document(books.database.connection(), document, "ada", NOW).unwrap();
    assert!(store::lock_period(books.database.connection(), books.period(10), "ada", NOW).is_err());

    store::post_manual_entry(
        books.database.connection(),
        books.seller,
        date("2026-10-31"),
        "Kosten op de juiste rekening",
        vec![
            crux::LedgerLine::debit(AccountCode::parse("7100").unwrap(), euros(1_000)).unwrap(),
            crux::LedgerLine::credit(AccountCode::parse("9999").unwrap(), euros(1_000)).unwrap(),
        ],
        "ada",
        NOW,
    )
    .unwrap();

    assert_eq!(books.balance("9999"), 0, "the account is empty again");
    assert_eq!(books.balance("7100"), 1_000_000_000);
    store::lock_period(books.database.connection(), books.period(10), "ada", NOW).unwrap();
}

#[test]
fn clearing_a_document_without_a_choice_is_named() {
    let books = Books::open();
    let error = store::expense::clear(
        books.database.connection(),
        books.buy(1_000, "2026-10-11", "L-2"),
        "ada",
        NOW,
    )
    .unwrap_err();
    assert!(error.to_string().contains("no account assigned"), "{error}");
}
