//! Deterministic demo data for trying the window.
//!
//! The generator writes through the same repository functions the CLI and the
//! window use, so the archive it produces is an ordinary archive: issued
//! documents have artefacts and audit events, received documents have payloads
//! and import records, and `bonk verify` passes on it. Nothing here is a
//! back door into the schema.
//!
//! The seed is the only source of randomness, and `today` is an input, so the
//! same options produce the same numbers, dates, and amounts. Document
//! identifiers are random, as they are everywhere else.

use application::{Archive as Session, render_issue};
use crux::{
    Address, CountryCode, Currency, CustomerId, DocumentId, DraftLine, InvoiceDraft, LineId, LocalDate, Money, Party,
    Quantity, SavedSearchId, SellerId, TaxTreatment, UnitCode, VatRate, calculate_totals,
};
use store::{
    CustomerRecord, Direction, DocumentStatus, IssueRequest, IssueView, SavedSearch, SellerRecord, advance_status,
    credit_note_for, insert_customer, insert_draft, insert_seller, issue_draft,
};

/// The passphrase used when the caller does not give one.
pub const DEFAULT_PASSPHRASE: &str = "bonk-demo";

const ACTOR: &str = "demo-seed";
const RENDERER: &str = "syntax-0.1";
const SELLER_NAME: &str = "Studio Noord B.V.";

/// How much data to generate.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Customers to create; capped at the size of the built-in catalogue.
    pub customers: usize,
    /// Issued documents, credit notes included.
    pub invoices: usize,
    /// Documents received from suppliers.
    pub received: usize,
    /// Generator seed.
    pub seed: u64,
    /// Business date the data ends on; tests pin it, the command uses today.
    pub today: Option<LocalDate>,
    /// Finish the year before the data's last one: post everything dated in it and
    /// close it, so a fresh archive has a closed year — with its result on equity —
    /// to open in the window, and this year is visibly still work in progress.
    pub close_last_year: bool,
}

/// What was written.
#[derive(Clone, Copy, Debug)]
pub struct Seeded {
    /// Customers created.
    pub customers: usize,
    /// Issued invoices.
    pub invoices: usize,
    /// Issued credit notes.
    pub credit_notes: usize,
    /// Documents received from suppliers.
    pub received: usize,
    /// Received documents whose supplier matched a party on file.
    pub matched: usize,
    /// Received documents whose supplier was parked rather than invented.
    pub parked: usize,
    /// Unissued drafts.
    pub drafts: usize,
    /// Saved find-panel views.
    pub views: usize,
    /// Documents posted to the ledger.
    pub posted: usize,
    /// Bank transactions waiting in the queue.
    pub bank_lines: usize,
    /// Standing instructions that classify a bank line.
    pub rules: usize,
    /// The year the archive has closed, when it has one.
    pub closed_year: Option<i32>,
}

/// Fills an empty archive with demo data.
pub fn seed(session: &mut Session, options: &Options) -> Result<Seeded, DemoError> {
    if options.customers == 0 {
        return Err(DemoError::Input(
            "a demo archive needs at least one customer".to_owned(),
        ));
    }

    let today = options.today.unwrap_or_else(LocalDate::today_utc);
    let mut rng = Rng::new(options.seed);
    let owner = SellerId::new();
    let seller_party = seller_party();
    let opening = at(today, 8, 0);

    let mut customers = Vec::new();
    {
        let connection = session.database().connection();
        insert_seller(
            connection,
            &SellerRecord {
                id: owner,
                party: seller_party.clone(),
                default_currency: Currency::EUR,
                default_locale: "nl-NL".to_owned(),
            },
            &opening,
        )?;

        for spec in CUSTOMERS.iter().take(options.customers.min(CUSTOMERS.len())) {
            let party = party_from(spec, &mut rng)?;
            let id = CustomerId::new();
            insert_customer(
                connection,
                &CustomerRecord {
                    id,
                    party: party.clone(),
                },
                &opening,
            )?;
            customers.push(SeededCustomer {
                id,
                party,
                currency: Currency::parse(spec.currency.unwrap_or("EUR"))?,
            });
        }
    }

    let mut issued = Vec::with_capacity(options.invoices);
    let mut invoice_count = 0_usize;
    let mut credit_count = 0_usize;
    let mut dates = Vec::with_capacity(options.invoices);
    for _ in 0..options.invoices {
        dates.push(today.plus_days(-rng.range(0, 450))?);
    }
    dates.sort_unstable();

    let mut documents: Vec<Issued> = Vec::with_capacity(options.invoices);
    for (index, date) in dates.into_iter().enumerate() {
        let credit = if index > 0 && index % 20 == 0 {
            issue_credit_note(session, owner, &issued, date, today, &mut rng)?
        } else {
            None
        };
        if let Some(note) = credit {
            documents.push(note);
            credit_count += 1;
        } else {
            let customer = &customers[rng.below(customers.len() as u64) as usize];
            let draft = invoice_draft(owner, customer, date, &mut rng)?;
            let record = issue(session, &draft, &at(date, 9, 0))?;
            let document = Issued {
                id: record.id,
                number: record.number,
                date,
            };
            // Credit notes point at invoices, so only the invoices go in `issued`.
            documents.push(document.clone());
            issued.push(document);
            invoice_count += 1;
        }
    }

    let inbound = seed_inbound(
        session,
        owner,
        &seller_party,
        &customers,
        options.received,
        today,
        &mut rng,
    )?;

    let drafts = seed_drafts(session, owner, &customers, today, &mut rng)?;
    let views = seed_views(session, owner, today)?;
    let books = seed_books(
        session,
        owner,
        today,
        &documents,
        inbound.open.as_slice(),
        &customers,
        options.close_last_year,
    )?;

    Ok(Seeded {
        customers: customers.len(),
        invoices: invoice_count,
        credit_notes: credit_count,
        received: inbound.received,
        matched: inbound.matched,
        parked: inbound.parked,
        drafts,
        views,
        posted: books.posted,
        bank_lines: books.bank_lines,
        rules: books.rules,
        closed_year: books.closed_year,
    })
}

/// Issues a credit note for a previously issued invoice.
///
/// A credit note is dated after the invoice it corrects, so an invoice dated
/// today is skipped: there is no room for a correction yet, and the caller
/// issues an ordinary invoice instead.
fn issue_credit_note(
    session: &mut Session,
    owner: SellerId,
    issued: &[Issued],
    date: LocalDate,
    today: LocalDate,
    rng: &mut Rng,
) -> Result<Option<Issued>, DemoError> {
    if issued.is_empty() {
        return Ok(None);
    }
    let previous = &issued[rng.below(issued.len() as u64) as usize];
    if previous.date >= today {
        return Ok(None);
    }

    let earliest = previous.date.plus_days(1)?;
    let credit_date = if date > earliest { date } else { earliest };
    let Some(draft) = credit_note_for(session.database().connection(), owner, &previous.number, credit_date)? else {
        return Ok(None);
    };

    let record = issue(session, &draft, &at(credit_date, 10, 0))?;
    Ok(Some(Issued {
        id: record.id,
        number: record.number,
        date: credit_date,
    }))
}

