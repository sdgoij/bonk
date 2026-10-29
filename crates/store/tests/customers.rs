#![allow(clippy::unwrap_used)]
//! Editing a customer touches the party on file and nothing else.

use crux::{Address, CountryCode, CustomerId, Party};
use store::{
    CustomerRecord, Database, KdfParams, KeyStore, StoreError, insert_customer, load_customer, update_customer,
};
use tempfile::TempDir;

const NOW: &str = "2026-10-29T09:15:00.123456Z";

fn database() -> (TempDir, Database) {
    let dir = TempDir::new().unwrap();
    let keystore = KeyStore::create(dir.path().join("keystore.bin"), "passphrase", KdfParams::TEST).unwrap();
    let database = Database::create(dir.path().join("bonk.db"), &keystore).unwrap();
    (dir, database)
}

fn party() -> Party {
    Party::new("Oude Naam BV", Address::new("Utrecht", CountryCode::NL))
}

fn revision(database: &Database, id: CustomerId) -> i64 {
    database
        .connection()
        .query_row("SELECT revision FROM customer WHERE id = ?1", [id.to_string()], |row| {
            row.get(0)
        })
        .unwrap()
}

#[test]
fn an_edit_replaces_every_field_on_file() {
    let (_dir, database) = database();
    let id = CustomerId::new();
    insert_customer(database.connection(), &CustomerRecord { id, party: party() }, NOW).unwrap();

    let mut edited = Party::new(
        "Nieuwe Naam BV",
        Address {
            street: Some("Damrak 1".to_owned()),
            city: "Amsterdam".to_owned(),
            postal_code: Some("1012 LG".to_owned()),
            country: CountryCode::NL,
        },
    );
    edited.vat_id = Some("NL123456782B01".to_owned());
    edited.kvk_number = Some("12345678".to_owned());
    edited.peppol_id = Some("0106:12345678".to_owned());
    edited.contact_name = Some("Ada".to_owned());
    edited.email = Some("ada@example.test".to_owned());
    update_customer(database.connection(), &CustomerRecord { id, party: edited }, NOW).unwrap();

    let loaded = load_customer(database.connection(), id).unwrap();
    assert_eq!(loaded.party.legal_name, "Nieuwe Naam BV");
    assert_eq!(loaded.party.address.street.as_deref(), Some("Damrak 1"));
    assert_eq!(loaded.party.address.city, "Amsterdam");
    assert_eq!(loaded.party.address.postal_code.as_deref(), Some("1012 LG"));
    assert_eq!(loaded.party.vat_id.as_deref(), Some("NL123456782B01"));
    assert_eq!(loaded.party.kvk_number.as_deref(), Some("12345678"));
    assert_eq!(loaded.party.peppol_id.as_deref(), Some("0106:12345678"));
    assert_eq!(loaded.party.contact_name.as_deref(), Some("Ada"));
    assert_eq!(loaded.party.email.as_deref(), Some("ada@example.test"));
}

#[test]
fn an_edit_bumps_the_revision() {
    let (_dir, database) = database();
    let id = CustomerId::new();
    insert_customer(database.connection(), &CustomerRecord { id, party: party() }, NOW).unwrap();

    let before = revision(&database, id);
    update_customer(database.connection(), &CustomerRecord { id, party: party() }, NOW).unwrap();
    assert_eq!(revision(&database, id), before + 1);
}

#[test]
fn editing_an_unknown_customer_is_an_error() {
    let (_dir, database) = database();
    let id = CustomerId::new();
    let error = update_customer(database.connection(), &CustomerRecord { id, party: party() }, NOW).unwrap_err();
    assert!(matches!(error, StoreError::NotFound(_)), "{error}");
}
