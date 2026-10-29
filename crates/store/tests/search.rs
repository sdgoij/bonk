#![allow(clippy::unwrap_used)]
//! Search: words find documents, filters narrow them, and the index follows the
//! facts it is derived from.

use crux::{
    Address, CountryCode, Currency, CustomerId, DraftLine, InvoiceDraft, LineId, LocalDate, Money, Party, Quantity,
    SellerId, TaxTreatment, UnitCode, VatRate,
};
use store::{
    CustomerRecord, Database, Direction, IssueRequest, KdfParams, KeyStore, SearchRequest, SellerRecord,
    insert_customer, insert_draft, insert_seller, issue_draft, search,
};
use tempfile::TempDir;

const NOW: &str = "2026-10-27T12:00:00Z";

fn party(name: &str) -> Party {
    Party::new(name, Address::new("Amsterdam", CountryCode::NL))
}

fn fixture() -> (TempDir, Database, SellerId, CustomerId, CustomerId) {
    let dir = TempDir::new().unwrap();
    let keystore = KeyStore::create(dir.path().join("keystore.bin"), "passphrase", KdfParams::TEST).unwrap();
    let database = Database::create(dir.path().join("bonk.db"), &keystore).unwrap();

    let seller_id = SellerId::new();
    let customer_id = CustomerId::new();
    let cafe_id = CustomerId::new();

    insert_seller(
        database.connection(),
        &SellerRecord {
            id: seller_id,
            party: party("Seller BV"),
            default_currency: Currency::EUR,
            default_locale: "nl-NL".to_owned(),
        },
        NOW,
    )
    .unwrap();
    for (id, name) in [(customer_id, "Customer BV"), (cafe_id, "Café Zuid BV")] {
        insert_customer(database.connection(), &CustomerRecord { id, party: party(name) }, NOW).unwrap();
    }

    (dir, database, seller_id, customer_id, cafe_id)
}

fn issue(
    database: &mut Database,
    seller: SellerId,
    customer: CustomerId,
    reference: &str,
    note: &str,
    date: &str,
) -> crux::DocumentId {
    let mut draft = InvoiceDraft::new(seller, customer, Currency::EUR, LocalDate::parse_iso(date).unwrap());
    draft.buyer_reference = Some(reference.to_owned());
    draft.note = Some(note.to_owned());
    draft.lines.push(DraftLine {
        id: LineId::new(),
        position: 1,
        name: "Consulting".to_owned(),
        description: None,
        quantity: Quantity::ONE,
        unit: UnitCode::PIECE,
        unit_price: Money::from_major_units(100, Currency::EUR).unwrap(),
        price_base_quantity: Quantity::ONE,
        treatment: TaxTreatment::standard(VatRate::STANDARD_NL),
    });
    insert_draft(database.connection(), &draft, NOW).unwrap();

    issue_draft(
        database.connection_mut(),
        IssueRequest {
            draft_id: draft.id,
            actor: "test",
            device: "test",
            renderer_version: "test",
            now: NOW,
        },
        |_| Ok(b"<Invoice/>".to_vec()),
    )
    .unwrap()
    .id
}

fn find(database: &Database, seller: SellerId, request: SearchRequest<'_>) -> Vec<store::SearchHit> {
    search(database.connection(), seller, &request).unwrap()
}

/// Stores a draft without issuing it.
fn draft(database: &Database, seller: SellerId, customer: CustomerId, note: &str, date: &str) -> crux::DocumentId {
    let mut draft = InvoiceDraft::new(seller, customer, Currency::EUR, LocalDate::parse_iso(date).unwrap());
    draft.note = Some(note.to_owned());
    draft.lines.push(DraftLine {
        id: LineId::new(),
        position: 1,
        name: "Advies".to_owned(),
        description: None,
        quantity: Quantity::ONE,
        unit: UnitCode::PIECE,
        unit_price: Money::from_major_units(100, Currency::EUR).unwrap(),
        price_base_quantity: Quantity::ONE,
        treatment: TaxTreatment::standard(VatRate::STANDARD_NL),
    });
    let id = draft.id;
    store::insert_draft(database.connection(), &draft, NOW).unwrap();
    id
}

