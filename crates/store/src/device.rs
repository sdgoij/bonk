//! "Remember this device": the archive key kept in the operating system's
//! credential store.
//!
//! This is the deliberate convenience trade. With it on, the archive is opened by
//! the operating system's login — Windows Credential Manager, the macOS Keychain,
//! or the Secret Service — instead of by the passphrase, so anyone who is already
//! logged in as the user can read the archive. The passphrase still protects a
//! copy taken off the machine, and turning the feature off deletes the entry.
//!
//! What is kept is the data key itself, not the passphrase: the keychain then
//! gains nothing that a copied keystore together with a guessed passphrase would
//! not give, and changing the passphrase does not have to touch this entry. The
//! entry is named after a hash of the keystore's path, so the path does not appear
//! in the credential store's UI.
//!
//! The store is a port rather than a direct call so the behaviour can be tested on
//! machines with no credential store — which includes every CI runner.

use std::path::Path;

use zeroize::Zeroizing;

use crate::audit;
use crate::error::StoreError;

/// Length of the key this module keeps.
pub const KEY_LEN: usize = 32;

/// Service name the credential store lists our entries under.
const SERVICE: &str = "bonk";

/// Where a device key is kept.
pub trait DeviceStore {
    /// Stores or replaces the secret for an account.
    fn put(&self, account: &str, secret: &[u8]) -> Result<(), StoreError>;

    /// Reads it back, or `None` when the account has no entry.
    fn get(&self, account: &str) -> Result<Option<Vec<u8>>, StoreError>;

    /// Removes an entry. Returns false when there was nothing to remove.
    fn delete(&self, account: &str) -> Result<bool, StoreError>;
}

/// The operating system's credential store.
#[derive(Clone, Copy, Debug, Default)]
pub struct Keychain;

impl DeviceStore for Keychain {
    fn put(&self, account: &str, secret: &[u8]) -> Result<(), StoreError> {
        let entry = keyring::Entry::new(SERVICE, account).map_err(keychain_error)?;
        entry.set_secret(secret).map_err(keychain_error)
    }

    fn get(&self, account: &str) -> Result<Option<Vec<u8>>, StoreError> {
        let entry = keyring::Entry::new(SERVICE, account).map_err(keychain_error)?;
        match entry.get_secret() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(keychain_error(error)),
        }
    }

    fn delete(&self, account: &str) -> Result<bool, StoreError> {
        let entry = keyring::Entry::new(SERVICE, account).map_err(keychain_error)?;
        match entry.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(error) => Err(keychain_error(error)),
        }
    }
}

/// True when this platform has a credential store to use.
///
/// Linux without a Secret Service (a headless box, a container) is the common
/// case for a "no", and the caller should say so rather than fail obscurely.
pub fn available() -> bool {
    keyring::Entry::store_status().is_ok()
}

/// Keeps `dek` for the archive at `keystore`.
pub fn remember(store: &dyn DeviceStore, keystore: &Path, dek: &[u8; KEY_LEN]) -> Result<(), StoreError> {
    store.put(&account(keystore), dek)
}

/// The key kept for the archive at `keystore`, when there is one.
pub fn recall(store: &dyn DeviceStore, keystore: &Path) -> Result<Option<Zeroizing<[u8; KEY_LEN]>>, StoreError> {
    let Some(secret) = store.get(&account(keystore))? else {
        return Ok(None);
    };
    if secret.len() != KEY_LEN {
        return Err(StoreError::Data(
            "the credential store holds something that is not a 32-byte key".to_owned(),
        ));
    }
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    key.copy_from_slice(&secret);
    Ok(Some(key))
}

/// Forgets the key for the archive at `keystore`. False when there was none.
pub fn forget(store: &dyn DeviceStore, keystore: &Path) -> Result<bool, StoreError> {
    store.delete(&account(keystore))
}

/// The credential store's account name for an archive.
///
/// A hash rather than the path: the credential store's UI lists accounts, and the
/// path is the user's business. The cost is that moving the archive makes the
/// entry unreachable, which `forget` cannot then clean up — a leftover entry
/// holding a key to a database that is no longer at that path.
fn account(keystore: &Path) -> String {
    let absolute = std::path::absolute(keystore).unwrap_or_else(|_| keystore.to_path_buf());
    let digest = audit::sha256_hex(absolute.to_string_lossy().as_bytes());
    format!("archive-{}", &digest[..32])
}

fn keychain_error(error: keyring::Error) -> StoreError {
    match error {
        keyring::Error::NoDefaultStore => {
            StoreError::Rejected("this system has no credential store to keep the key in".to_owned())
        }
        other => StoreError::Keychain(other.to_string()),
    }
}

/// An in-memory store, for tests and for front ends with nothing to put a key in.
///
/// It forgets everything when the process ends, which is what makes it useless in
/// production and safe to use in a test that should not touch a real keychain.
#[doc(hidden)]
#[derive(Debug, Default)]
pub struct MemoryStore {
    entries: std::sync::Mutex<std::collections::BTreeMap<String, Vec<u8>>>,
}

impl DeviceStore for MemoryStore {
    fn put(&self, account: &str, secret: &[u8]) -> Result<(), StoreError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| StoreError::Keychain("the in-memory credential store was poisoned".to_owned()))?;
        entries.insert(account.to_owned(), secret.to_vec());
        Ok(())
    }

    fn get(&self, account: &str) -> Result<Option<Vec<u8>>, StoreError> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| StoreError::Keychain("the in-memory credential store was poisoned".to_owned()))?;
        Ok(entries.get(account).cloned())
    }

    fn delete(&self, account: &str) -> Result<bool, StoreError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| StoreError::Keychain("the in-memory credential store was poisoned".to_owned()))?;
        Ok(entries.remove(account).is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_round_trips_and_is_forgotten() {
        let store = MemoryStore::default();
        let keystore = Path::new("/tmp/bonk/keystore.bin");
        let key = [7u8; KEY_LEN];

        assert!(recall(&store, keystore).unwrap().is_none());
        remember(&store, keystore, &key).unwrap();
        assert_eq!(*recall(&store, keystore).unwrap().unwrap(), key);
        assert!(forget(&store, keystore).unwrap());
        assert!(recall(&store, keystore).unwrap().is_none());
        assert!(!forget(&store, keystore).unwrap());
    }

    #[test]
    fn another_archive_does_not_see_the_key() {
        let store = MemoryStore::default();
        remember(&store, Path::new("/tmp/a/keystore.bin"), &[1u8; KEY_LEN]).unwrap();
        assert!(recall(&store, Path::new("/tmp/b/keystore.bin")).unwrap().is_none());
    }

    #[test]
    fn a_secret_of_the_wrong_length_is_refused() {
        let store = MemoryStore::default();
        let keystore = Path::new("/tmp/bonk/keystore.bin");
        store.put(&account(keystore), b"short").unwrap();
        assert!(recall(&store, keystore).is_err());
    }

    #[test]
    fn the_account_is_stable_and_does_not_name_the_path() {
        let first = account(Path::new("/tmp/bonk/keystore.bin"));
        assert_eq!(first, account(Path::new("/tmp/bonk/keystore.bin")));
        assert_ne!(first, account(Path::new("/tmp/other/keystore.bin")));
        assert!(first.starts_with("archive-"));
        assert!(
            !first.contains("bonk"),
            "a path must not appear in the credential store's UI"
        );
        assert!(!first.contains("keystore"));
    }
}
