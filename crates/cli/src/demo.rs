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

use app::Session;
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

    for (index, date) in dates.into_iter().enumerate() {
        let mut credit = false;
        if index > 0 && index % 20 == 0 {
            credit = issue_credit_note(session, owner, &issued, date, today, &mut rng)?;
        }
        if credit {
            credit_count += 1;
        } else {
            let customer = &customers[rng.below(customers.len() as u64) as usize];
            let draft = invoice_draft(owner, customer, date, &mut rng)?;
            let record = issue(session, &draft, &at(date, 9, 0))?;
            issued.push(Issued {
                number: record.number,
                date,
            });
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

    Ok(Seeded {
        customers: customers.len(),
        invoices: invoice_count,
        credit_notes: credit_count,
        received: inbound.received,
        matched: inbound.matched,
        parked: inbound.parked,
        drafts,
        views,
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
) -> Result<bool, DemoError> {
    if issued.is_empty() {
        return Ok(false);
    }
    let previous = &issued[rng.below(issued.len() as u64) as usize];
    if previous.date >= today {
        return Ok(false);
    }

    let earliest = previous.date.plus_days(1)?;
    let credit_date = if date > earliest { date } else { earliest };
    let Some(draft) = credit_note_for(session.database().connection(), owner, &previous.number, credit_date)? else {
        return Ok(false);
    };

    issue(session, &draft, &at(credit_date, 10, 0))?;
    Ok(true)
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
        let bytes = app::render_issue(&view)?;

        let imported_at = at(earliest(today, date.plus_days(1)?), 8, 30);
        let path = format!("demo/inbound/{number}.xml");
        let outcome = import::import_bytes(session.database(), owner, Some(&path), &bytes, &imported_at)?;
        if outcome.supplier.is_some() {
            matched += 1;
        } else {
            parked += 1;
        }

        advance_payable(session, outcome.document_id, date, days_ago, today, rng)?;
    }

    Ok(InboundCounts {
        received: count,
        matched,
        parked,
    })
}

/// Moves a received document as far through the register as its age suggests.
fn advance_payable(
    session: &Session,
    document: DocumentId,
    date: LocalDate,
    days_ago: i64,
    today: LocalDate,
    rng: &mut Rng,
) -> Result<(), DemoError> {
    let steps: &[DocumentStatus] = if days_ago >= 150 {
        &[DocumentStatus::Approved, DocumentStatus::Paid, DocumentStatus::Archived]
    } else if days_ago >= 75 {
        &[DocumentStatus::Approved, DocumentStatus::Paid]
    } else if days_ago >= 30 {
        &[DocumentStatus::Approved]
    } else {
        &[]
    };

    // Some old documents stay open on purpose: an archive where everything is
    // settled does not exercise the "open purchases" view.
    let steps = if !steps.is_empty() && rng.below(6) == 0 {
        &steps[..steps.len() - 1]
    } else {
        steps
    };

    for (index, status) in steps.iter().enumerate() {
        let days = 15 * (index as i64 + 1);
        let when = earliest(today, date.plus_days(days)?);
        advance_status(
            session.database().connection(),
            document,
            *status,
            ACTOR,
            &at(when, 12, 0),
        )?;
    }
    Ok(())
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
        app::render_issue,
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
struct Issued {
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
    format!("{}T{hour:02}:{minute:02}:00Z", date.to_iso_string())
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
        assert_eq!(store::verify_artefacts(connection).unwrap(), 40);

        let report = store::integrity::audit(connection).unwrap();
        assert_eq!(report.artefacts, 40);
        assert_eq!(report.imports, 10);
        assert_eq!(report.blobs, 50);
        assert_eq!(report.orphaned_blobs, 0);

        let received = store::list_received(connection, owner(&session)).unwrap();
        assert_eq!(received.len(), 10);
        assert!(
            received.iter().any(|row| row.status != DocumentStatus::Received),
            "no received document moved through the register"
        );

        let hits = all_hits(&session, owner(&session));
        assert_eq!(hits.len(), 50);
        assert!(hits.iter().any(|hit| hit.number.contains("-C")));
        for hit in &hits {
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