/// Creates a handful of drafts that were never issued.
fn seed_drafts(
    session: &Session,
    owner: SellerId,
    customers: &[SeededCustomer],
    today: LocalDate,
    rng: &mut Rng,
) -> Result<usize, DemoError> {
    let mut count = 0_usize;
    for (index, customer) in customers.iter().take(4).enumerate() {
        let date = today.plus_days(-(index as i64 + 1))?;
        let mut draft = invoice_draft(owner, customer, date, rng)?;
        draft.due_date = Some(today.plus_days(30)?);
        insert_draft(
            session.database().connection(),
            &draft,
            &at(today, 10, u8::try_from(index).unwrap_or(0)),
        )?;
        count += 1;
    }
    Ok(count)
}

/// Saves a few find-panel views, one per side and status a user reaches for.
fn seed_views(session: &Session, owner: SellerId, today: LocalDate) -> Result<usize, DemoError> {
    let views = [
        SavedSearch {
            id: SavedSearchId::new(),
            name: "Open purchases".to_owned(),
            query: String::new(),
            direction: Some(Direction::Inbound),
            status: Some(DocumentStatus::Received),
        },
        SavedSearch {
            id: SavedSearchId::new(),
            name: "Paid purchases".to_owned(),
            query: String::new(),
            direction: Some(Direction::Inbound),
            status: Some(DocumentStatus::Paid),
        },
        SavedSearch {
            id: SavedSearchId::new(),
            name: "Everything we issued".to_owned(),
            query: String::new(),
            direction: Some(Direction::Outbound),
            status: None,
        },
        SavedSearch {
            id: SavedSearchId::new(),
            name: "Amsterdam customers".to_owned(),
            query: "Amsterdam".to_owned(),
            direction: Some(Direction::Outbound),
            status: None,
        },
    ];

    let now = at(today, 11, 0);
    for view in &views {
        store::saved::save(session.database().connection(), owner, view, &now)?;
    }
    Ok(views.len())
}

/// Counts for the payable register.
struct InboundCounts {
    received: usize,
    matched: usize,
    parked: usize,
    /// Documents that matched a supplier and are still waiting to be paid, so they
    /// can be posted and become open payables the bank queue can settle.
    open: Vec<DocumentId>,
}

/// Records invoices (and the occasional supplier document) received from
/// suppliers, then moves some of them through the payable register.
fn seed_inbound(
    session: &Session,
    owner: SellerId,
    seller_party: &Party,
    customers: &[SeededCustomer],
    count: usize,
    today: LocalDate,
    rng: &mut Rng,
) -> Result<InboundCounts, DemoError> {
    let suppliers = suppliers(customers)?;
    let us = CustomerRecord {
        id: CustomerId::new(),
        party: seller_party.clone(),
    };

    let mut dates = Vec::with_capacity(count);
    for _ in 0..count {
        let days_ago = rng.range(0, 450);
        dates.push((today.plus_days(-days_ago)?, days_ago));
    }
    dates.sort_by_key(|(date, _)| *date);

    let mut counters = vec![0_u32; suppliers.len()];
    let mut matched = 0_usize;
    let mut parked = 0_usize;

    for (index, (date, days_ago)) in dates.into_iter().enumerate() {
        let slot = index % suppliers.len();
        let supplier = &suppliers[slot];
        counters[slot] += 1;
        let number = format!("{}-{}-{:04}", supplier.prefix, year_of(date), counters[slot]);

        let draft = inbound_draft(date, supplier.currency, rng)?;
        let totals = calculate_totals(&draft.lines)?;
        let supplier_record = SellerRecord {
            id: SellerId::new(),
            party: supplier.party.clone(),
            default_currency: supplier.currency,
            default_locale: "en-GB".to_owned(),
        };
        let view = IssueView {
            id: DocumentId::new(),
            number: &number,
            draft: &draft,
            seller: &supplier_record,
            customer: &us,
            totals: &totals,
        };
        let bytes = render_issue(&view)?;

        let imported_at = at(earliest(today, date.plus_days(1)?), 8, 30);
        let path = format!("demo/inbound/{number}.xml");
        let outcome = application::import::import_bytes(session.database(), owner, Some(&path), &bytes, &imported_at)?;
        if outcome.supplier.is_some() {
            matched += 1;
        } else {
            parked += 1;
        }

        advance_payable(session, outcome.document_id, date, days_ago, today, rng)?;
    }

    // Every received document is posted, matched or not: the VAT readiness check
    // refuses a month while any document dated in it is unposted, and a purchase
    // that arrived is a liability whether or not somebody has said who sent it.
    let open = store::list_received(session.database().connection(), owner)?
        .into_iter()
        .map(|row| row.id)
        .collect();

    Ok(InboundCounts {
        received: count,
        matched,
        parked,
        open,
    })
}

/// Moves a received document as far through the register as its age suggests.
///
/// It stops at approved. A paid purchase would need a settlement the demo does not
/// invent: the books would either carry a debt the register calls settled, or leave
/// the document unposted and make its month unfileable. The bank statement pays one
/// of these invoices instead, which is what the queue is for.
fn advance_payable(
    session: &Session,
    document: DocumentId,
    date: LocalDate,
    days_ago: i64,
    today: LocalDate,
    rng: &mut Rng,
) -> Result<(), DemoError> {
    let approved = days_ago >= 75 || (days_ago >= 30 && rng.below(2) == 0);
    if approved {
        let when = earliest(today, date.plus_days(15)?);
        advance_status(
            session.database().connection(),
            document,
            DocumentStatus::Approved,
            ACTOR,
            &at(when, 12, 0),
        )?;
    }
    Ok(())
}

/// What the books hold after seeding.
struct BookCounts {
    posted: usize,
    bank_lines: usize,
    rules: usize,
    closed_year: Option<i32>,
}

/// How many documents the demo posts to the ledger.
///
/// The default archive issues about three hundred documents, and all of them post:
/// the VAT readiness check refuses a period while any document dated in it is
/// unposted, so a demo that stopped halfway would show every month as `fileable:
/// no` and teach the opposite of what the books are for. The cap is there for the
/// twenty-thousand-document search benchmark, where posting everything would turn a
/// minute of seeding into an hour; that archive keeps the newest few months of
/// books, which is all its reports are read for.
const LEDGER_DOCUMENTS: usize = 400;

