//! The unlocked archive: one database and the keystore that wraps its key.
//!
//! A front end supplies the paths it wants; this crate has no platform data
//! directory. The data key is kept for the life of the session so a verified
//! backup can be written, and so a worker can open a second connection without
//! asking the user for the passphrase again.

use std::path::Path;

use store::{Database, KdfParams, KeyStore, StoreError};
use zeroize::Zeroizing;

use crate::dto::Summary;

/// An unlocked application session: one database, one keystore.
pub struct Archive {
    database: Database,
    database_path: std::path::PathBuf,
    keystore_path: std::path::PathBuf,
    /// The data key, kept so the window can write a verified backup without
    /// asking for the passphrase again. It is the same key SQLCipher is using.
    dek: Zeroizing<[u8; 32]>,
}

impl Archive {
    /// Creates a new keystore and database. Refuses to overwrite either.
    pub fn create(
        database_path: impl AsRef<Path>,
        keystore_path: impl AsRef<Path>,
        passphrase: &str,
    ) -> Result<Self, StoreError> {
        Self::create_with_params(database_path, keystore_path, passphrase, KdfParams::DEFAULT)
    }

    fn create_with_params(
        database_path: impl AsRef<Path>,
        keystore_path: impl AsRef<Path>,
        passphrase: &str,
        params: KdfParams,
    ) -> Result<Self, StoreError> {
        let database_path = database_path.as_ref().to_path_buf();
        let keystore_path = keystore_path.as_ref().to_path_buf();
        if database_path.exists() || keystore_path.exists() {
            return Err(StoreError::InvalidKeystore);
        }
        if let Some(parent) = database_path.parent() {
            store::permissions::private_directory(parent)?;
        }
        if let Some(parent) = keystore_path.parent() {
            store::permissions::private_directory(parent)?;
        }

        let keystore = KeyStore::create(&keystore_path, passphrase, params)?;
        let dek = Zeroizing::new(*keystore.dek());
        let database = Database::create(&database_path, &keystore)?;
        Ok(Self {
            database,
            database_path,
            keystore_path,
            dek,
        })
    }

    /// Unlocks an existing database.
    pub fn unlock(
        database_path: impl AsRef<Path>,
        keystore_path: impl AsRef<Path>,
        passphrase: &str,
    ) -> Result<Self, StoreError> {
        let database_path = database_path.as_ref().to_path_buf();
        let keystore_path = keystore_path.as_ref().to_path_buf();
        let keystore = KeyStore::open(&keystore_path, passphrase)?;
        let dek = Zeroizing::new(*keystore.dek());
        let database = Database::open(&database_path, &keystore)?;
        Ok(Self {
            database,
            database_path,
            keystore_path,
            dek,
        })
    }

    /// Dashboard data.
    pub fn summary(&self) -> Result<Summary, StoreError> {
        Ok(Summary {
            sellers: self.database.seller_count()?,
            drafts: self.database.draft_count()?,
            schema_version: self.database.schema_version()?,
        })
    }

    /// Runs SQLite's integrity check on the open database.
    pub fn integrity_check(&self) -> Result<(), StoreError> {
        self.database.integrity_check()
    }

    /// The data key, for a second connection that must run off the UI thread.
    ///
    /// The health check re-hashes the whole archive and cannot do that on the
    /// frame; SQLite allows a reader beside the window's writer, so the worker
    /// opens its own connection behind the same key. Callers copy it into a
    /// `Zeroizing` buffer that is wiped when the worker is done.
    pub fn dek(&self) -> &[u8; 32] {
        &self.dek
    }

    /// The open database, for application-level commands.
    pub fn database(&self) -> &Database {
        &self.database
    }

    /// Mutable access, required for issuing.
    pub fn database_mut(&mut self) -> &mut Database {
        &mut self.database
    }

    /// Path of the open database.
    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    /// Path of the keystore.
    pub fn keystore_path(&self) -> &Path {
        &self.keystore_path
    }

