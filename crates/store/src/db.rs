use std::io;
use std::path::Path;
use std::time::Duration;

use rusqlite::Connection;

use crate::error::StoreError;
use crate::keystore::KeyStore;
use crate::migrations;

/// An opened, migrated, encrypted database.
///
/// One writer connection is the supported model. Readers, when they are added,
/// get their own connections behind the same key.
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Opens an existing database with the key from `keystore`.
    ///
    /// A missing file is an error here, not an invitation to create a fresh
    /// database: a mistyped path must never look like an empty archive.
    pub fn open(path: impl AsRef<Path>, keystore: &KeyStore) -> Result<Self, StoreError> {
        Self::open_inner(path.as_ref(), keystore.dek(), false)
    }

    /// Creates a database, failing if the file already exists.
    pub fn create(path: impl AsRef<Path>, keystore: &KeyStore) -> Result<Self, StoreError> {
        Self::open_inner(path.as_ref(), keystore.dek(), true)
    }

    /// Opens an existing database with a raw 32-byte key.
    pub fn open_with_key(path: impl AsRef<Path>, dek: &[u8; 32]) -> Result<Self, StoreError> {
        Self::open_inner(path.as_ref(), dek, false)
    }

    fn open_inner(path: &Path, dek: &[u8; 32], create: bool) -> Result<Self, StoreError> {
        if !create && !path.exists() {
            return Err(StoreError::Io(io::Error::new(
                io::ErrorKind::NotFound,
                format!("database file does not exist: {}", path.display()),
            )));
        }
        if create && path.exists() {
            return Err(StoreError::Io(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("database file already exists: {}", path.display()),
            )));
        }

        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        apply_key(&conn, dek)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA foreign_keys = ON;
             PRAGMA temp_store = MEMORY;
             PRAGMA secure_delete = ON;",
        )?;

        // Exercise the key before anything else: a wrong key fails here with
        // "file is not a database" rather than later with a confusing error.
        let _: i64 = conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))?;

        let mut database = Self { conn };
        database.migrate()?;
        Ok(database)
    }

    fn migrate(&mut self) -> Result<(), StoreError> {
        migrations::apply(&mut self.conn)
    }

    /// Runs SQLite's integrity check; `Ok(())` means the file agrees with itself.
    pub fn integrity_check(&self) -> Result<(), StoreError> {
        let result: String = self.conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        if result == "ok" {
            Ok(())
        } else {
            Err(StoreError::Integrity(result))
        }
    }

    /// Highest applied migration, or 0 on an empty schema table.
    pub fn schema_version(&self) -> Result<i64, StoreError> {
        let version: Option<i64> = self
            .conn
            .query_row("SELECT max(version) FROM schema_migration", [], |row| row.get(0))?;
        Ok(version.unwrap_or(0))
    }

    /// Number of seller records.
    pub fn seller_count(&self) -> Result<i64, StoreError> {
        Ok(self
            .conn
            .query_row("SELECT count(*) FROM seller", [], |row| row.get(0))?)
    }

    /// Number of drafts.
    pub fn draft_count(&self) -> Result<i64, StoreError> {
        Ok(self
            .conn
            .query_row("SELECT count(*) FROM invoice_draft", [], |row| row.get(0))?)
    }

    /// Creates an encrypted copy of the database at `path` using
    /// `VACUUM INTO`, which compacts the copy and inherits the connection's
    /// encryption key.
    ///
    /// The destination must not exist: overwriting a backup silently is how
    /// people lose the only good copy.
    pub fn backup_to(&self, path: impl AsRef<Path>) -> Result<(), StoreError> {
        let path = path.as_ref();
        if path.exists() {
            return Err(StoreError::Io(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("backup destination already exists: {}", path.display()),
            )));
        }
        let destination = path.to_string_lossy().into_owned();
        self.conn.execute("VACUUM INTO ?1", [destination])?;
        Ok(())
    }

    /// The underlying connection, for repositories in this crate and its
    /// integration tests. Other crates should go through application services.
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Mutable connection access, required for issuing (one transaction).
    pub fn connection_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }
}

#[cfg(feature = "sqlcipher")]
fn apply_key(conn: &Connection, dek: &[u8; 32]) -> Result<(), StoreError> {
    use std::fmt::Write as _;

    let mut hex = String::with_capacity(64);
    for byte in dek {
        // Writing to a String cannot fail.
        let _ = write!(hex, "{byte:02x}");
    }
    conn.execute_batch(&format!("PRAGMA key = \"x'{hex}'\";"))?;
    conn.execute_batch("PRAGMA cipher_memory_security = ON;")?;
    Ok(())
}

#[cfg(all(not(feature = "sqlcipher"), feature = "plain"))]
fn apply_key(_conn: &Connection, _dek: &[u8; 32]) -> Result<(), StoreError> {
    // Test-only: the keystore still works, the file is not encrypted.
    Ok(())
}

