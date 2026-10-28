#![allow(clippy::unwrap_used)]
//! Backups: written encrypted, verified before anything is pruned, and
//! restorable on a clean machine.

use crux::{Address, CountryCode, Currency, Party, SellerId};
use store::backup::{self, RETENTION};
use store::{Database, KdfParams, KeyStore, SellerRecord, insert_seller};
use tempfile::TempDir;

const NOW: &str = "2026-10-28T12:00:00Z";
const CANARY: &str = "CANARY-Supplier-9d3f2";

fn fixture() -> (TempDir, Database, KeyStore) {
    let dir = TempDir::new().unwrap();
    let keystore = KeyStore::create(dir.path().join("keystore.bin"), "passphrase", KdfParams::TEST).unwrap();
    let database = Database::create(dir.path().join("bonk.db"), &keystore).unwrap();

    let mut party = Party::new(CANARY, Address::new("Amsterdam", CountryCode::NL));
    party.vat_id = Some("NL123456782B01".to_owned());
    insert_seller(
        database.connection(),
        &SellerRecord {
            id: SellerId::new(),
            party,
            default_currency: Currency::EUR,
            default_locale: "nl-NL".to_owned(),
        },
        NOW,
    )
    .unwrap();

    (dir, database, keystore)
}

fn stamps(count: usize) -> Vec<String> {
    (0..count).map(|index| format!("2026-10-28T12:00:0{index}Z")).collect()
}

#[test]
fn a_backup_verifies_and_restores_on_a_fresh_path() {
    let (dir, database, keystore) = fixture();
    let directory = dir.path().join("backups");

    let report = backup::backup(&database, &keystore, &directory, "2026-10-28T12:00:00Z", RETENTION).unwrap();
    assert!(report.path.exists());
    assert!(report.bytes > 0);
    assert!(report.removed.is_empty());

    // Verification reads only; it must not need the live database at all, and
    // the copy must hold the same chain as the original.
    let live = store::audit::verify_chain(database.connection()).unwrap();
    let events = backup::verify(&report.path, keystore.dek()).unwrap();
    assert_eq!(events, live, "the copy holds the same chain");

    // Restore is a file copy onto a clean machine, with the same keystore.
    let restored_path = dir.path().join("restored.db");
    std::fs::copy(&report.path, &restored_path).unwrap();
    let restored = Database::open(&restored_path, &keystore).unwrap();
    assert_eq!(restored.seller_count().unwrap(), 1);
    restored.integrity_check().unwrap();
    assert_eq!(store::audit::verify_chain(restored.connection()).unwrap(), live);
}

#[test]
fn a_backup_is_encrypted() {
    let (dir, database, keystore) = fixture();
    let report = backup::backup(
        &database,
        &keystore,
        dir.path().join("backups"),
        "2026-10-28T12:00:00Z",
        RETENTION,
    )
    .unwrap();

    let bytes = std::fs::read(&report.path).unwrap();
    #[cfg(feature = "sqlcipher")]
    {
        let haystack = String::from_utf8_lossy(&bytes);
        assert!(!haystack.contains(CANARY), "the backup contains plaintext");
        assert!(!bytes.starts_with(b"SQLite format 3"), "a plain SQLite file");

        // The key is what makes it readable; another keystore cannot open it.
        let other_dir = TempDir::new().unwrap();
        let other = KeyStore::create(other_dir.path().join("keystore.bin"), "other", KdfParams::TEST).unwrap();
        assert!(backup::verify(&report.path, other.dek()).is_err());
    }

    // The plain build exists so the stack can be developed without SQLCipher; it
    // writes an ordinary SQLite file on purpose, and says so here.
    #[cfg(not(feature = "sqlcipher"))]
    assert!(bytes.starts_with(b"SQLite format 3"), "the plain build is unencrypted");
}

#[test]
fn rotation_keeps_the_newest_and_reports_what_went() {
    let (dir, database, keystore) = fixture();
    let directory = dir.path().join("backups");

    for stamp in stamps(4) {
        backup::backup(&database, &keystore, &directory, &stamp, 2).unwrap();
    }

    let mut remaining: Vec<String> = std::fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    remaining.sort();
    assert_eq!(
        remaining,
        vec![
            "backup-20261028T120002Z.db".to_owned(),
            "backup-20261028T120003Z.db".to_owned(),
        ],
        "the newest two survive"
    );

    let report = backup::backup(&database, &keystore, &directory, "2026-10-28T12:00:04Z", 2).unwrap();
    assert_eq!(report.removed.len(), 1);
    assert!(report.removed[0].ends_with("backup-20261028T120002Z.db"));
}

#[test]
fn nothing_is_pruned_before_the_new_copy_verifies() {
    let (dir, database, keystore) = fixture();
    let directory = dir.path().join("backups");
    backup::backup(&database, &keystore, &directory, "2026-10-28T12:00:00Z", 1).unwrap();

    // The same stamp means the same destination, which already exists: the run
    // fails, and the existing backup must still be there.
    assert!(backup::backup(&database, &keystore, &directory, "2026-10-28T12:00:00Z", 1).is_err());
    let count = std::fs::read_dir(&directory).unwrap().count();
    assert_eq!(count, 1, "a refused run leaves the previous backup alone");
    backup::verify(directory.join("backup-20261028T120000Z.db"), keystore.dek()).unwrap();
}

#[test]
fn a_damaged_backup_is_detected() {
    let (dir, database, keystore) = fixture();
    let directory = dir.path().join("backups");
    let report = backup::backup(&database, &keystore, &directory, "2026-10-28T12:00:00Z", RETENTION).unwrap();

    let mut bytes = std::fs::read(&report.path).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0xff;
    std::fs::write(&report.path, &bytes).unwrap();

    assert!(
        backup::verify(&report.path, keystore.dek()).is_err(),
        "damage must not verify"
    );
}

#[test]
fn the_copy_holds_the_same_head_as_the_original() {
    let (dir, database, keystore) = fixture();
    let hash = store::audit::append(
        database.connection(),
        NOW,
        "test",
        "noted",
        "document",
        "d1",
        serde_json::json!({ "note": "canary" }),
    )
    .unwrap();

    let live = store::audit::head(database.connection()).unwrap().expect("a head");
    assert_eq!(live.hash, hash);
    assert_eq!(live.seq, 1);

    let report = backup::backup(
        &database,
        &keystore,
        dir.path().join("backups"),
        "2026-10-28T12:00:00Z",
        RETENTION,
    )
    .unwrap();
    let summary = backup::inspect(&report.path, keystore.dek()).unwrap();
    assert_eq!(summary.head.as_ref(), Some(&live), "two copies must agree on the head");
    assert_eq!(summary.events, 1);
    assert_eq!(summary.schema_version, database.schema_version().unwrap());
}

#[test]
fn a_missing_backup_is_reported() {
    let (dir, _database, keystore) = fixture();
    let error = backup::verify(dir.path().join("nothing.db"), keystore.dek()).unwrap_err();
    assert!(matches!(error, store::StoreError::Io(_)), "{error}");
}