/// Fills the ledger, a bank account, and the queue the window works.
///
/// Everything is written through the same functions the CLI uses, so the demo
/// archive is an ordinary archive whose entries the reports actually read. The
/// bank statement is built to hold one of each thing the queue can show: a payment
/// that names one invoice, a transfer that pays two, a part payment, a fee a rule
/// books, interest nothing will ever match, and a supplier payment.
fn seed_books(
    session: &Session,
    owner: SellerId,
    today: LocalDate,
    issued: &[Issued],
    payables: &[DocumentId],
    customers: &[SeededCustomer],
    close_last_year: bool,
) -> Result<BookCounts, DemoError> {
    let connection = session.database().connection();
    store::seed_starter_chart(connection, &at(today, 8, 45))?;
    // The demo trades in dollars and pounds; fixed, labelled rates keep posting
    // deterministic without pretending to be a market quote.
    for (currency, micros) in [(Currency::USD, 925_000), (Currency::GBP, 1_170_000)] {
        store::seed_demo_rate(
            connection,
            owner,
            currency,
            LocalDate::parse_iso("2020-01-02")?,
            crux::ExchangeRate::from_per_unit_micros(micros)?,
            ACTOR,
            &at(today, 8, 46),
        )?;
    }

    let mut posted = 0_usize;
    let mut newest: Vec<&Issued> = issued.iter().collect();
    newest.sort_by(|left, right| right.date.cmp(&left.date).then_with(|| left.number.cmp(&right.number)));
    for document in newest.into_iter().take(LEDGER_DOCUMENTS) {
        store::post_document(
            connection,
            document.id,
            ACTOR,
            &at(earliest(today, document.date), 10, 0),
        )?;
        posted += 1;
    }
    for document in payables.iter().take(LEDGER_DOCUMENTS) {
        // Every purchase is given an account before it is posted, so the books can
        // actually be closed: an unclassified cost is exactly what the close refuses,
        // and a demo where no period could be locked would teach the opposite of
        // what that account is for.
        store::expense::assign(connection, *document, "7000", ACTOR, &at(today, 10, 15))?;
        store::post_document(connection, *document, ACTOR, &at(today, 10, 15))?;
        posted += 1;
    }

    // Two kinds of year in one archive: last year finished, this one in progress.
    // The close is the same one `bonk ledger close-year` performs, so the demo's
    // closed year is an ordinary closed year rather than a flag somebody set.
    let closed_year = if close_last_year {
        match finish_last_year(session, owner, today)? {
            Some((year, finished)) => {
                posted += finished;
                Some(year)
            }
            None => None,
        }
    } else {
        None
    };

    store::insert_bank_account(
        connection,
        &store::BankAccount {
            id: BANK_ACCOUNT.to_owned(),
            seller_id: owner,
            name: "Zakelijke rekening".to_owned(),
            iban: Some("NL91ABNA0417164300".to_owned()),
            currency: Currency::EUR,
            ledger_account_code: crux::AccountCode::parse("1100")?,
            import_profile: None,
        },
        &at(today, 10, 30),
    )?;

    let receivables = open_items(session, owner, store::OpenItemKind::Receivable)?;
    let open_payables = open_items(session, owner, store::OpenItemKind::Payable)?;
    let statement = demo_statement(today, &receivables, &open_payables, customers)?;

    let mut bank_lines = 0_usize;
    if !statement.transactions.is_empty() {
        let report = store::import_statement(
            connection,
            BANK_ACCOUNT,
            &statement,
            &store::audit::sha256_hex(b"demo/inbound/afschrift-2026"),
            &at(today, 11, 0),
        )?;
        bank_lines = report.imported;
    }

    // One rule, pointed at the fee lines, so `bonk ledger bank classify --dry-run`
    // has something to show and the queue has something a person came for. The
    // interest line is deliberately left without one: it is nobody's invoice and
    // no rule's business, which is what the queue is for.
    let rule = store::bank_rule::RuleDefinition {
        name: "bankkosten".to_owned(),
        direction: Some(-1),
        counterparty_iban: None,
        remittance_contains: Some("kosten".to_owned()),
        counterparty_contains: None,
        account_code: "7100".to_owned(),
    };
    store::bank_rule::add(connection, owner, &rule, ACTOR, &at(today, 11, 15))?;

    Ok(BookCounts {
        posted,
        bank_lines,
        rules: 1,
        closed_year,
    })
}

/// Posts whatever is still missing from last year and closes it.
///
/// The year before the data's last one is finished here: every document dated in it
/// is in the books and the year is closed, which is what puts a `closed` year and a
/// result on equity in front of somebody opening the demo. This year stays open,
/// because that is what the queue, the payable register, and the month controls are
/// for.
///
/// Nothing is invented for it: the documents are the ones the generator already
/// created, and the close is the same `store::close_year` the command runs. A year
/// with no postings is left alone — there would be nothing to close.
fn finish_last_year(session: &Session, owner: SellerId, today: LocalDate) -> Result<Option<(i32, usize)>, DemoError> {
    let connection = session.database().connection();
    let year = today.as_date().year() - 1;
    if store::list_periods(connection, owner, year)?.is_empty() {
        return Ok(None);
    }

    let stamp = at(today, 10, 20);
    let pending = store::posting::pending(connection, owner)?;
    let mut posted = 0_usize;
    for document in pending
        .iter()
        .filter(|document| document.issue_date.as_date().year() == year)
    {
        if document.direction == "inbound" {
            store::expense::assign(connection, document.id, "7000", ACTOR, &stamp)?;
        }
        store::post_document(connection, document.id, ACTOR, &stamp)?;
        posted += 1;
    }

    store::close_year(connection, owner, year, ACTOR, &at(today, 12, 0))?;
    Ok(Some((year, posted)))
}

/// The bank account the demo statements belong to.
const BANK_ACCOUNT: &str = "demo-zakelijk";

/// Open items of one kind, newest first.
fn open_items(
    session: &Session,
    owner: SellerId,
    kind: store::OpenItemKind,
) -> Result<Vec<store::OpenItem>, DemoError> {
    let mut items: Vec<store::OpenItem> = store::open_items::list(session.database().connection(), owner, kind)?
        .into_iter()
        .filter(store::OpenItem::is_open)
        .collect();
    items.sort_by(|left, right| {
        right
            .original_date
            .cmp(&left.original_date)
            .then_with(|| left.number.cmp(&right.number))
    });
    Ok(items)
}

