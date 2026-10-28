#![allow(clippy::unwrap_used)]
//! Saved find-panel filters: a name, and the filters it stands for.

use crux::{Address, CountryCode, Party, SavedSearchId, SellerId};
use store::{
    Database, Direction, DocumentStatus, KdfParams, KeyStore, SavedSearch, SellerRecord, insert_seller, saved,
};
use tempfile::TempDir;

const NOW: &str = "2026-10-27T12:00:00Z";

fn fixture() -> (TempDir, Database, SellerId) {
    let dir = TempDir::new().unwrap();
    let keystore = KeyStore::create(dir.path().join("keystore.bin"), "passphrase", KdfParams::TEST).unwrap();
    let database = Database::create(dir.path().join("bonk.db"), &keystore).unwrap();
    let seller_id = SellerId::new();
    insert_seller(
        database.connection(),
        &SellerRecord {
            id: seller_id,
            party: Party::new("Seller BV", Address::new("Amsterdam", CountryCode::NL)),
            default_currency: crux::Currency::EUR,
            default_locale: "nl-NL".to_owned(),
        },
        NOW,
    )
    .unwrap();
    (dir, database, seller_id)
}

fn search(name: &str, query: &str) -> SavedSearch {
    SavedSearch {
        id: SavedSearchId::new(),
        name: name.to_owned(),
        query: query.to_owned(),
        direction: Some(Direction::Inbound),
        status: Some(DocumentStatus::Approved),
    }
}

#[test]
fn a_saved_search_round_trips() {
    let (_dir, database, seller_id) = fixture();
    saved::save(database.connection(), seller_id, &search("Unpaid", "acme"), NOW).unwrap();

    let listed = saved::list(database.connection(), seller_id).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "Unpaid");
    assert_eq!(listed[0].query, "acme");
    assert_eq!(listed[0].direction, Some(Direction::Inbound));
    assert_eq!(listed[0].status, Some(DocumentStatus::Approved));
}

#[test]
fn saving_under_the_same_name_replaces_the_filters() {
    let (_dir, database, seller_id) = fixture();
    let first = search("Unpaid", "acme");
    saved::save(database.connection(), seller_id, &first, NOW).unwrap();

    let mut second = search("Unpaid", "globex");
    second.status = None;
    saved::save(database.connection(), seller_id, &second, NOW).unwrap();

    let listed = saved::list(database.connection(), seller_id).unwrap();
    assert_eq!(listed.len(), 1, "the name is the identity the user sees");
    assert_eq!(listed[0].id, first.id, "editing keeps the row it started as");
    assert_eq!(listed[0].query, "globex");
    assert_eq!(listed[0].status, None);
}

#[test]
fn deleting_removes_only_the_named_one() {
    let (_dir, database, seller_id) = fixture();
    saved::save(database.connection(), seller_id, &search("Unpaid", "acme"), NOW).unwrap();
    saved::save(database.connection(), seller_id, &search("Drafts", ""), NOW).unwrap();

    assert!(saved::delete(database.connection(), seller_id, "Unpaid").unwrap());
    let listed = saved::list(database.connection(), seller_id).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "Drafts");

    // A second delete of the same name is a race, not an error.
    assert!(!saved::delete(database.connection(), seller_id, "Unpaid").unwrap());
}

#[test]
fn a_saved_search_needs_a_name() {
    let (_dir, database, seller_id) = fixture();
    let error = saved::save(database.connection(), seller_id, &search("   ", "acme"), NOW).unwrap_err();
    assert!(error.to_string().contains("needs a name"), "{error}");
}

#[test]
fn saves_belong_to_one_seller() {
    let (_dir, database, seller_id) = fixture();
    let other = SellerId::new();
    insert_seller(
        database.connection(),
        &SellerRecord {
            id: other,
            party: Party::new("Other BV", Address::new("Utrecht", CountryCode::NL)),
            default_currency: crux::Currency::EUR,
            default_locale: "nl-NL".to_owned(),
        },
        NOW,
    )
    .unwrap();

    saved::save(database.connection(), seller_id, &search("Mine", "acme"), NOW).unwrap();
    assert!(saved::list(database.connection(), other).unwrap().is_empty());
}