#[test]
fn a_draft_is_findable_and_carries_its_own_totals() {
    let (_dir, database, seller, customer, _cafe) = fixture();
    let draft_id = draft(&database, seller, customer, "Concept voor mei", "2026-10-20");

    let hits = find(&database, seller, SearchRequest::default());
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, draft_id);
    assert_eq!(hits[0].status, store::DocumentStatus::Draft);
    assert_eq!(hits[0].party, "Customer BV");
    assert_eq!(hits[0].direction, Direction::Outbound);
    assert_eq!(hits[0].gross.micros(), 121_000_000, "100 net plus 21%");

    let hits = find(
        &database,
        seller,
        SearchRequest {
            query: Some("concept"),
            ..SearchRequest::default()
        },
    );
    assert_eq!(hits.len(), 1, "the note is searched too");
}

#[test]
fn issuing_a_draft_removes_it_from_the_list() {
    let (_dir, mut database, seller, customer, _cafe) = fixture();
    let id = draft(&database, seller, customer, "Concept", "2026-10-20");

    issue_draft(
        database.connection_mut(),
        IssueRequest {
            draft_id: id,
            actor: "test",
            device: "test",
            renderer_version: "test",
            now: NOW,
        },
        |_| Ok(b"<Invoice/>".to_vec()),
    )
    .unwrap();

    assert!(
        store::load_draft(database.connection(), id).unwrap().is_none(),
        "the draft became the document"
    );
    let hits = find(&database, seller, SearchRequest::default());
    assert_eq!(hits.len(), 1);
    assert_ne!(hits[0].status, store::DocumentStatus::Draft);
}

#[test]
fn the_footer_counts_what_the_filters_match_and_what_is_there() {
    let (_dir, mut database, seller, customer, cafe) = fixture();
    issue(&mut database, seller, customer, "PO-1", "Advice", "2026-10-27");
    issue(&mut database, seller, cafe, "PO-2", "Review", "2026-09-01");
    draft(&database, seller, customer, "Concept", "2026-10-20");

    let counts = store::search_counts(database.connection(), seller, &SearchRequest::default()).unwrap();
    assert_eq!((counts.matched, counts.total), (3, 3));

    let counts = store::search_counts(
        database.connection(),
        seller,
        &SearchRequest {
            query: Some("2026-0002"),
            ..SearchRequest::default()
        },
    )
    .unwrap();
    assert_eq!((counts.matched, counts.total), (1, 3), "one match, three on file");

    let counts = store::search_counts(
        database.connection(),
        seller,
        &SearchRequest {
            direction: Some(Direction::Inbound),
            ..SearchRequest::default()
        },
    )
    .unwrap();
    assert_eq!((counts.matched, counts.total), (0, 3));
}

#[test]
fn a_draft_is_only_listed_where_it_belongs() {
    let (_dir, database, seller, customer, _cafe) = fixture();
    draft(&database, seller, customer, "Concept", "2026-10-20");

    // A draft has no number, so a number query cannot find it.
    let hits = find(
        &database,
        seller,
        SearchRequest {
            query: Some("2026-0001"),
            ..SearchRequest::default()
        },
    );
    assert!(hits.is_empty());

    // It is on our side of the books only.
    let hits = find(
        &database,
        seller,
        SearchRequest {
            direction: Some(Direction::Inbound),
            ..SearchRequest::default()
        },
    );
    assert!(hits.is_empty());

    // And the status filter can ask for drafts alone.
    let hits = find(
        &database,
        seller,
        SearchRequest {
            status: Some(store::DocumentStatus::Draft),
            ..SearchRequest::default()
        },
    );
    assert_eq!(hits.len(), 1);
}

#[test]
fn words_find_the_document_people_remember() {
    let (_dir, mut database, seller, customer, cafe) = fixture();
    let first = issue(
        &mut database,
        seller,
        customer,
        "PO-1",
        "Quarterly advice",
        "2026-10-27",
    );
    let second = issue(&mut database, seller, cafe, "PO-2", "Annual review", "2026-09-01");

    let hits = find(
        &database,
        seller,
        SearchRequest {
            query: Some("2026-0002"),
            ..SearchRequest::default()
        },
    );
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, second);
    assert_eq!(hits[0].party, "Café Zuid BV");
    assert_eq!(hits[0].direction, Direction::Outbound);
    assert_eq!(hits[0].gross.micros(), 121_000_000);
    assert_eq!(hits[0].currency, Currency::EUR);

    // A partly typed name still finds the party, and diacritics do not matter.
    let hits = find(
        &database,
        seller,
        SearchRequest {
            query: Some("cafe"),
            ..SearchRequest::default()
        },
    );
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, second);

    // The reference and the note are searchable too.
    for query in ["PO-1", "quarterly"] {
        let hits = find(
            &database,
            seller,
            SearchRequest {
                query: Some(query),
                ..SearchRequest::default()
            },
        );
        assert_eq!(hits.len(), 1, "{query}");
        assert_eq!(hits[0].id, first, "{query}");
    }
}

