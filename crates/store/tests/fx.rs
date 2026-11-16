#![allow(clippy::unwrap_used)]
//! Foreign-currency postings: a document keeps its own currency, the ledger
//! keeps the reporting currency, and the rate travels with the posting.

use crux::{
    AccountKind, Address, CountryCode, Currency, CustomerId, DocType, DraftLine, ExchangeRate, InvoiceDraft, LineId,
    LocalDate, Money, Party, Quantity, SellerId, Side, TaxBreakdown, TaxTreatment, UnitCode, VatCategory, VatRate,
};
use store::bank::{
    BankAccount, MatchMethod, ParsedStatement, ParsedTransaction, allocate, allocate_group, import_statement,
    insert_bank_account, unmatch,
};
use store::open_items::{OpenItemKind, list};
use store::{
    CustomerRecord, Database, ImportSource, IssueRequest, KdfParams, KeyStore, ReceivedDocument, SellerRecord,
    insert_customer, insert_draft, insert_seller, issue_draft, load_seller, post_document, post_outbound_document,
    record_received, seed_starter_chart,
};
use tempfile::TempDir;

const NOW: &str = "2026-11-16T12:00:00.000000Z";
const ACCOUNT: &str = "acct-1";

fn date(raw: &str) -> LocalDate {
    LocalDate::parse_iso(raw).unwrap()
}

fn eur_micros(micros: i64) -> Money {
    Money::from_micros(micros, Currency::EUR)
}

fn gbp_micros(micros: i64) -> Money {
    Money::from_micros(micros, Currency::GBP)
}