    /// Adds or replaces the recovery key, returning the secret to show once.
    ///
    /// The passphrase is required again rather than taken from the unlock: a
    /// credential change should not ride on a window that has been left open.
    pub fn add_recovery_key(&self, passphrase: &str) -> Result<String, StoreError> {
        let mut keystore = KeyStore::open(self.keystore_path(), passphrase)?;
        let key = keystore.enable_recovery(passphrase)?;
        if let Err(error) = store::audit::record_recovery_key(
            self.database.connection(),
            &store::now_rfc3339(),
            "gui",
            &keystore.path().display().to_string(),
        ) {
            // The key is written but unrecorded. Saying so is better than
            // hiding it, and adding a key again replaces it with one that is
            // recorded.
            return Err(StoreError::Rejected(format!(
                "the recovery key was written, but recording it in the audit chain failed: {error}. \
                 Add a recovery key again to replace it."
            )));
        }
        Ok(key)
    }

    /// Rewraps the data key under a new passphrase.
    pub fn change_passphrase(&self, current: &str, new: &str) -> Result<(), StoreError> {
        let mut keystore = KeyStore::open(self.keystore_path(), current)?;
        keystore.change_passphrase(new)?;
        if let Err(error) = store::audit::record_passphrase_change(
            self.database.connection(),
            &store::now_rfc3339(),
            "gui",
            &keystore.path().display().to_string(),
        ) {
            return Err(StoreError::Rejected(format!(
                "the passphrase was changed, but recording it in the audit chain failed: {error}"
            )));
        }
        Ok(())
    }

    /// Keeps the data key in this device's credential store, so the archive can
    /// be opened without the passphrase.
    ///
    /// The passphrase is required again: trusting a machine should not be
    /// something an unattended window can do. The decision goes into the audit
    /// chain, because a second way in is evidence about the archive.
    pub fn remember_device(&self, passphrase: &str, store: &dyn store::DeviceStore) -> Result<(), StoreError> {
        let keystore = KeyStore::open(self.keystore_path(), passphrase)?;
        store::device::remember(store, self.keystore_path(), keystore.dek())?;
        if let Err(error) = store::audit::record_device_trust(
            self.database.connection(),
            &store::now_rfc3339(),
            "gui",
            &self.keystore_path().display().to_string(),
        ) {
            return Err(StoreError::Rejected(format!(
                "the key was stored for this device, but recording it in the audit chain failed: {error}"
            )));
        }
        Ok(())
    }

    /// Stops this device from keeping the key. False when it did not have one.
    pub fn forget_device(&self, store: &dyn store::DeviceStore) -> Result<bool, StoreError> {
        let forgotten = store::device::forget(store, self.keystore_path())?;
        if forgotten
            && let Err(error) = store::audit::record_device_forget(
                self.database.connection(),
                &store::now_rfc3339(),
                "gui",
                &self.keystore_path().display().to_string(),
            )
        {
            return Err(StoreError::Rejected(format!(
                "the key was forgotten by this device, but recording it in the audit chain failed: {error}"
            )));
        }
        Ok(forgotten)
    }

    /// True when this device holds a key for the archive at these paths.
    pub fn device_ready(keystore_path: impl AsRef<Path>, store: &dyn store::DeviceStore) -> bool {
        matches!(store::device::recall(store, keystore_path.as_ref()), Ok(Some(_)))
    }

    /// Opens the archive with the key this device kept.
    ///
    /// The keystore is not consulted: the point of the feature is that the
    /// operating system's login is enough.
    pub fn unlock_with_device(
        database_path: impl AsRef<Path>,
        keystore_path: impl AsRef<Path>,
        store: &dyn store::DeviceStore,
    ) -> Result<Self, StoreError> {
        let database_path = database_path.as_ref().to_path_buf();
        let keystore_path = keystore_path.as_ref().to_path_buf();
        let Some(dek) = store::device::recall(store, &keystore_path)? else {
            return Err(StoreError::Rejected(
                "this device has no key for that archive; unlock with the passphrase once and remember it".to_owned(),
            ));
        };
        let database = Database::open_with_key(&database_path, &dek)?;
        Ok(Self {
            database,
            database_path,
            keystore_path,
            dek,
        })
    }