/// A statement holding one of each thing the queue can show.
fn demo_statement(
    today: LocalDate,
    receivables: &[store::OpenItem],
    payables: &[store::OpenItem],
    customers: &[SeededCustomer],
) -> Result<store::ParsedStatement, DemoError> {
    let name_of = |item: &store::OpenItem| {
        customers
            .iter()
            .find(|customer| customer.id == item.party_id)
            .map(|customer| customer.party.legal_name.clone())
            .unwrap_or_else(|| "Klant".to_owned())
    };
    let mut transactions = Vec::new();
    let mut add = |days_ago: i64, amount: i64, counterparty: &str, iban: &str, remittance: String| {
        let booking_date = today.plus_days(-days_ago)?;
        transactions.push(store::ParsedTransaction {
            booking_date,
            value_date: None,
            amount_micros: amount,
            currency: Currency::EUR,
            counterparty_name: Some(counterparty.to_owned()),
            counterparty_iban: if iban.is_empty() { None } else { Some(iban.to_owned()) },
            remittance_info: Some(remittance),
            external_ref: None,
        });
        Ok::<(), DemoError>(())
    };

    if let Some(item) = receivables.first() {
        add(
            25,
            item.outstanding.micros(),
            &name_of(item),
            "NL02RABO0123456789",
            format!("Betaling factuur {}", item.number),
        )?;
    }
    if receivables.len() >= 3 {
        let (first, second) = (&receivables[1], &receivables[2]);
        add(
            22,
            first.outstanding.micros() + second.outstanding.micros(),
            &name_of(first),
            "NL02RABO0123456789",
            format!("Facturen {} en {}", first.number, second.number),
        )?;
    }
    if let Some(item) = receivables.get(3) {
        // Half of it, so the queue shows money still unaccounted for.
        add(
            18,
            item.outstanding.micros() / 2,
            &name_of(item),
            "NL02RABO0123456789",
            format!("Deelbetaling {}", item.number),
        )?;
    }
    add(15, -12_500_000, "Rabobank", "", "Kosten zakelijke rekening".to_owned())?;
    add(12, 3_400_000, "Rabobank", "", "Rente spaarrekening".to_owned())?;
    add(9, -12_500_000, "Rabobank", "", "Kosten pinapparaten".to_owned())?;
    if let Some(item) = payables.first() {
        add(
            6,
            -item.outstanding.micros(),
            &name_of(item),
            "NL66INGB0001234567",
            format!("Betaling {}", item.number),
        )?;
    }

    let opening: i64 = 1_000_000_000;
    let movements: i64 = transactions.iter().map(|line| line.amount_micros).sum();
    let first = transactions.first().map(|line| line.booking_date);
    let last = transactions.last().map(|line| line.booking_date);
    Ok(store::ParsedStatement {
        format: "mt940",
        source_name: Some("demo/bank/afschrift-2026.txt".to_owned()),
        currency: Currency::EUR,
        period_start: first,
        period_end: last,
        opening_balance_micros: Some(opening),
        closing_balance_micros: Some(opening + movements),
        transactions,
    })
}

/// Inserts a draft and issues it in one step.
fn issue(session: &mut Session, draft: &InvoiceDraft, now: &str) -> Result<store::DocumentRecord, DemoError> {
    insert_draft(session.database().connection(), draft, now)?;
    let record = issue_draft(
        session.database_mut().connection_mut(),
        IssueRequest {
            draft_id: draft.id,
            actor: ACTOR,
            device: ACTOR,
            renderer_version: RENDERER,
            now,
        },
        render_issue,
    )?;
    Ok(record)
}

/// A customer that was written to the party table.
struct SeededCustomer {
    id: CustomerId,
    party: Party,
    currency: Currency,
}

/// An invoice we issued, kept so a credit note can point at it.
#[derive(Clone)]
struct Issued {
    id: DocumentId,
    number: String,
    date: LocalDate,
}

/// A party on the supplier side of the books.
struct SupplierFixture {
    party: Party,
    prefix: &'static str,
    currency: Currency,
}

/// Builds alternating matched and unmatched suppliers, so any slice of the
/// cycle covers both cases.
fn suppliers(customers: &[SeededCustomer]) -> Result<Vec<SupplierFixture>, DemoError> {
    const PREFIXES: [&str; 6] = ["INV", "FAC", "RE", "RG", "NS", "FT"];

    let mut fixtures = Vec::with_capacity(UNMATCHED_SUPPLIERS.len() * 2);
    for (index, spec) in UNMATCHED_SUPPLIERS.iter().enumerate() {
        if let Some(customer) = customers.get(index) {
            fixtures.push(SupplierFixture {
                party: customer.party.clone(),
                prefix: PREFIXES[index % PREFIXES.len()],
                currency: customer.currency,
            });
        }
        fixtures.push(SupplierFixture {
            party: supplier_party(spec)?,
            prefix: spec.prefix,
            currency: Currency::parse(spec.currency)?,
        });
    }
    Ok(fixtures)
}

/// A draft for a document a supplier sent us.
fn inbound_draft(date: LocalDate, currency: Currency, rng: &mut Rng) -> Result<InvoiceDraft, DemoError> {
    let mut draft = InvoiceDraft::new(SellerId::new(), CustomerId::new(), currency, date);
    draft.due_date = Some(date.plus_days(30)?);
    draft.delivery_date = Some(date);
    draft.buyer_reference = Some(format!("PO-{}-{:04}", year_of(date), rng.range(1, 9_999)));
    add_lines(&mut draft, rng)?;
    Ok(draft)
}

/// A draft for one of our invoices.
fn invoice_draft(
    owner: SellerId,
    customer: &SeededCustomer,
    date: LocalDate,
    rng: &mut Rng,
) -> Result<InvoiceDraft, DemoError> {
    let mut draft = InvoiceDraft::new(owner, customer.id, customer.currency, date);
    draft.due_date = Some(date.plus_days(if rng.below(2) == 0 { 14 } else { 30 })?);
    draft.delivery_date = Some(date.plus_days(-rng.range(0, 2))?);
    draft.buyer_reference = Some(format!("PO-{}-{:04}", year_of(date), rng.range(1, 9_999)));
    if rng.below(3) == 0 {
        draft.order_reference = Some(format!("ORD-{:05}", rng.range(1, 99_999)));
    }
    if rng.below(5) == 0 {
        draft.note = Some("Thank you for your business.".to_owned());
    }
    add_lines(&mut draft, rng)?;
    Ok(draft)
}

/// Appends one to four distinct services, so a document has a real breakdown.
fn add_lines(draft: &mut InvoiceDraft, rng: &mut Rng) -> Result<(), DemoError> {
    let count = rng.range(1, 4) as usize;
    let start = rng.below(SERVICES.len() as u64) as usize;
    for offset in 0..count {
        let service = &SERVICES[(start + offset) % SERVICES.len()];
        let position = u32::try_from(offset + 1).map_err(|_| DemoError::Input("too many lines".to_owned()))?;
        draft.lines.push(make_line(service, position, draft.currency, rng)?);
    }
    Ok(())
}

/// Builds one line with a price jitter, so amounts do not look like fixtures.
fn make_line(service: &Service, position: u32, currency: Currency, rng: &mut Rng) -> Result<DraftLine, DemoError> {
    let quantity = if service.unit == UnitCode::HOUR {
        Quantity::from_micros(rng.range(2, 120) * 500_000)
    } else if service.unit == UnitCode::DAY {
        Quantity::from_micros(rng.range(1, 5) * crux::MICROS_PER_UNIT)
    } else {
        Quantity::from_micros(rng.range(1, 3) * crux::MICROS_PER_UNIT)
    };
    let variation = rng.range(90, 110);
    Ok(DraftLine {
        id: LineId::new(),
        position,
        name: service.name.to_owned(),
        description: None,
        quantity,
        unit: service.unit,
        unit_price: Money::from_micros(service.price * variation / 100, currency),
        price_base_quantity: Quantity::ONE,
        treatment: TaxTreatment::standard(VatRate::from_basis_points(service.rate)?),
    })
}

/// A billable service, spelled out in micros so no float touches an amount.
struct Service {
    name: &'static str,
    unit: UnitCode,
    /// Base price in micros.
    price: i64,
    /// VAT rate in basis points.
    rate: u16,
}

