#![allow(clippy::unwrap_used)]
//! The reports that make the ledger visible, and the two identities that make it
//! trustworthy: a trial balance nets to zero, and a balance sheet balances
//! because the current result is derived rather than posted.

use crux::{
    AccountCode, Address, CountryCode, Currency, CustomerId, DraftLine, InvoiceDraft, Journal, LedgerEntry, LedgerLine,
    LineId, LocalDate, Money, Party, Quantity, SellerId, Side, TaxTreatment, UnitCode, VatRate,
};
use store::ledger::{OpeningBalance, PostingRequest, PostingSource};
use store::{
    CustomerRecord, Database, IssueRequest, KdfParams, KeyStore, SellerRecord, balance_sheet, insert_customer,
    insert_draft, insert_seller, issue_draft, post_entry, post_opening_balances, post_outbound_document,
    profit_and_loss, reverse_entry, seed_starter_chart, trial_balance,
};
use tempfile::TempDir;

const NOW: &str = "2026-11-06T15:00:00.123456Z";

fn date(raw: &str) -> LocalDate {
    LocalDate::parse_iso(raw).unwrap()
}

fn code(raw: &str) -> AccountCode {
    AccountCode::parse(raw).unwrap()
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
    /// A quarter of trading: opening capital, two sales, and a bank charge.
    fn traded() -> Self {
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

        let mut books = Self {
            _dir: dir,
            database,
            seller,
            customer,
        };

        post_opening_balances(
            books.database.connection(),
            seller,
            date("2026-01-01"),
            &[
                OpeningBalance {
                    account: code("1100"),
                    side: Side::Debit,
                    amount: euros(5_000),
                },
                OpeningBalance {
                    account: code("0500"),
                    side: Side::Credit,
                    amount: euros(5_000),
                },
            ],
            "ada",
            NOW,
        )
        .unwrap();

        // 1 000 net at 21 % in January, 2 000 net at 9 % in February.
        books.sell(TaxTreatment::standard(VatRate::STANDARD_NL), 1_000, "2026-01-15");
        books.sell(TaxTreatment::standard(VatRate::REDUCED_NL), 2_000, "2026-02-15");

        // A bank charge, posted by hand for now: bank import is A4.
        let fee = LedgerEntry::new(
            Journal::Bank,
            "Bankkosten januari",
            vec![
                LedgerLine::debit(code("7100"), euros(20)).unwrap(),
                LedgerLine::credit(code("1100"), euros(20)).unwrap(),
            ],
        )
        .unwrap();
        post_entry(
            books.database.connection(),
            PostingRequest {
                entry: &fee,
                seller_id: seller,
                entry_date: date("2026-01-31"),
                source: PostingSource::Manual,
                actor: "ada",
                now: NOW,
                posting_profile_id: "nl-sme",
                rule_version: "1",
                reverses: None,
            },
        )
        .unwrap();

        books
    }

    fn sell(&mut self, treatment: TaxTreatment, net_units: i64, on: &str) {
        let mut draft = InvoiceDraft::new(self.seller, self.customer, Currency::EUR, date(on));
        draft.lines.push(DraftLine {
            id: LineId::new(),
            position: 1,
            name: "Consulting".to_owned(),
            description: None,
            quantity: Quantity::ONE,
            unit: UnitCode::PIECE,
            unit_price: euros(net_units),
            price_base_quantity: Quantity::ONE,
            treatment,
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
    }
}

#[test]
fn the_trial_balance_nets_to_zero_and_names_the_movements() {
    let books = Books::traded();
    let balance = trial_balance(
        books.database.connection(),
        books.seller,
        date("2026-01-01"),
        date("2026-03-31"),
    )
    .unwrap();

    assert!(balance.balances(), "difference: {}", balance.difference_micros());
    assert_eq!(balance.currency, Currency::EUR);

    let row = |account: &str| {
        balance
            .rows
            .iter()
            .find(|row| row.code.as_str() == account)
            .unwrap_or_else(|| panic!("{account} has no movement"))
            .clone()
    };

    // Debtors carried two invoices: 1 210 and 2 180.
    assert_eq!(row("1300").debit_micros, 3_390_000_000);
    assert_eq!(row("1300").credit_micros, 0);
    // Revenue split by rate, so the return can be explained later.
    assert_eq!(row("8000").credit_micros, 1_000_000_000);
    assert_eq!(row("8010").credit_micros, 2_000_000_000);
    // VAT per rate, and the bank charge.
    assert_eq!(row("1600").credit_micros, 210_000_000);
    assert_eq!(row("1610").credit_micros, 180_000_000);
    assert_eq!(row("7100").debit_micros, 20_000_000);
    // Opening capital and the bank both moved.
    assert_eq!(row("0500").credit_micros, 5_000_000_000);
    assert_eq!(row("1100").balance_micros(), 4_980_000_000);
}

#[test]
fn the_window_limits_which_months_are_counted() {
    let books = Books::traded();
    let january = trial_balance(
        books.database.connection(),
        books.seller,
        date("2026-01-01"),
        date("2026-01-31"),
    )
    .unwrap();

    assert!(january.balances());
    // The February sale is outside the window.
    assert!(january.rows.iter().all(|row| row.code.as_str() != "8010"));
    let revenue = january.rows.iter().find(|row| row.code.as_str() == "8000").unwrap();
    assert_eq!(revenue.credit_micros, 1_000_000_000);

    let backwards = trial_balance(
        books.database.connection(),
        books.seller,
        date("2026-03-01"),
        date("2026-01-01"),
    )
    .unwrap_err();
    assert!(backwards.to_string().contains("ends before it starts"), "{backwards}");
}

#[test]
fn the_profit_and_loss_shows_revenue_against_costs() {
    let books = Books::traded();
    let pnl = profit_and_loss(
        books.database.connection(),
        books.seller,
        date("2026-01-01"),
        date("2026-03-31"),
    )
    .unwrap();

    assert_eq!(pnl.revenue_micros, 3_000_000_000, "1 000 + 2 000 net");
    assert_eq!(pnl.expense_micros, 20_000_000);
    assert_eq!(pnl.result_micros(), 2_980_000_000);

    assert!(pnl.revenue.iter().all(|row| row.kind == crux::AccountKind::Revenue));
    assert!(pnl.expenses.iter().all(|row| row.kind == crux::AccountKind::Expense));
    assert_eq!(pnl.revenue.len(), 2);
    assert_eq!(pnl.expenses.len(), 1);
}

#[test]
fn the_balance_sheet_balances_with_the_current_result() {
    let books = Books::traded();
    let sheet = balance_sheet(books.database.connection(), books.seller, date("2026-03-31")).unwrap();

    assert!(sheet.balances(), "difference: {}", sheet.difference_micros());
    assert_eq!(sheet.assets_micros, 4_980_000_000 + 3_390_000_000, "bank plus debtors");
    assert_eq!(sheet.equity_micros, 5_000_000_000);
    assert_eq!(
        sheet.result_micros, 2_980_000_000,
        "the same result the P&L reports, derived rather than posted"
    );
    assert!(sheet.liabilities.iter().any(|row| row.code.as_str() == "1600"));
}

#[test]
fn a_reversal_takes_its_effect_out_of_the_reports() {
    let mut books = Books::traded();
    books.sell(TaxTreatment::standard(VatRate::STANDARD_NL), 1_000, "2026-03-15");
    let before = trial_balance(
        books.database.connection(),
        books.seller,
        date("2026-01-01"),
        date("2026-03-31"),
    )
    .unwrap();

    let entry: String = books
        .database
        .connection()
        .query_row(
            "SELECT id FROM journal_entry WHERE entry_date_local = '2026-03-15'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    reverse_entry(
        books.database.connection(),
        crux::JournalEntryId::parse_str(&entry).unwrap(),
        "wrong customer",
        date("2026-03-20"),
        "ada",
        NOW,
    )
    .unwrap();

    let after = trial_balance(
        books.database.connection(),
        books.seller,
        date("2026-01-01"),
        date("2026-03-31"),
    )
    .unwrap();

    assert!(after.balances());
    let row = |balance: &store::TrialBalance, account: &str| {
        balance
            .rows
            .iter()
            .find(|row| row.code.as_str() == account)
            .map(|row| (row.debit_micros, row.credit_micros))
            .unwrap_or((0, 0))
    };
    assert_eq!(row(&before, "1300").0, 3_390_000_000 + 1_210_000_000);
    assert_eq!(
        row(&after, "1300").0,
        3_390_000_000 + 1_210_000_000,
        "the invoice stays"
    );
    assert_eq!(
        row(&after, "1300").1,
        1_210_000_000,
        "and its reversal appears as a credit"
    );
    assert_eq!(
        row(&after, "8000").1 - row(&after, "8000").0,
        1_000_000_000,
        "which leaves only the January sale on the revenue account"
    );
    assert_eq!(
        row(&before, "8000").1 - row(&before, "8000").0,
        2_000_000_000,
        "the March sale was there before the reversal"
    );
}

#[test]
fn an_empty_ledger_has_a_balanced_empty_balance_sheet() {
    let dir = TempDir::new().unwrap();
    let keystore = KeyStore::create(dir.path().join("keystore.bin"), "passphrase", KdfParams::TEST).unwrap();
    let database = Database::create(dir.path().join("bonk.db"), &keystore).unwrap();
    let seller = SellerId::new();
    insert_seller(
        database.connection(),
        &SellerRecord {
            id: seller,
            party: Party::new("Nieuw BV", Address::new("Delft", CountryCode::NL)),
            default_currency: Currency::EUR,
            default_locale: "nl-NL".to_owned(),
        },
        NOW,
    )
    .unwrap();

    let sheet = balance_sheet(database.connection(), seller, date("2026-12-31")).unwrap();
    assert!(sheet.balances());
    assert_eq!(sheet.assets_micros, 0);
    assert!(sheet.assets.is_empty());

    let balance = trial_balance(database.connection(), seller, date("2026-01-01"), date("2026-12-31")).unwrap();
    assert!(balance.balances());
    assert!(balance.rows.is_empty());
}