    /// Replaces a forgotten passphrase using the recovery key.
    ///
    /// Returns the session and the replacement recovery key. The key that was
    /// used is spent: it has now been typed somewhere, and leaving it valid would
    /// keep a second route open.
    pub fn reset_passphrase(
        database_path: impl AsRef<Path>,
        keystore_path: impl AsRef<Path>,
        recovery_key: &str,
        new_passphrase: &str,
    ) -> Result<(Self, String), StoreError> {
        let database_path = database_path.as_ref().to_path_buf();
        let keystore_path = keystore_path.as_ref().to_path_buf();
        // Open the database first: a wrong key or a mismatched archive must
        // leave the keystore exactly as it was.
        let mut keystore = KeyStore::open_with_recovery(&keystore_path, recovery_key).map_err(|error| match error {
            StoreError::WrongPassphrase => {
                StoreError::Rejected("that recovery key does not open this archive".to_owned())
            }
            other => other,
        })?;
        let database = Database::open(&database_path, &keystore)?;
        let dek = Zeroizing::new(*keystore.dek());

        keystore.change_passphrase(new_passphrase)?;
        let replacement = keystore.enable_recovery(new_passphrase)?;
        if let Err(error) = store::audit::record_passphrase_reset(
            database.connection(),
            &store::now_rfc3339(),
            "gui",
            &keystore.path().display().to_string(),
            true,
        ) {
            return Err(StoreError::Rejected(format!(
                "the passphrase was reset, but recording it in the audit chain failed: {error}. \
                 The new recovery key is {replacement}; write it down now."
            )));
        }

        Ok((
            Self {
                database,
                database_path,
                keystore_path,
                dek,
            },
            replacement,
        ))
    }

    /// Writes a verified, encrypted backup using the key this session opened with.
    pub fn backup(
        &self,
        directory: impl AsRef<Path>,
        stamp: &str,
        keep: usize,
    ) -> Result<store::BackupReport, StoreError> {
        store::backup::backup_with_key(&self.database, &self.dek, directory, stamp, keep)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(directory: &tempfile::TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
        (directory.path().join("bonk.db"), directory.path().join("keystore.bin"))
    }

    fn archive() -> (tempfile::TempDir, Archive) {
        let directory = tempfile::TempDir::new().unwrap();
        let (database, keystore) = paths(&directory);
        let archive = Archive::create_with_params(&database, &keystore, "passphrase", KdfParams::TEST).unwrap();
        (directory, archive)
    }

    #[test]
    fn an_archive_round_trips_through_lock_and_unlock() {
        let directory = tempfile::TempDir::new().unwrap();
        let (database, keystore) = paths(&directory);
        drop(Archive::create_with_params(&database, &keystore, "passphrase", KdfParams::TEST).unwrap());

        assert!(Archive::unlock(&database, &keystore, "wrong").is_err());
        let archive = Archive::unlock(&database, &keystore, "passphrase").unwrap();
        assert_eq!(archive.summary().unwrap().schema_version, 17);
        assert_eq!(archive.database_path(), database);
    }

    #[test]
    fn creation_refuses_to_overwrite_an_existing_archive() {
        let directory = tempfile::TempDir::new().unwrap();
        let (database, keystore) = paths(&directory);
        let _archive = Archive::create_with_params(&database, &keystore, "passphrase", KdfParams::TEST).unwrap();
        let again = Archive::create_with_params(&database, &keystore, "passphrase", KdfParams::TEST);
        assert!(again.is_err());
    }

    #[test]
    fn an_archive_writes_a_verified_backup() {
        let (directory, archive) = archive();
        let backups = directory.path().join("backups");
        let report = archive.backup(&backups, "20261107T120000Z", 3).unwrap();
        assert!(report.path.exists());
        assert!(report.bytes > 0);
    }
}
