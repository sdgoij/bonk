//! Keeps writing to an archive until somebody kills the process.
//!
//! This is the child half of `crates/store/tests/crash.rs`: it seeds an archive,
//! reports `ready` on stdout once it has committed real work, and then loops
//! forever on transactions a kill can land in the middle of. The test kills it
//! without warning and checks that the next `Database::open` still finds a
//! coherent archive.
//!
//! It is a development aid. `just build` builds `cli`, so this binary is not part
//! of a shipped archive; it exists only so Cargo can hand the test a path through
//! `CARGO_BIN_EXE_crash-writer`.

use std::error::Error;
use std::io::Write as _;
use std::path::PathBuf;

use crux::{
    Address, CountryCode, Currency, CustomerId, DraftLine, InvoiceDraft, LineId, LocalDate, Money, Party, Quantity,
    SellerId, TaxTreatment, UnitCode, VatRate,
};
use store::{
    CustomerRecord, Database, IssueRequest, KeyStore, SellerRecord, audit, first_seller, insert_customer,
    insert_seller, issue_draft, list_customers, save_draft,
};

/// Rounds allowed to finish before the parent may kill us, so the archive is
/// never empty when the test looks at it.
const PRE_READY: usize = 5;

/// Upper bound on the lines one round writes. A round then spans several
/// statements and one commit, which is the window the test wants to interrupt.
const MAX_LINES: usize = 180;

/// Fixed business date; the archive is discarded after the test.
const ISSUE_DATE: &str = "2026-10-27";

struct Args {
    database: PathBuf,
    keystore: PathBuf,
    passphrase: String,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;

    let keystore = KeyStore::open(&args.keystore, &args.passphrase)?;
    let mut database = Database::open(&args.database, &keystore)?;

    let (seller, customer) = ensure_parties(&database)?;
    if count(&database, "document")? == 0 {
        issue_one(&mut database, seller, customer)?;
    }

    for round in 0..PRE_READY {
        write_round(&database, seller, customer, round)?;
    }
    println!("ready");
    std::io::stdout().flush()?;

    let mut round = PRE_READY;
    loop {
        write_round(&database, seller, customer, round)?;
        round += 1;
    }
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut database = None;
    let mut keystore = None;
    let mut passphrase = None;

    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| -> Box<dyn Error> { format!("{flag} needs a value").into() })?;
        match flag.as_str() {
            "--db" => database = Some(PathBuf::from(value)),
            "--keystore" => keystore = Some(PathBuf::from(value)),
            "--passphrase" => passphrase = Some(value),
            other => return Err(format!("unknown argument {other}").into()),
        }
    }

    Ok(Args {
        database: database.ok_or("--db is required")?,
        keystore: keystore.ok_or("--keystore is required")?,
        passphrase: passphrase.ok_or("--passphrase is required")?,
    })
}

/// Returns the archive's parties, creating them on the first start only.
fn ensure_parties(database: &Database) -> Result<(SellerId, CustomerId), Box<dyn Error>> {
    let seller = match first_seller(database.connection())? {
        Some(seller) => seller.id,
        None => {
            let id = SellerId::new();
            let mut party = Party::new("Crash Seller BV", Address::new("Amsterdam", CountryCode::NL));
            party.vat_id = Some("NL123456782B01".to_owned());
            insert_seller(
                database.connection(),
                &SellerRecord {
                    id,
                    party,
                    default_currency: Currency::EUR,
                    default_locale: "nl-NL".to_owned(),
                },
                ISSUE_DATE,
            )?;
            id
        }
    };

    let customer = match list_customers(database.connection())?.first() {
        Some(customer) => customer.id,
        None => {
            let id = CustomerId::new();
            insert_customer(
                database.connection(),
                &CustomerRecord {
                    id,
                    party: Party::new("Crash Customer BV", Address::new("Rotterdam", CountryCode::NL)),
                },
                ISSUE_DATE,
            )?;
            id
        }
    };

    Ok((seller, customer))
}

/// One issued document, so the artefact table is not empty when the test checks
/// it. Issuing writes a document, its lines, two audit links, and one artefact in
/// a single transaction; the parent cannot kill this, because it waits for
/// `ready` first.
fn issue_one(database: &mut Database, seller: SellerId, customer: CustomerId) -> Result<(), Box<dyn Error>> {
    let mut draft = seed_draft(seller, customer)?;
    draft.lines.push(line(1)?);
    save_draft(database.connection(), &draft, ISSUE_DATE)?;
    issue_draft(
        database.connection_mut(),
        IssueRequest {
            draft_id: draft.id,
            actor: "crash-writer",
            device: "crash-writer",
            renderer_version: "crash-writer",
            now: ISSUE_DATE,
        },
        |_| Ok(b"<Invoice/>".to_vec()),
    )?;
    Ok(())
}

/// One round: a draft row plus up to `MAX_LINES` line rows, then one audit link.
///
/// The note records the line count the same transaction writes, so the test can
/// prove that a surviving draft kept every line it says it has.
fn write_round(
    database: &Database,
    seller: SellerId,
    customer: CustomerId,
    round: usize,
) -> Result<(), Box<dyn Error>> {
    let lines = 20 + round % MAX_LINES;
    let mut draft = seed_draft(seller, customer)?;
    draft.note = Some(format!("lines={lines}"));
    for position in 1..=lines {
        draft.lines.push(line(position)?);
    }

    save_draft(database.connection(), &draft, ISSUE_DATE)?;
    audit::append(
        database.connection(),
        ISSUE_DATE,
        "crash-writer",
        "draft_saved",
        "invoice_draft",
        &draft.id.to_string(),
        serde_json::json!({ "round": round, "lines": lines }),
    )?;
    Ok(())
}

fn seed_draft(seller: SellerId, customer: CustomerId) -> Result<InvoiceDraft, Box<dyn Error>> {
    Ok(InvoiceDraft::new(
        seller,
        customer,
        Currency::EUR,
        LocalDate::parse_iso(ISSUE_DATE)?,
    ))
}

fn line(position: usize) -> Result<DraftLine, Box<dyn Error>> {
    Ok(DraftLine {
        id: LineId::new(),
        position: u32::try_from(position)?,
        name: format!("Crash line {position}"),
        description: Some("written while the process was about to die".to_owned()),
        quantity: Quantity::from_units(1)?,
        unit: UnitCode::PIECE,
        unit_price: Money::from_major_units(1, Currency::EUR)?,
        price_base_quantity: Quantity::ONE,
        treatment: TaxTreatment::standard(VatRate::STANDARD_NL),
    })
}

fn count(database: &Database, table: &str) -> Result<i64, Box<dyn Error>> {
    // `table` is a compile-time constant at every call site.
    let sql = format!("SELECT count(*) FROM {table}");
    Ok(database.connection().query_row(&sql, [], |row| row.get(0))?)
}