const SERVICES: [Service; 15] = [
    Service {
        name: "Design consultancy",
        unit: UnitCode::HOUR,
        price: 145 * crux::MICROS_PER_UNIT,
        rate: 2100,
    },
    Service {
        name: "Front-end development",
        unit: UnitCode::HOUR,
        price: 125 * crux::MICROS_PER_UNIT,
        rate: 2100,
    },
    Service {
        name: "Copywriting",
        unit: UnitCode::HOUR,
        price: 95 * crux::MICROS_PER_UNIT,
        rate: 2100,
    },
    Service {
        name: "Technical writing",
        unit: UnitCode::HOUR,
        price: 105 * crux::MICROS_PER_UNIT,
        rate: 2100,
    },
    Service {
        name: "UX research day",
        unit: UnitCode::DAY,
        price: 980 * crux::MICROS_PER_UNIT,
        rate: 2100,
    },
    Service {
        name: "Design sprint",
        unit: UnitCode::PIECE,
        price: 4_800 * crux::MICROS_PER_UNIT,
        rate: 2100,
    },
    Service {
        name: "Brand identity",
        unit: UnitCode::PIECE,
        price: 6_500 * crux::MICROS_PER_UNIT,
        rate: 2100,
    },
    Service {
        name: "Content strategy",
        unit: UnitCode::PIECE,
        price: 1_100 * crux::MICROS_PER_UNIT,
        rate: 2100,
    },
    Service {
        name: "Print production",
        unit: UnitCode::PIECE,
        price: 340 * crux::MICROS_PER_UNIT,
        rate: 2100,
    },
    Service {
        name: "Hosting",
        unit: UnitCode::PIECE,
        price: 89 * crux::MICROS_PER_UNIT,
        rate: 2100,
    },
    Service {
        name: "Support retainer",
        unit: UnitCode::PIECE,
        price: 750 * crux::MICROS_PER_UNIT,
        rate: 2100,
    },
    Service {
        name: "Training workshop",
        unit: UnitCode::PIECE,
        price: 1_950 * crux::MICROS_PER_UNIT,
        rate: 2100,
    },
    Service {
        name: "Photography",
        unit: UnitCode::PIECE,
        price: 620 * crux::MICROS_PER_UNIT,
        rate: 2100,
    },
    Service {
        name: "Accessibility audit",
        unit: UnitCode::PIECE,
        price: 2_400 * crux::MICROS_PER_UNIT,
        rate: 2100,
    },
    Service {
        name: "Editorial production",
        unit: UnitCode::PIECE,
        price: 1_200 * crux::MICROS_PER_UNIT,
        rate: 900,
    },
];

/// The seller whose books the demo archive holds.
fn seller_party() -> Party {
    let mut party = Party::new(
        SELLER_NAME,
        Address {
            street: Some("Prinsengracht 512".to_owned()),
            city: "Amsterdam".to_owned(),
            postal_code: Some("1017 KJ".to_owned()),
            country: CountryCode::NL,
        },
    );
    party.trading_name = Some("Studio Noord".to_owned());
    party.vat_id = Some("NL123456782B01".to_owned());
    party.kvk_number = Some("34567890".to_owned());
    party.peppol_id = Some("0106:34567890".to_owned());
    party.contact_name = Some("Sanne de Vries".to_owned());
    party.email = Some("finance@studionoord.example".to_owned());
    party.phone = Some("+31 20 555 0142".to_owned());
    party
}