#[test]
fn filters_narrow_the_result_set() {
    let (_dir, mut database, seller, customer, cafe) = fixture();
    let first = issue(
        &mut database,
        seller,
        customer,
        "PO-1",
        "Quarterly advice",
        "2026-10-27",
    );
    let second = issue(&mut database, seller, cafe, "PO-2", "Annual review", "2026-09-01");

    let both = find(
        &database,
        seller,
        SearchRequest {
            query: Some("bv"),
            ..SearchRequest::default()
        },
    );
    assert_eq!(both.len(), 2, "both parties contain the word");

    let recent = find(
        &database,
        seller,
        SearchRequest {
            query: Some("bv"),
            from: Some(LocalDate::parse_iso("2026-10-01").unwrap()),
            ..SearchRequest::default()
        },
    );
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].id, first);

    let older = find(
        &database,
        seller,
        SearchRequest {
            query: Some("bv"),
            to: Some(LocalDate::parse_iso("2026-09-30").unwrap()),
            ..SearchRequest::default()
        },
    );
    assert_eq!(older.len(), 1);
    assert_eq!(older[0].id, second);

    let outbound = find(
        &database,
        seller,
        SearchRequest {
            query: Some("bv"),
            direction: Some(Direction::Outbound),
            ..SearchRequest::default()
        },
    );
    assert_eq!(outbound.len(), 2);
    let inbound = find(
        &database,
        seller,
        SearchRequest {
            query: Some("bv"),
            direction: Some(Direction::Inbound),
            ..SearchRequest::default()
        },
    );
    assert!(inbound.is_empty(), "nothing has been received yet");
}

#[test]
fn every_word_must_appear() {
    let (_dir, mut database, seller, customer, _cafe) = fixture();
    let first = issue(
        &mut database,
        seller,
        customer,
        "PO-1",
        "Quarterly advice",
        "2026-10-27",
    );

    let hits = find(
        &database,
        seller,
        SearchRequest {
            query: Some("customer quarterly"),
            ..SearchRequest::default()
        },
    );
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, first);

    let none = find(
        &database,
        seller,
        SearchRequest {
            query: Some("customer annual"),
            ..SearchRequest::default()
        },
    );
    assert!(none.is_empty(), "words are combined, not either-or");
}

#[test]
fn no_query_lists_everything_newest_first() {
    let (_dir, mut database, seller, customer, cafe) = fixture();
    let first = issue(
        &mut database,
        seller,
        customer,
        "PO-1",
        "Quarterly advice",
        "2026-10-27",
    );
    let second = issue(&mut database, seller, cafe, "PO-2", "Annual review", "2026-09-01");

    let all = find(&database, seller, SearchRequest::default());
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].id, first, "newest first");
    assert_eq!(all[1].id, second);
    assert_eq!(
        all[0].status,
        store::DocumentStatus::Issued,
        "nothing has happened to it yet"
    );

    let received = find(
        &database,
        seller,
        SearchRequest {
            direction: Some(Direction::Inbound),
            ..SearchRequest::default()
        },
    );
    assert!(received.is_empty());
}

#[test]
fn an_empty_query_is_refused() {
    let (_dir, database, seller, _customer, _cafe) = fixture();
    let error = search(
        database.connection(),
        seller,
        &SearchRequest {
            query: Some("   "),
            ..SearchRequest::default()
        },
    )
    .unwrap_err();
    assert!(matches!(error, store::StoreError::Rejected(_)), "{error}");
}

#[test]
fn a_quote_is_not_a_query_operator() {
    let (_dir, mut database, seller, customer, _cafe) = fixture();
    let first = issue(
        &mut database,
        seller,
        customer,
        "PO-1",
        "Quarterly advice",
        "2026-10-27",
    );

    // FTS5 would treat an unbalanced quote as a syntax error; a search box must not.
    let hits = find(
        &database,
        seller,
        SearchRequest {
            query: Some("\"Customer"),
            ..SearchRequest::default()
        },
    );
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, first);
}