#[cfg(all(not(feature = "sqlcipher"), not(feature = "plain")))]
fn apply_key(_conn: &Connection, _dek: &[u8; 32]) -> Result<(), StoreError> {
    Err(StoreError::FeatureDisabled("sqlcipher"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keystore::KdfParams;
    use tempfile::TempDir;

    const CANARY: &str = "PLAINTEXT-CANARY-4c1f9a";

    fn temp_keystore(dir: &TempDir) -> KeyStore {
        KeyStore::create(dir.path().join("keystore.bin"), "passphrase", KdfParams::TEST).unwrap()
    }

    fn insert_canary(database: &Database) {
        database
            .connection()
            .execute(
                "INSERT INTO seller (id, legal_name, default_currency, created_at, updated_at, hlc)
                 VALUES ('s1', ?1, 'EUR', '2026-10-27T00:00:00Z', '2026-10-27T00:00:00Z', '')",
                [CANARY],
            )
            .unwrap();
    }

    #[test]
    fn creates_persists_and_reopens() {
        let dir = TempDir::new().unwrap();
        let keystore = temp_keystore(&dir);
        let path = dir.path().join("bonk.db");

        {
            let database = Database::create(&path, &keystore).unwrap();
            insert_canary(&database);
            assert_eq!(database.seller_count().unwrap(), 1);
            assert_eq!(database.schema_version().unwrap(), 2);
            database.integrity_check().unwrap();
        }

        let reopened = Database::open(&path, &keystore).unwrap();
        assert_eq!(reopened.seller_count().unwrap(), 1);
    }

    #[test]
    fn a_missing_file_is_never_created_by_open() {
        let dir = TempDir::new().unwrap();
        let keystore = temp_keystore(&dir);
        let path = dir.path().join("typo.db");
        assert!(Database::open(&path, &keystore).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn create_refuses_to_overwrite() {
        let dir = TempDir::new().unwrap();
        let keystore = temp_keystore(&dir);
        let path = dir.path().join("bonk.db");
        drop(Database::create(&path, &keystore).unwrap());
        assert!(Database::create(&path, &keystore).is_err());
    }

    #[test]
    #[cfg(feature = "sqlcipher")]
    fn a_wrong_key_cannot_open_the_file() {
        let dir = TempDir::new().unwrap();
        let keystore = temp_keystore(&dir);
        let path = dir.path().join("bonk.db");
        drop(Database::create(&path, &keystore).unwrap());

        let other_dir = TempDir::new().unwrap();
        let other = temp_keystore(&other_dir);
        assert!(Database::open(&path, &other).is_err());
    }

    #[test]
    #[cfg(feature = "sqlcipher")]
    fn the_file_does_not_contain_plaintext() {
        let dir = TempDir::new().unwrap();
        let keystore = temp_keystore(&dir);
        let path = dir.path().join("bonk.db");

        {
            let database = Database::create(&path, &keystore).unwrap();
            insert_canary(&database);
        }

        let mut scanned = Vec::new();
        for suffix in ["", "-wal", "-shm"] {
            let candidate = dir.path().join(format!("bonk.db{suffix}"));
            if candidate.exists() {
                scanned.extend(std::fs::read(candidate).unwrap());
            }
        }
        let haystack = String::from_utf8_lossy(&scanned);
        assert!(!haystack.contains(CANARY), "plaintext leaked into the database files");
    }

    #[test]
    fn opening_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let keystore = temp_keystore(&dir);
        let path = dir.path().join("bonk.db");
        drop(Database::create(&path, &keystore).unwrap());

        for _ in 0..3 {
            let database = Database::open(&path, &keystore).unwrap();
            assert_eq!(database.schema_version().unwrap(), 2);
        }
    }

    #[test]
    fn backup_inherits_the_key_and_the_data() {
        let dir = TempDir::new().unwrap();
        let keystore = temp_keystore(&dir);
        let path = dir.path().join("bonk.db");
        let backup = dir.path().join("backup.db");

        {
            let database = Database::create(&path, &keystore).unwrap();
            insert_canary(&database);
            database.backup_to(&backup).unwrap();
            // Never overwrite an existing backup.
            assert!(database.backup_to(&backup).is_err());
        }

        let restored = Database::open(&backup, &keystore).unwrap();
        assert_eq!(restored.seller_count().unwrap(), 1);
        restored.integrity_check().unwrap();

        #[cfg(feature = "sqlcipher")]
        {
            let other_dir = TempDir::new().unwrap();
            let other = temp_keystore(&other_dir);
            assert!(Database::open(&backup, &other).is_err());

            let bytes = std::fs::read(&backup).unwrap();
            let haystack = String::from_utf8_lossy(&bytes);
            assert!(!haystack.contains(CANARY), "backup contains plaintext");
        }
    }
}