fn pounds(units: i64) -> Money {
    Money::from_major_units(units, Currency::GBP).unwrap()
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
                ledger_account_code: crux::AccountCode::parse("1100").unwrap(),
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

    fn rate(&self, currency: Currency, as_of: &str, micros: i64) {
        store::set_manual_rate(
            self.database.connection(),
            self.seller,
            currency,
            date(as_of),
            ExchangeRate::from_per_unit_micros(micros).unwrap(),
            "ada",
            NOW,
        )
        .unwrap();
    }

    fn issue(&mut self, currency: Currency, net_units: i64, on: &str, due: &str) -> crux::DocumentId {
        let mut draft = InvoiceDraft::new(self.seller, self.customer, currency, date(on));
        draft.due_date = Some(date(due));
        draft.lines.push(DraftLine {
            id: LineId::new(),
            position: 1,
            name: "Consulting".to_owned(),
            description: None,
            quantity: Quantity::ONE,
            unit: UnitCode::PIECE,
            unit_price: Money::from_major_units(net_units, currency).unwrap(),
            price_base_quantity: Quantity::ONE,
            treatment: TaxTreatment::standard(VatRate::STANDARD_NL),
        });
        insert_draft(self.database.connection(), &draft, NOW).unwrap();
        issue_draft(
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
        .unwrap()
        .id
    }

    /// Imports a statement holding one euro payment on a date.
    fn receive_payment(&self, micros: i64, on: &str, remittance: &str, tag: &str) -> String {
        let statement = ParsedStatement {
            format: "camt053",
            source_name: Some(format!("{tag}.xml")),
            currency: Currency::EUR,
            period_start: Some(date(on)),
            period_end: Some(date(on)),
            opening_balance_micros: None,
            closing_balance_micros: None,
            transactions: vec![ParsedTransaction {
                booking_date: date(on),
                value_date: None,
                amount_micros: micros,
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

    /// Imports a received invoice in `currency` and posts it.
    fn receive(&mut self, currency: Currency, net_units: i64, on: &str, number: &str) -> crux::DocumentId {
        let net = Money::from_major_units(net_units, currency).unwrap();
        let tax = crux::tax_amount(net, VatRate::STANDARD_NL).unwrap();
        let totals = crux::InvoiceTotals {
            net,
            tax,
            gross: net.checked_add(tax).unwrap(),
            breakdown: vec![TaxBreakdown {
                category: VatCategory::Standard,
                rate_bp: 2100,
                taxable_base: net,
                tax,
            }],
        };
        let issuer = Party::new("Leverancier BV", Address::new("Rotterdam", CountryCode::NL));
        let seller = load_seller(self.database.connection(), self.seller).unwrap();
        let received = ReceivedDocument {
            number,
            doc_type: DocType::Invoice,
            currency,
            issue_date: date(on),
            due_date: Some(date("2026-02-09")),
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
        record_received(
            self.database.connection(),
            self.seller,
            Some(self.customer),
            &received,
            &ImportSource {
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
        let id = crux::DocumentId::parse_str(&id).unwrap();
        post_document(self.database.connection(), id, "ada", NOW).unwrap();
        id
    }

    fn payable(&self) -> store::OpenItem {
        list(self.database.connection(), self.seller, OpenItemKind::Payable)
            .unwrap()
            .into_iter()
            .next()
            .expect("an open payable")
    }

    fn receivable(&self) -> store::OpenItem {
        list(self.database.connection(), self.seller, OpenItemKind::Receivable)
            .unwrap()
            .into_iter()
            .next()
            .expect("an open receivable")
    }
}

#[test]
fn a_foreign_invoice_posts_in_both_currencies() {
    let mut books = Books::open();
    books.rate(Currency::GBP, "2026-01-01", 1_170_000);
    let invoice = books.issue(Currency::GBP, 1_000, "2026-01-10", "2026-02-09");
    let posted = post_outbound_document(books.database.connection(), invoice, "ada", NOW).unwrap();

    let stored = store::load_entry(books.database.connection(), posted.id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.entry.currency(), Currency::GBP);
    let fx = stored.entry.fx().expect("the entry names its rate");
    assert_eq!(fx.base_currency, Currency::EUR);
    assert_eq!(fx.rate.per_unit_micros(), 1_170_000);

    // The debtor is the gross in the reporting currency; revenue and VAT keep
    // their historical amounts.
    let balance = store::trial_balance(
        books.database.connection(),
        books.seller,
        date("2026-01-01"),
        date("2026-12-31"),
    )
    .unwrap();
    assert!(balance.balances(), "the reporting-currency columns must agree");
    let debtor = balance.rows.iter().find(|row| row.code.as_str() == "1300").unwrap();
    assert_eq!(debtor.balance_micros(), 1_415_700_000, "£1 210 at 1.17");
    let revenue = balance.rows.iter().find(|row| row.code.as_str() == "8000").unwrap();
    assert_eq!(revenue.balance_micros(), 1_170_000_000);
    let vat = balance.rows.iter().find(|row| row.code.as_str() == "1600").unwrap();
    assert_eq!(vat.balance_micros(), 245_700_000);

    // The subledger keeps the document currency and snapshots the carrying value.
    let items = list(books.database.connection(), books.seller, OpenItemKind::Receivable).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].amount, pounds(1_210));
    assert_eq!(items[0].base_amount, eur_micros(1_415_700_000));
    assert_eq!(items[0].fx_rate_micros, Some(1_170_000));
}

#[test]
fn posting_without_a_rate_is_refused_and_names_the_fix() {
    let mut books = Books::open();
    let invoice = books.issue(Currency::GBP, 100, "2026-01-10", "2026-02-09");
    let error = post_outbound_document(books.database.connection(), invoice, "ada", NOW).unwrap_err();
    let text = error.to_string();
    assert!(text.contains("no rate"), "{text}");
    assert!(text.contains("fx import"), "{text}");
    assert!(text.contains("fx override"), "{text}");
}

#[test]
fn an_override_wins_over_the_cache_and_is_part_of_the_posting() {
    let mut books = Books::open();
    books.rate(Currency::GBP, "2026-01-01", 1_170_000);
    let invoice = books.issue(Currency::GBP, 1_000, "2026-01-10", "2026-02-09");
    store::set_override(
        books.database.connection(),
        books.seller,
        invoice,
        ExchangeRate::from_per_unit_micros(1_200_000).unwrap(),
        date("2026-01-10"),
        "ada",
        NOW,
    )
    .unwrap();
    let posted = post_outbound_document(books.database.connection(), invoice, "ada", NOW).unwrap();
    let stored = store::load_entry(books.database.connection(), posted.id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.entry.fx().unwrap().rate.per_unit_micros(), 1_200_000);

    // An override cannot be changed after the posting: the entry is append-only.
    let refused = store::set_override(
        books.database.connection(),
        books.seller,
        invoice,
        ExchangeRate::from_per_unit_micros(1_300_000).unwrap(),
        date("2026-01-10"),
        "ada",
        NOW,
    )
    .unwrap_err();
    assert!(refused.to_string().contains("posted"), "{refused}");
}

#[test]
fn a_reversal_keeps_the_rate_and_nets_the_books_to_zero() {
    let mut books = Books::open();
    books.rate(Currency::GBP, "2026-01-01", 1_170_000);
    let invoice = books.issue(Currency::GBP, 1_000, "2026-01-10", "2026-02-09");
    let posted = post_outbound_document(books.database.connection(), invoice, "ada", NOW).unwrap();
    let reversal = store::reverse_entry(
        books.database.connection(),
        posted.id,
        "wrong customer",
        date("2026-01-31"),
        "ada",
        NOW,
    )
    .unwrap();

    let original_fx = store::load_entry(books.database.connection(), posted.id)
        .unwrap()
        .unwrap()
        .entry
        .fx()
        .unwrap();
    let stored = store::load_entry(books.database.connection(), reversal.id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.entry.fx(), Some(original_fx));
    let balance = store::trial_balance(
        books.database.connection(),
        books.seller,
        date("2026-01-01"),
        date("2026-12-31"),
    )
    .unwrap();
    assert!(balance.balances());
    assert!(
        balance.rows.iter().all(|row| row.balance_micros() == 0),
        "{:?}",
        balance.rows
    );
}

#[test]
fn an_ecb_file_is_crossed_into_the_reporting_currency() {
    let books = Books::open();
    let file = b"Date,USD,GBP\n2026-01-02,1.08,0.85\n2026-01-03,1.09,0.86\n";
    let report = store::import_ecb(books.database.connection(), books.seller, file, "ada", NOW).unwrap();
    assert_eq!(report.rows, 2);
    assert_eq!(report.currencies, 2);
    assert_eq!(report.provider_version.len(), 64);

    // 1 / 1.08 = 0.9259259..., rounded once by the ledger's money rule; by the
    // third the file quotes 1.09, which the later dates take.
    let dollar = store::rate_on(
        books.database.connection(),
        books.seller,
        Currency::USD,
        date("2026-01-02"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(dollar.per_unit_micros(), 925_926);
    let latest = store::rate_on(
        books.database.connection(),
        books.seller,
        Currency::USD,
        date("2026-01-05"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(latest.per_unit_micros(), 917_431);
    let pound = store::rate_on(
        books.database.connection(),
        books.seller,
        Currency::GBP,
        date("2026-01-03"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(pound.per_unit_micros(), 1_162_791);

    // A weekend takes the last published reference rate; before the file starts
    // there is no rate at all.
    assert_eq!(
        store::rate_on(
            books.database.connection(),
            books.seller,
            Currency::USD,
            date("2026-01-04")
        )
        .unwrap()
        .unwrap()
        .per_unit_micros(),
        917_431
    );
    assert!(
        store::rate_on(
            books.database.connection(),
            books.seller,
            Currency::USD,
            date("2026-01-01")
        )
        .unwrap()
        .is_none()
    );
    assert_eq!(
        store::list_rates(books.database.connection(), books.seller, None)
            .unwrap()
            .len(),
        4
    );
}

#[test]
fn a_books_reporting_outside_the_euro_gets_its_euro_leg() {
    let books = Books::open();
    let other = SellerId::new();
    insert_seller(
        books.database.connection(),
        &SellerRecord {
            id: other,
            party: Party::new("Dollar Co", Address::new("Boston", CountryCode::parse("US").unwrap())),
            default_currency: Currency::USD,
            default_locale: "en-US".to_owned(),
        },
        NOW,
    )
    .unwrap();
    let file = b"Date,GBP,USD\n2026-01-02,0.85,1.08\n";
    store::import_ecb(books.database.connection(), other, file, "ada", NOW).unwrap();

    // USD per GBP = 1.08 / 0.85; USD per EUR = 1.08, because the file has no
    // euro column: the euro is its unit.
    let pound = store::rate_on(books.database.connection(), other, Currency::GBP, date("2026-01-03"))
        .unwrap()
        .unwrap();
    assert_eq!(pound.per_unit_micros(), 1_270_588);
    let euro = store::rate_on(books.database.connection(), other, Currency::EUR, date("2026-01-03"))
        .unwrap()
        .unwrap();
    assert_eq!(euro.per_unit_micros(), 1_080_000);
    let dollar = store::rate_on(books.database.connection(), other, Currency::USD, date("2026-01-03"))
        .unwrap()
        .unwrap();
    assert_eq!(dollar.per_unit_micros(), 1_000_000);
}

#[test]
fn a_posted_foreign_document_verifies_offline() {
    let mut books = Books::open();
    books.rate(Currency::GBP, "2026-01-01", 1_170_000);
    let invoice = books.issue(Currency::GBP, 1_000, "2026-01-10", "2026-02-09");
    post_outbound_document(books.database.connection(), invoice, "ada", NOW).unwrap();

    let bundle = TempDir::new().unwrap();
    store::evidence::export(
        books.database.connection(),
        books.seller,
        invoice,
        bundle.path(),
        "ada",
        NOW,
    )
    .unwrap();
    let report = store::evidence::verify(bundle.path()).unwrap();
    assert_eq!(report.ledger_entries, 1);
}

#[test]
fn a_rebuild_uses_the_rate_the_posting_used() {
    let mut books = Books::open();
    books.rate(Currency::GBP, "2026-01-01", 1_170_000);
    let invoice = books.issue(Currency::GBP, 1_000, "2026-01-10", "2026-02-09");
    post_outbound_document(books.database.connection(), invoice, "ada", NOW).unwrap();

    // An older archive has the entry and no subledger row. The cache then moves,
    // but a repair must reproduce the posting's rate rather than today's.
    let connection = books.database.connection();
    connection.execute("DROP TRIGGER open_item_no_delete", []).unwrap();
    connection.execute("DELETE FROM open_item", []).unwrap();
    books.rate(Currency::GBP, "2026-01-11", 1_300_000);

    let report = store::rebuild_open_items(connection, books.seller, "ada", NOW).unwrap();
    assert_eq!(report.added, 1);
    let items = list(connection, books.seller, OpenItemKind::Receivable).unwrap();
    assert_eq!(items[0].fx_rate_micros, Some(1_170_000));
    assert_eq!(items[0].base_amount, eur_micros(1_415_700_000));
}

#[test]
fn the_control_account_check_uses_the_carrying_value() {
    let mut books = Books::open();
    books.rate(Currency::GBP, "2026-01-01", 1_170_000);
    let invoice = books.issue(Currency::GBP, 1_000, "2026-01-10", "2026-02-09");
    post_outbound_document(books.database.connection(), invoice, "ada", NOW).unwrap();

    let readiness = store::readiness(books.database.connection(), books.seller, 2026, 1).unwrap();
    assert!(
        matches!(readiness.control_accounts, store::Check::Passed),
        "{:?}",
        readiness.control_accounts
    );
    assert!(
        matches!(readiness.invoice_to_ledger, store::Check::Passed),
        "the document's VAT and the posted VAT are compared after conversion: {:?}",
        readiness.invoice_to_ledger
    );
    assert!(
        matches!(readiness.ledger_to_return, store::Check::Passed),
        "{:?}",
        readiness.ledger_to_return
    );
    assert!(
        matches!(readiness.balance_sheet, store::Check::Passed),
        "{:?}",
        readiness.balance_sheet
    );
    assert_eq!(
        store::load_account(books.database.connection(), &crux::AccountCode::parse("1300").unwrap())
            .unwrap()
            .unwrap()
            .kind,
        AccountKind::Asset
    );
}

/// The line on `account`, which these entries keep at one line per account.
fn line<'a>(entry: &'a crux::LedgerEntry, account: &str) -> &'a crux::LedgerLine {
    entry
        .lines()
        .iter()
        .find(|line| line.account().as_str() == account)
        .unwrap_or_else(|| panic!("no line on {account}: {:?}", entry.lines()))
}

#[test]
fn a_foreign_invoice_settles_at_the_payment_date_rate() {
    let mut books = Books::open();
    books.rate(Currency::GBP, "2026-01-01", 1_170_000);
    books.rate(Currency::GBP, "2026-02-01", 1_250_000);
    let invoice = books.issue(Currency::GBP, 1_000, "2026-01-10", "2026-02-09");
    post_outbound_document(books.database.connection(), invoice, "ada", NOW).unwrap();

    let item = books.receivable();
    let payment = books.receive_payment(1_512_500_000, "2026-02-20", "Betaling 2026-0001", "feb");
    let report = allocate(
        books.database.connection(),
        books.seller,
        &payment,
        &item.id,
        MatchMethod::Manual,
        "ada",
        NOW,
    )
    .unwrap();

    // €1 512.50 received clears £1 210 carried at 1.17 (€1 415.70) and leaves a
    // realised gain of €96.80.
    assert_eq!(report.settled, eur_micros(-1_512_500_000));
    assert_eq!(report.settled_base, eur_micros(-1_415_700_000));
    assert_eq!(report.fx_difference, eur_micros(96_800_000));

    let stored = store::load_entry(books.database.connection(), report.entry_id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.entry.currency(), Currency::EUR);
    assert_eq!(stored.entry.lines().len(), 3);
    assert_eq!(line(&stored.entry, "1100").side(), Side::Debit);
    assert_eq!(line(&stored.entry, "1100").amount(), eur_micros(1_512_500_000));
    assert_eq!(line(&stored.entry, "1300").side(), Side::Credit);
    assert_eq!(line(&stored.entry, "1300").amount(), eur_micros(1_415_700_000));
    assert_eq!(line(&stored.entry, "7900").side(), Side::Credit);
    assert_eq!(line(&stored.entry, "7900").amount(), eur_micros(96_800_000));

    // The allocation carries all four quantities: document, payment, carrying
    // value and the realised result.
    let (amount, transaction, base, difference): (i64, i64, i64, i64) = books
        .database
        .connection()
        .query_row(
            "SELECT amount_micros, transaction_amount_micros, base_amount_micros, fx_difference_micros
             FROM allocation WHERE id = ?1",
            [&report.allocation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(amount, -1_210_000_000, "£1 210 in the item's currency");
    assert_eq!(transaction, -1_512_500_000, "the euro payment applied");
    assert_eq!(base, -1_415_700_000, "the carrying value released");
    assert_eq!(difference, 96_800_000, "the realised gain");

    let settled = list(books.database.connection(), books.seller, OpenItemKind::Receivable).unwrap();
    assert_eq!(settled[0].outstanding, gbp_micros(0));
    assert!(!settled[0].is_open());

    let balance = store::trial_balance(
        books.database.connection(),
        books.seller,
        date("2026-01-01"),
        date("2026-12-31"),
    )
    .unwrap();
    assert!(balance.balances());
    let difference_account = balance.rows.iter().find(|row| row.code.as_str() == "7900").unwrap();
    assert_eq!(
        difference_account.balance_micros(),
        -96_800_000,
        "a gain credits the exchange-difference expense account"
    );

    let readiness = store::readiness(books.database.connection(), books.seller, 2026, 2).unwrap();
    assert!(
        matches!(readiness.control_accounts, store::Check::Passed),
        "the control account and the subledger agree after settlement: {:?}",
        readiness.control_accounts
    );
    assert!(matches!(readiness.balance_sheet, store::Check::Passed));
}

#[test]
fn a_falling_rate_posts_a_realised_loss() {
    let mut books = Books::open();
    books.rate(Currency::GBP, "2026-01-01", 1_170_000);
    books.rate(Currency::GBP, "2026-02-01", 1_100_000);
    let invoice = books.issue(Currency::GBP, 1_000, "2026-01-10", "2026-02-09");
    post_outbound_document(books.database.connection(), invoice, "ada", NOW).unwrap();

    let item = books.receivable();
    let payment = books.receive_payment(1_331_000_000, "2026-02-20", "Betaling 2026-0001", "feb");
    let report = allocate(
        books.database.connection(),
        books.seller,
        &payment,
        &item.id,
        MatchMethod::Manual,
        "ada",
        NOW,
    )
    .unwrap();

    assert_eq!(report.settled, eur_micros(-1_331_000_000));
    assert_eq!(report.settled_base, eur_micros(-1_415_700_000));
    assert_eq!(report.fx_difference, eur_micros(-84_700_000));

    let stored = store::load_entry(books.database.connection(), report.entry_id)
        .unwrap()
        .unwrap();
    assert_eq!(line(&stored.entry, "7900").side(), Side::Debit);
    assert_eq!(line(&stored.entry, "7900").amount(), eur_micros(84_700_000));
    assert_eq!(stored.entry.lines().len(), 3);
}

#[test]
fn a_partial_foreign_settlement_leaves_the_rest_open() {
    let mut books = Books::open();
    books.rate(Currency::GBP, "2026-01-01", 1_170_000);
    books.rate(Currency::GBP, "2026-02-01", 1_250_000);
    let invoice = books.issue(Currency::GBP, 1_000, "2026-01-10", "2026-02-09");
    post_outbound_document(books.database.connection(), invoice, "ada", NOW).unwrap();

    let item = books.receivable();
    let payment = books.receive_payment(1_000_000_000, "2026-02-20", "Betaling 2026-0001", "feb");
    let report = allocate(
        books.database.connection(),
        books.seller,
        &payment,
        &item.id,
        MatchMethod::Manual,
        "ada",
        NOW,
    )
    .unwrap();

    // €1 000 buys £800 at the payment date; £800 carried at 1.17 released €936,
    // so €64 is realised and £410 stays open.
    assert_eq!(report.settled, eur_micros(-1_000_000_000));
    assert_eq!(report.settled_base, eur_micros(-936_000_000));
    assert_eq!(report.fx_difference, eur_micros(64_000_000));
    let item = books.receivable();
    assert_eq!(item.outstanding, gbp_micros(410_000_000));
    assert_eq!(item.base_amount, eur_micros(1_415_700_000));
    assert!(item.is_open());

    let queued =
        store::list_transactions(books.database.connection(), books.seller, Some("partially_matched")).unwrap();
    assert_eq!(queued.len(), 1, "the rest of the payment is still waiting");
    assert_eq!(
        store::allocated_micros(books.database.connection(), &payment).unwrap(),
        1_000_000_000
    );
}

#[test]
fn one_payment_can_settle_a_euro_and_a_pound_invoice() {
    let mut books = Books::open();
    books.rate(Currency::GBP, "2026-01-01", 1_170_000);
    books.rate(Currency::GBP, "2026-02-01", 1_250_000);
    let euro = books.issue(Currency::EUR, 100, "2026-01-10", "2026-02-09");
    post_outbound_document(books.database.connection(), euro, "ada", NOW).unwrap();
    let pound = books.issue(Currency::GBP, 100, "2026-01-11", "2026-02-09");
    post_outbound_document(books.database.connection(), pound, "ada", NOW).unwrap();

    let items = list(books.database.connection(), books.seller, OpenItemKind::Receivable).unwrap();
    let euro_item = items
        .iter()
        .find(|item| item.amount.currency() == Currency::EUR)
        .unwrap();
    let pound_item = items
        .iter()
        .find(|item| item.amount.currency() == Currency::GBP)
        .unwrap();
    // €121 + £121 at the payment date (€151.25).
    let payment = books.receive_payment(272_250_000, "2026-02-20", "Betalingen", "feb");
    let reports = allocate_group(
        books.database.connection(),
        books.seller,
        &payment,
        &[euro_item.id.clone(), pound_item.id.clone()],
        MatchMethod::Group,
        "ada",
        NOW,
    )
    .unwrap();

    assert_eq!(reports.len(), 2);
    assert_eq!(reports[0].fx_difference, eur_micros(0));
    assert_eq!(reports[1].fx_difference, eur_micros(9_680_000));
    assert_eq!(reports[0].entry_id, reports[1].entry_id, "one payment, one entry");
    let stored = store::load_entry(books.database.connection(), reports[0].entry_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.entry.lines().len(),
        3,
        "one bank line, one control line for both invoices, and the difference"
    );
    assert_eq!(
        list(books.database.connection(), books.seller, OpenItemKind::Receivable)
            .unwrap()
            .iter()
            .filter(|item| item.is_open())
            .count(),
        0,
        "both invoices are settled"
    );
    let queued = store::list_transactions(books.database.connection(), books.seller, Some("matched")).unwrap();
    assert_eq!(queued.len(), 1, "the payment is used up");
}

#[test]
fn a_foreign_purchase_settles_with_a_realised_loss() {
    let mut books = Books::open();
    books.rate(Currency::GBP, "2026-01-01", 1_170_000);
    books.rate(Currency::GBP, "2026-02-01", 1_250_000);
    books.receive(Currency::GBP, 1_000, "2026-01-10", "SUP-1");

    let item = books.payable();
    let payment = books.receive_payment(-1_512_500_000, "2026-02-20", "Betaling SUP-1", "feb");
    let report = allocate(
        books.database.connection(),
        books.seller,
        &payment,
        &item.id,
        MatchMethod::Manual,
        "ada",
        NOW,
    )
    .unwrap();

    // Paying €1 512.50 to clear a £1 210 debt carried at €1 415.70 costs €96.80
    // more than the books held, which is a realised loss.
    assert_eq!(report.settled, eur_micros(-1_512_500_000));
    assert_eq!(report.settled_base, eur_micros(-1_415_700_000));
    assert_eq!(report.fx_difference, eur_micros(-96_800_000));

    let stored = store::load_entry(books.database.connection(), report.entry_id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.entry.lines().len(), 3);
    assert_eq!(line(&stored.entry, "1400").side(), Side::Debit);
    assert_eq!(line(&stored.entry, "1400").amount(), eur_micros(1_415_700_000));
    assert_eq!(line(&stored.entry, "1100").side(), Side::Credit);
    assert_eq!(line(&stored.entry, "1100").amount(), eur_micros(1_512_500_000));
    assert_eq!(line(&stored.entry, "7900").side(), Side::Debit);
    assert_eq!(line(&stored.entry, "7900").amount(), eur_micros(96_800_000));

    assert!(!books.payable().is_open());
    let readiness = store::readiness(books.database.connection(), books.seller, 2026, 2).unwrap();
    assert!(
        matches!(readiness.control_accounts, store::Check::Passed),
        "{:?}",
        readiness.control_accounts
    );
}

#[test]
fn a_foreign_settlement_cannot_be_undone_without_a_reversal() {
    let mut books = Books::open();
    books.rate(Currency::GBP, "2026-01-01", 1_170_000);
    books.rate(Currency::GBP, "2026-02-01", 1_250_000);
    let invoice = books.issue(Currency::GBP, 1_000, "2026-01-10", "2026-02-09");
    post_outbound_document(books.database.connection(), invoice, "ada", NOW).unwrap();

    let item = books.receivable();
    let payment = books.receive_payment(1_000_000_000, "2026-02-20", "Betaling 2026-0001", "feb");
    let report = allocate(
        books.database.connection(),
        books.seller,
        &payment,
        &item.id,
        MatchMethod::Manual,
        "ada",
        NOW,
    )
    .unwrap();

    let refused = unmatch(
        books.database.connection(),
        books.seller,
        &report.allocation_id,
        "ada",
        NOW,
    )
    .unwrap_err();
    assert!(refused.to_string().contains("at a rate"), "{refused}");

    let item = books.receivable();
    let refused = allocate(
        books.database.connection(),
        books.seller,
        &payment,
        &item.id,
        MatchMethod::Manual,
        "ada",
        NOW,
    )
    .unwrap_err();
    assert!(refused.to_string().contains("already matched at a rate"), "{refused}");
}