/// How a customer's VAT identifier is obtained.
enum VatSpec {
    /// Derived from the seed and valid under the Dutch eleven-proof.
    Dutch,
    /// Spelled out in the catalogue.
    Fixed(&'static str),
    /// The party has none; a consumer or a company outside the VAT area.
    None,
}

/// How a customer's Peppol endpoint is obtained.
enum PeppolSpec {
    /// The Dutch chamber of commerce scheme, `0106`.
    Dutch,
    /// Spelled out in the catalogue.
    Fixed(&'static str),
    /// An email endpoint, for parties without a national scheme.
    Email(&'static str),
}

/// One row of the customer catalogue.
struct CustomerSpec {
    name: &'static str,
    street: &'static str,
    postal: &'static str,
    city: &'static str,
    country: &'static str,
    vat: VatSpec,
    peppol: PeppolSpec,
    /// Document currency; `None` means EUR.
    currency: Option<&'static str>,
    contact: &'static str,
    email: &'static str,
}

const CUSTOMERS: [CustomerSpec; 18] = [
    CustomerSpec {
        name: "Amsterdam Design Collective B.V.",
        street: "Herengracht 120",
        postal: "1015 BT",
        city: "Amsterdam",
        country: "NL",
        vat: VatSpec::Dutch,
        peppol: PeppolSpec::Dutch,
        currency: None,
        contact: "Eva Jansen",
        email: "eva@amsterdamdesign.example",
    },
    CustomerSpec {
        name: "Rotterdam Havenwerken B.V.",
        street: "Wilhelminakade 301",
        postal: "3072 AP",
        city: "Rotterdam",
        country: "NL",
        vat: VatSpec::Dutch,
        peppol: PeppolSpec::Dutch,
        currency: None,
        contact: "Bram de Boer",
        email: "bram@havenwerken.example",
    },
    CustomerSpec {
        name: "Utrecht Zorg Groep B.V.",
        street: "Maliebaan 45",
        postal: "3581 CD",
        city: "Utrecht",
        country: "NL",
        vat: VatSpec::Dutch,
        peppol: PeppolSpec::Dutch,
        currency: None,
        contact: "Noor van Dijk",
        email: "noor@utrechtzorg.example",
    },
    CustomerSpec {
        name: "Eindhoven Lighting Systems B.V.",
        street: "Strijp-S 12",
        postal: "5617 BC",
        city: "Eindhoven",
        country: "NL",
        vat: VatSpec::Dutch,
        peppol: PeppolSpec::Dutch,
        currency: None,
        contact: "Jeroen Smit",
        email: "jeroen@eindhovenlighting.example",
    },
    CustomerSpec {
        name: "Groningen Data Services B.V.",
        street: "Helperpark 8",
        postal: "9723 ZA",
        city: "Groningen",
        country: "NL",
        vat: VatSpec::Dutch,
        peppol: PeppolSpec::Dutch,
        currency: None,
        contact: "Ilse Bos",
        email: "ilse@groningendata.example",
    },
    CustomerSpec {
        name: "Maastricht Bouw & Renovatie B.V.",
        street: "Wycker Grachtstraat 22",
        postal: "6221 CX",
        city: "Maastricht",
        country: "NL",
        vat: VatSpec::Dutch,
        peppol: PeppolSpec::Dutch,
        currency: None,
        contact: "Huub Hermans",
        email: "huub@maastrichtbouw.example",
    },
    CustomerSpec {
        name: "Brussels Digital Studio SRL",
        street: "Rue du Canal 41",
        postal: "1000",
        city: "Brussels",
        country: "BE",
        vat: VatSpec::Fixed("BE0123456789"),
        peppol: PeppolSpec::Fixed("0208:0123456789"),
        currency: None,
        contact: "Camille Dubois",
        email: "camille@brusselsdigital.example",
    },
    CustomerSpec {
        name: "Köln Medienhaus GmbH",
        street: "Hohenzollernring 57",
        postal: "50672",
        city: "Köln",
        country: "DE",
        vat: VatSpec::Fixed("DE123456789"),
        peppol: PeppolSpec::Fixed("9930:123456789"),
        currency: None,
        contact: "Lena Fischer",
        email: "lena@koelnmedien.example",
    },
    CustomerSpec {
        name: "Atelier Lumière SARL",
        street: "18 Rue de la République",
        postal: "69002",
        city: "Lyon",
        country: "FR",
        vat: VatSpec::Fixed("FR12345678901"),
        peppol: PeppolSpec::Fixed("0009:12345678901"),
        currency: None,
        contact: "Élodie Martin",
        email: "elodie@atelierlumiere.example",
    },
    CustomerSpec {
        name: "Talleres Ibéricos S.L.",
        street: "Carrer de Colón 74",
        postal: "46004",
        city: "Valencia",
        country: "ES",
        vat: VatSpec::Fixed("ESA12345678"),
        peppol: PeppolSpec::Fixed("9920:A12345678"),
        currency: None,
        contact: "Marta Ruiz",
        email: "marta@talleresibericos.example",
    },
    CustomerSpec {
        name: "Milano Forma S.r.l.",
        street: "Via Tortona 31",
        postal: "20144",
        city: "Milano",
        country: "IT",
        vat: VatSpec::Fixed("IT12345678901"),
        peppol: PeppolSpec::Fixed("0097:12345678901"),
        currency: None,
        contact: "Luca Bianchi",
        email: "luca@milanoforma.example",
    },
    CustomerSpec {
        name: "Dublin Cloud Partners Ltd",
        street: "12 Grand Canal Quay",
        postal: "D02 XY45",
        city: "Dublin",
        country: "IE",
        vat: VatSpec::Fixed("IE1234567A"),
        peppol: PeppolSpec::Fixed("0213:1234567A"),
        currency: None,
        contact: "Aoife Murphy",
        email: "aoife@dublincloud.example",
    },
    CustomerSpec {
        name: "København Grøn Energi ApS",
        street: "Vesterbrogade 88",
        postal: "1620",
        city: "København",
        country: "DK",
        vat: VatSpec::Fixed("DK12345678"),
        peppol: PeppolSpec::Fixed("0205:12345678"),
        currency: None,
        contact: "Freja Nielsen",
        email: "freja@groenenergi.example",
    },
    CustomerSpec {
        name: "Nordic Fjord Consulting AB",
        street: "Sveavägen 44",
        postal: "111 34",
        city: "Stockholm",
        country: "SE",
        vat: VatSpec::Fixed("SE123456789012"),
        peppol: PeppolSpec::Fixed("0184:123456789012"),
        currency: None,
        contact: "Elin Lindqvist",
        email: "elin@nordicfjord.example",
    },
    CustomerSpec {
        name: "Helsinki Design Lab Oy",
        street: "Kalevankatu 19",
        postal: "00100",
        city: "Helsinki",
        country: "FI",
        vat: VatSpec::Fixed("FI12345678"),
        peppol: PeppolSpec::Fixed("0212:12345678"),
        currency: None,
        contact: "Aino Korhonen",
        email: "aino@helsinkidesign.example",
    },
    CustomerSpec {
        name: "Wien Kultur Projekte GmbH",
        street: "Mariahilfer Straße 101",
        postal: "1060",
        city: "Wien",
        country: "AT",
        vat: VatSpec::Fixed("ATU12345678"),
        peppol: PeppolSpec::Fixed("0204:ATU12345678"),
        currency: None,
        contact: "Felix Gruber",
        email: "felix@wienkultur.example",
    },
    CustomerSpec {
        name: "Brighton Editorial Ltd",
        street: "23 North Laine",
        postal: "BN1 1YD",
        city: "Brighton",
        country: "GB",
        vat: VatSpec::Fixed("GB123456789"),
        peppol: PeppolSpec::Email("accounts@brighton-editorial.example"),
        currency: Some("GBP"),
        contact: "Oliver Grant",
        email: "oliver@brighton-editorial.example",
    },
    CustomerSpec {
        name: "Northwind Analytics Inc.",
        street: "88 Beacon Street",
        postal: "02108",
        city: "Boston",
        country: "US",
        vat: VatSpec::None,
        peppol: PeppolSpec::Email("ap@northwind-analytics.example"),
        currency: Some("USD"),
        contact: "Dana Whitfield",
        email: "dana@northwind-analytics.example",
    },
];

/// Builds a customer party, deriving identifiers where the catalogue says so.
fn party_from(spec: &CustomerSpec, rng: &mut Rng) -> Result<Party, DemoError> {
    let country = CountryCode::parse(spec.country)?;
    let mut party = Party::new(
        spec.name,
        Address {
            street: Some(spec.street.to_owned()),
            city: spec.city.to_owned(),
            postal_code: Some(spec.postal.to_owned()),
            country,
        },
    );
    party.contact_name = Some(spec.contact.to_owned());
    party.email = Some(spec.email.to_owned());

    let (vat, kvk) = match spec.vat {
        VatSpec::Dutch => {
            let kvk = rng.range(10_000_000, 99_999_999).to_string();
            (Some(dutch_vat(rng)), Some(kvk))
        }
        VatSpec::Fixed(value) => (Some(value.to_owned()), None),
        VatSpec::None => (None, None),
    };
    let peppol = match spec.peppol {
        PeppolSpec::Dutch => {
            let Some(kvk) = kvk.as_deref() else {
                return Err(DemoError::Input(format!("{} has no KvK number", spec.name)));
            };
            format!("0106:{kvk}")
        }
        PeppolSpec::Fixed(value) => value.to_owned(),
        PeppolSpec::Email(value) => format!("EM:{value}"),
    };

    party.vat_id = vat;
    party.kvk_number = kvk;
    party.peppol_id = Some(peppol);
    Ok(party)
}

/// A supplier that is deliberately not in the party table.
struct SupplierSpec {
    name: &'static str,
    street: &'static str,
    postal: &'static str,
    city: &'static str,
    country: &'static str,
    vat: &'static str,
    endpoint: &'static str,
    prefix: &'static str,
    currency: &'static str,
}

const UNMATCHED_SUPPLIERS: [SupplierSpec; 6] = [
    SupplierSpec {
        name: "Fjord Papir AS",
        street: "Akersgata 55",
        postal: "0180",
        city: "Oslo",
        country: "NO",
        vat: "NO999888777MVA",
        endpoint: "0192:999888777",
        prefix: "FP",
        currency: "EUR",
    },
    SupplierSpec {
        name: "Iberia Print Solutions S.L.",
        street: "Avenida de la Constitución 12",
        postal: "41004",
        city: "Sevilla",
        country: "ES",
        vat: "ESB98877665",
        endpoint: "9920:B98877665",
        prefix: "IPS",
        currency: "EUR",
    },
    SupplierSpec {
        name: "Helvetia Software AG",
        street: "Bahnhofstrasse 70",
        postal: "8001",
        city: "Zürich",
        country: "CH",
        vat: "CHE123456789",
        endpoint: "EM:billing@helvetia-software.example",
        prefix: "HS",
        currency: "EUR",
    },
    SupplierSpec {
        name: "Baltic Freight OÜ",
        street: "Sadama 5",
        postal: "10111",
        city: "Tallinn",
        country: "EE",
        vat: "EE123456789",
        endpoint: "0193:123456789",
        prefix: "BF",
        currency: "EUR",
    },
    SupplierSpec {
        name: "Lusitânia Eventos Lda",
        street: "Rua Augusta 210",
        postal: "1100-053",
        city: "Lisboa",
        country: "PT",
        vat: "PT123456789",
        endpoint: "EM:contas@lusitania-eventos.example",
        prefix: "LE",
        currency: "EUR",
    },
    SupplierSpec {
        name: "Maple Ridge Packaging Inc.",
        street: "400 King Street West",
        postal: "M5V 1K2",
        city: "Toronto",
        country: "CA",
        vat: "CA123456789",
        endpoint: "EM:ar@mapleridge-packaging.example",
        prefix: "MRP",
        currency: "USD",
    },
];

/// Builds a supplier party. None of these are written to the party table.
fn supplier_party(spec: &SupplierSpec) -> Result<Party, DemoError> {
    let country = CountryCode::parse(spec.country)?;
    let mut party = Party::new(
        spec.name,
        Address {
            street: Some(spec.street.to_owned()),
            city: spec.city.to_owned(),
            postal_code: Some(spec.postal.to_owned()),
            country,
        },
    );
    party.vat_id = Some(spec.vat.to_owned());
    party.peppol_id = Some(spec.endpoint.to_owned());
    party.email = Some(format!("billing@{}", slug(spec.name)));
    Ok(party)
}

/// A deterministic Dutch VAT identifier that passes the eleven-proof.
fn dutch_vat(rng: &mut Rng) -> String {
    loop {
        let mut digits = [0_u8; 8];
        for digit in &mut digits {
            *digit = rng.below(10) as u8;
        }
        let sum: u32 = digits
            .iter()
            .enumerate()
            .map(|(index, digit)| u32::from(*digit) * (9 - index as u32))
            .sum();
        let check = sum % 11;
        if check <= 9 {
            let number: String = digits.iter().map(|digit| char::from(b'0' + digit)).collect();
            return format!("NL{number}{check}B01");
        }
    }
}

/// The year prefix used in document numbers.
fn year_of(date: LocalDate) -> String {
    date.to_iso_string().chars().take(4).collect()
}

/// An RFC 3339 timestamp on a business date, for a backdated audit event.
fn at(date: LocalDate, hour: u8, minute: u8) -> String {
    format!("{}T{hour:02}:{minute:02}:00.000000Z", date.to_iso_string())
}

fn earliest(left: LocalDate, right: LocalDate) -> LocalDate {
    if left < right { left } else { right }
}

fn slug(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

/// A small deterministic generator. Same seed, same data; no dependency and no
/// floats, because the amounts it drives must stay exact.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed ^ 0xA5A5_5A5A_C3C3_3C3C)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 { 0 } else { self.next() % bound }
    }

    /// An inclusive range.
    fn range(&mut self, low: i64, high: i64) -> i64 {
        let span = u64::try_from(high - low + 1).unwrap_or(1);
        low + i64::try_from(self.below(span)).unwrap_or(0)
    }
}

/// Why the demo archive could not be written.
#[derive(Debug, thiserror::Error)]
pub enum DemoError {
    /// Storage refused a step.
    #[error(transparent)]
    Store(#[from] store::StoreError),
    /// A domain value was rejected.
    #[error(transparent)]
    Domain(#[from] crux::DomainError),
    /// The import pipeline refused a generated document.
    #[error(transparent)]
    Import(#[from] import::ImportError),
    /// The options do not describe a usable archive.
    #[error("{0}")]
    Input(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use store::SearchRequest;

    const PASS: &str = "demo-test-passphrase";

    fn create(directory: &std::path::Path) -> Session {
        Session::create(directory.join("bonk.db"), directory.join("keystore.bin"), PASS).unwrap()
    }

    fn owner(session: &Session) -> SellerId {
        store::first_seller(session.database().connection())
            .unwrap()
            .expect("the demo has a seller")
            .id
    }

    fn options(invoices: usize, received: usize) -> Options {
        Options {
            customers: 6,
            invoices,
            received,
            seed: 99,
            today: Some(LocalDate::parse_iso("2026-10-28").unwrap()),
            close_last_year: true,
        }
    }

    fn all_hits(session: &Session, seller: SellerId) -> Vec<store::SearchHit> {
        store::search(
            session.database().connection(),
            seller,
            &SearchRequest {
                query: None,
                direction: None,
                from: None,
                to: None,
                status: None,
            },
        )
        .unwrap()
    }

    #[test]
    fn seeds_a_verifiable_archive() {
        let directory = tempfile::tempdir().unwrap();
        let mut session = create(directory.path());
        let seeded = seed(&mut session, &options(40, 10)).unwrap();

        assert_eq!(seeded.customers, 6);
        assert_eq!(seeded.invoices + seeded.credit_notes, 40);
        assert!(seeded.credit_notes >= 1, "the credit-note path was not exercised");
        assert_eq!(seeded.received, 10);
        assert!(seeded.matched > 0, "no supplier matched");
        assert!(seeded.parked > 0, "no supplier was parked");
        assert_eq!(seeded.drafts, 4);
        assert_eq!(seeded.views, 4);

        session.integrity_check().unwrap();
        let connection = session.database().connection();
        assert!(store::audit::verify_chain(connection).unwrap() > 0);
        // One UBL per issued document, and one page per received one: the demo
        // imports its inbound documents through the same path a user does.
        assert_eq!(store::verify_artefacts(connection).unwrap(), 50);

        let report = store::integrity::audit(connection).unwrap();
        assert_eq!(report.artefacts, 50);
        assert_eq!(report.imports, 10);
        assert_eq!(report.blobs, 60);
        assert_eq!(report.orphaned_blobs, 0);

        let received = store::list_received(connection, owner(&session)).unwrap();
        assert_eq!(received.len(), 10);
        assert!(
            received.iter().any(|row| row.status != DocumentStatus::Received),
            "no received document moved through the register"
        );

        let hits = all_hits(&session, owner(&session));
        // The four drafts are listed alongside the issued and received documents:
        // unfinished work is searchable, which is the only way to find it again.
        assert_eq!(hits.len(), 54);
        assert_eq!(
            hits.iter().filter(|hit| hit.status == DocumentStatus::Draft).count(),
            seeded.drafts
        );
        assert!(hits.iter().any(|hit| hit.number.contains("-C")));
        for hit in hits.iter().filter(|hit| hit.status != DocumentStatus::Draft) {
            assert!(
                store::archive::load(connection, hit.id).unwrap().is_some(),
                "{} cannot be read back from the archive",
                hit.number
            );
        }

        let views = store::saved::list(connection, owner(&session)).unwrap();
        assert_eq!(views.len(), 4);
    }

    #[test]
    fn the_demo_archive_carries_a_queue_to_work() {
        let directory = tempfile::tempdir().unwrap();
        let mut session = create(directory.path());
        let seeded = seed(&mut session, &options(40, 10)).unwrap();
        let connection = session.database().connection();
        let owner = owner(&session);
        let today = LocalDate::parse_iso("2026-10-28").unwrap();

        // The documents post, and the books they post to balance.
        assert!(seeded.posted > 0, "nothing was posted");
        let report = store::trial_balance(connection, owner, today.plus_days(-450).unwrap(), today).unwrap();
        assert!(report.balances(), "the demo ledger does not net to zero");

        // There is money to work with on both sides of the subledger, and a bank
        // account whose statement is waiting.
        for kind in [store::OpenItemKind::Receivable, store::OpenItemKind::Payable] {
            let open = store::open_items::list(connection, owner, kind)
                .unwrap()
                .into_iter()
                .filter(store::OpenItem::is_open)
                .count();
            assert!(open > 0, "nothing is open on the {} side", kind.as_str());
        }
        assert_eq!(store::list_bank_accounts(connection, owner).unwrap().len(), 1);
        assert_eq!(
            store::list_transactions(connection, owner, Some("new")).unwrap().len(),
            seeded.bank_lines,
            "every demo transaction is waiting in the queue"
        );
        assert_eq!(store::bank_rule::list(connection, owner).unwrap().len(), seeded.rules);

        // Nothing is left unclassified, so a period can actually be closed — which is
        // what `9999` is for: to be emptied, not lived in.
        let unclassified: i64 = connection
            .query_row(
                "SELECT COALESCE(sum(CASE side WHEN 'debit' THEN amount_micros ELSE -amount_micros END), 0)
                 FROM journal_line WHERE account_code = '9999'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(unclassified, 0, "the demo leaves costs on Nog te rubriceren");

        let period: String = connection
            .query_row(
                "SELECT id FROM fiscal_period ORDER BY year DESC, period_no DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let stamp = at(today, 13, 0);
        store::lock_period(
            connection,
            crux::FiscalPeriodId::parse_str(&period).unwrap(),
            ACTOR,
            &stamp,
        )
        .unwrap();
    }

    /// The archive holds two kinds of year: last year finished, this one in progress,
    /// so the window has a closed year to open and an actionable refusal on the other.
    #[test]
    fn the_demo_archive_has_a_finished_year_and_an_open_one() {
        let directory = tempfile::tempdir().unwrap();
        let mut session = create(directory.path());
        let seeded = seed(&mut session, &options(40, 10)).unwrap();
        let connection = session.database().connection();
        let owner = owner(&session);
        let today = LocalDate::parse_iso("2026-10-28").unwrap();
        let last = today.as_date().year() - 1;

        assert_eq!(seeded.closed_year, Some(last), "last year is closed by the seed");
        assert!(store::year_is_closed(connection, owner, last).unwrap());
        assert!(
            !store::year_is_closed(connection, owner, today.as_date().year()).unwrap(),
            "this year is still open"
        );

        // The closed year's months are locked and its result is on equity.
        for period in store::list_periods(connection, owner, last).unwrap() {
            assert!(!period.is_open(), "{}-{:02} is open", last, period.period_no);
        }
        let equity: i64 = connection
            .query_row(
                "SELECT COALESCE(sum(CASE side WHEN 'debit' THEN amount_micros ELSE -amount_micros END), 0)
                 FROM journal_line WHERE account_code = ?1",
                [store::RESULT_ACCOUNT],
                |row| row.get(0),
            )
            .unwrap();
        assert_ne!(equity, 0, "the resultaatbestemming put last year's result on equity");

        // And it refuses a posting, which is what a closed year is for.
        let said = store::post_manual_entry(
            connection,
            owner,
            today.plus_days(-400).unwrap(),
            "Te laat",
            vec![
                crux::LedgerLine::debit(
                    crux::AccountCode::parse("7100").unwrap(),
                    Money::from_major_units(5, Currency::EUR).unwrap(),
                )
                .unwrap(),
                crux::LedgerLine::credit(
                    crux::AccountCode::parse("1100").unwrap(),
                    Money::from_major_units(5, Currency::EUR).unwrap(),
                )
                .unwrap(),
            ],
            ACTOR,
            &at(today, 13, 0),
        )
        .unwrap_err()
        .to_string();
        assert!(said.contains("is closed"), "{said}");
    }

    /// The demo's rule books the fees and nothing else.
    ///
    /// This is the property the seed has to keep: an archive that demonstrates the
    /// queue must not also demonstrate a rule taking a customer's payment, so the
    /// guards are checked against the data rather than assumed.
    #[test]
    fn the_demo_rule_takes_the_fees_and_leaves_the_payments_alone() {
        let directory = tempfile::tempdir().unwrap();
        let mut session = create(directory.path());
        seed(&mut session, &options(40, 10)).unwrap();
        let owner = owner(&session);

        let stamp = at(LocalDate::parse_iso("2026-10-28").unwrap(), 12, 0);
        let outcomes = store::bank_rule::classify(session.database().connection(), owner, true, ACTOR, &stamp).unwrap();
        let posted: Vec<&store::bank_rule::Classified> = outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                store::bank_rule::Classification::Posted(classified) => Some(classified),
                _ => None,
            })
            .collect();
        assert_eq!(posted.len(), 2, "the two fee lines are the only ones a rule fits");
        for classified in &posted {
            assert_eq!(classified.rule, "bankkosten");
            assert_eq!(classified.amount.currency(), Currency::EUR);
            assert_eq!(classified.amount.micros(), 12_500_000, "a fee, not a payment");
        }

        // The lines that name an invoice are proposed for it, which is what keeps a
        // rule away from them.
        let reserved = outcomes
            .iter()
            .filter(|outcome| matches!(outcome, store::bank_rule::Classification::Reserved { .. }))
            .count();
        assert!(reserved >= 3, "the payments should be reserved, saw {reserved}");
        assert!(
            outcomes
                .iter()
                .any(|outcome| matches!(outcome, store::bank_rule::Classification::NoRule { .. })),
            "the interest line is nobody's invoice and no rule's business"
        );
    }

    #[test]
    fn the_same_seed_produces_the_same_documents() {
        let first_dir = tempfile::tempdir().unwrap();
        let second_dir = tempfile::tempdir().unwrap();
        let mut first = create(first_dir.path());
        let mut second = create(second_dir.path());
        let settings = options(25, 6);
        let first_seed = seed(&mut first, &settings).unwrap();
        let second_seed = seed(&mut second, &settings).unwrap();
        assert_eq!(first_seed.invoices, second_seed.invoices);
        assert_eq!(first_seed.credit_notes, second_seed.credit_notes);

        let shape = |session: &Session, seller: SellerId| {
            let mut rows: Vec<(String, String, String)> = all_hits(session, seller)
                .into_iter()
                .map(|hit| (hit.number, hit.party, hit.gross.to_fixed_string(2).unwrap_or_default()))
                .collect();
            rows.sort();
            rows
        };

        assert_eq!(
            shape(&first, owner(&first)),
            shape(&second, owner(&second)),
            "the generator is not deterministic"
        );
    }
}
