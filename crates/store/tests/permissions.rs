#![allow(clippy::unwrap_used)]
//! What mode the archive's files land with.
//!
//! The mode assertions are Unix tests: the Windows build inherits `%APPDATA%`
//! ACLs and the module is a no-op there, which the threat model says rather than
//! pretending otherwise. The two tests at the end run everywhere, so the module's
//! shape is checked on every platform.

use store::permissions;
use tempfile::TempDir;

#[test]
fn a_private_directory_is_created_once_and_only_once() {
    let root = TempDir::new().unwrap();
    let nested = root.path().join("a").join("b");

    permissions::private_directory(&nested).unwrap();
    assert!(nested.is_dir());
    permissions::private_directory(&nested).unwrap();
    assert!(nested.is_dir());
}

#[test]
fn restricting_a_missing_file_is_not_an_error() {
    let root = TempDir::new().unwrap();
    let missing = root.path().join("not-there");

    assert!(permissions::restrict(&missing).is_ok());
    permissions::restrict_quietly(&missing);
    permissions::restrict_archive(&missing);
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use store::{Database, KdfParams, KeyStore, backup, permissions};
    use tempfile::TempDir;

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn paths(root: &TempDir) -> (PathBuf, PathBuf, PathBuf) {
        let directory = root.path().join("Bonk");
        permissions::private_directory(&directory).unwrap();
        let database = directory.join("bonk.db");
        let keystore = directory.join("keystore.bin");
        (directory, database, keystore)
    }

    #[test]
    fn a_new_archive_lands_private() {
        let root = TempDir::new().unwrap();
        let (directory, database_path, keystore_path) = paths(&root);
        assert_eq!(mode(&directory), 0o700, "the directory is ours to narrow");

        let keystore = KeyStore::create(&keystore_path, "correct horse battery staple", KdfParams::TEST).unwrap();
        assert_eq!(mode(&keystore_path), 0o600);

        let database = Database::create(&database_path, &keystore).unwrap();
        assert_eq!(mode(&database_path), 0o600);

        // A write puts the archive into the write-ahead log, which must not be
        // more readable than the database itself.
        store::insert_seller(
            database.connection(),
            &store::SellerRecord {
                id: crux::SellerId::new(),
                party: crux::Party::new("Seller BV", crux::Address::new("Amsterdam", crux::CountryCode::NL)),
                default_currency: crux::Currency::EUR,
                default_locale: "nl-NL".to_owned(),
            },
            "2026-10-27T12:00:00Z",
        )
        .unwrap();
        let wal = Path::new(&format!("{}-wal", database_path.display())).to_path_buf();
        if wal.exists() {
            assert_eq!(mode(&wal), 0o600);
        }
    }

    #[test]
    fn opening_an_archive_tightens_what_was_left_open() {
        let root = TempDir::new().unwrap();
        let (_directory, database_path, keystore_path) = paths(&root);
        let passphrase = "correct horse battery staple";

        let keystore = KeyStore::create(&keystore_path, passphrase, KdfParams::TEST).unwrap();
        Database::create(&database_path, &keystore).unwrap();
        drop(keystore);

        // As if copied from a share or restored with the wrong umask.
        for path in [&keystore_path, &database_path] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert_eq!(mode(path), 0o644);
        }

        let reopened = KeyStore::open(&keystore_path, passphrase).unwrap();
        assert_eq!(mode(&keystore_path), 0o600);
        Database::open(&database_path, &reopened).unwrap();
        assert_eq!(mode(&database_path), 0o600);
    }

    #[test]
    fn a_backup_lands_private_and_its_directory_does_not_widen() {
        let root = TempDir::new().unwrap();
        let (_directory, database_path, keystore_path) = paths(&root);
        let keystore = KeyStore::create(&keystore_path, "correct horse battery staple", KdfParams::TEST).unwrap();
        let database = Database::create(&database_path, &keystore).unwrap();

        let backups = root.path().join("backups");
        let report = backup(&database, &keystore, &backups, "2026-10-27T12:00:00Z", 3).unwrap();

        assert_eq!(mode(&backups), 0o700);
        assert_eq!(mode(&report.path), 0o600);
    }
}
