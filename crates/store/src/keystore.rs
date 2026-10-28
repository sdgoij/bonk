//! Key wrapping: Argon2id-derived KEK encrypts a random data encryption key.
//!
//! Why wrap instead of using the passphrase directly as the SQLCipher key:
//!
//! - Changing a passphrase rewraps 32 bytes instead of re-encrypting the whole
//!   database.
//! - The KDF parameters are ours and auditable, not SQLCipher's defaults.
//! - A future OS-keychain unlock can wrap the same DEK a second time.
//!
//! File layout (little endian), 97 bytes:
//!
//! ```text
//! magic "BNKKEY01" (8) | version (1) | m_cost_kib (4) | t_cost (4) |
//! p_cost (4) | salt (16) | nonce (12) | ciphertext (32) | tag (16)
//! ```
//!
//! The first 37 bytes (header through salt) are authenticated as additional
//! data, so parameter or salt tampering fails decryption rather than silently
//! producing a different key.

use std::fs;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use zeroize::Zeroizing;

use crate::error::StoreError;

const MAGIC: &[u8; 8] = b"BNKKEY01";
const VERSION: u8 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const DEK_LEN: usize = 32;
const TAG_LEN: usize = 16;
const HEADER_LEN: usize = MAGIC.len() + 1 + 4 + 4 + 4 + SALT_LEN;
const FILE_LEN: usize = HEADER_LEN + NONCE_LEN + DEK_LEN + TAG_LEN;

/// Argon2id cost parameters.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KdfParams {
    /// Memory cost in kibibytes.
    pub m_cost_kib: u32,
    /// Iterations.
    pub t_cost: u32,
    /// Parallelism.
    pub p_cost: u32,
}

impl KdfParams {
    /// Production defaults: 64 MiB, 3 iterations, single lane.
    pub const DEFAULT: Self = Self {
        m_cost_kib: 65_536,
        t_cost: 3,
        p_cost: 1,
    };

    /// Cheap parameters for tests only. Never used for real keystores.
    pub const TEST: Self = Self {
        m_cost_kib: 8,
        t_cost: 1,
        p_cost: 1,
    };
}

/// An unlocked keystore holding the database data key in memory.
pub struct KeyStore {
    path: PathBuf,
    dek: Zeroizing<[u8; DEK_LEN]>,
    params: KdfParams,
    salt: [u8; SALT_LEN],
}

impl KeyStore {
    /// Creates a new keystore, refusing to overwrite an existing file.
    pub fn create(path: impl AsRef<Path>, passphrase: &str, params: KdfParams) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        if path.exists() {
            return Err(StoreError::InvalidKeystore);
        }

        let mut salt = [0u8; SALT_LEN];
        let mut nonce_bytes = [0u8; NONCE_LEN];
        let mut dek = Zeroizing::new([0u8; DEK_LEN]);
        fill_random(&mut salt);
        fill_random(&mut nonce_bytes);
        fill_random(&mut *dek);

        let store = Self {
            path,
            dek,
            params,
            salt,
        };
        let bytes = store.seal(passphrase, &nonce_bytes)?;
        write_atomic(&store.path, &bytes)?;
        Ok(store)
    }

    /// Opens an existing keystore.
    pub fn open(path: impl AsRef<Path>, passphrase: &str) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        let bytes = fs::read(&path)?;
        Self::from_bytes(path, &bytes, passphrase)
    }

    /// Unseals keystore bytes that are already in memory.
    ///
    /// Separate from [`Self::open`] so a rewrapped file can be checked before it
    /// replaces the one on disk.
    fn from_bytes(path: PathBuf, bytes: &[u8], passphrase: &str) -> Result<Self, StoreError> {
        if bytes.len() != FILE_LEN || &bytes[..MAGIC.len()] != MAGIC {
            return Err(StoreError::InvalidKeystore);
        }
        if bytes[MAGIC.len()] != VERSION {
            return Err(StoreError::InvalidKeystore);
        }

        let mut cursor = MAGIC.len() + 1;
        let params = KdfParams {
            m_cost_kib: read_u32(bytes, &mut cursor),
            t_cost: read_u32(bytes, &mut cursor),
            p_cost: read_u32(bytes, &mut cursor),
        };
        let mut salt = [0u8; SALT_LEN];
        salt.copy_from_slice(&bytes[cursor..cursor + SALT_LEN]);
        cursor += SALT_LEN;
        let mut nonce_bytes = [0u8; NONCE_LEN];
        nonce_bytes.copy_from_slice(&bytes[cursor..cursor + NONCE_LEN]);
        cursor += NONCE_LEN;

        let kek = derive_kek(passphrase, &salt, params)?;
        let cipher = Aes256Gcm::new_from_slice(kek.as_ref()).map_err(|error| StoreError::Crypto(error.to_string()))?;
        let aad = &bytes[..HEADER_LEN];
        let plaintext = cipher
            .decrypt(
                Nonce::from_slice(&nonce_bytes),
                Payload {
                    msg: &bytes[cursor..],
                    aad,
                },
            )
            .map_err(|_| StoreError::WrongPassphrase)?;
        if plaintext.len() != DEK_LEN {
            return Err(StoreError::InvalidKeystore);
        }
        let mut dek = Zeroizing::new([0u8; DEK_LEN]);
        dek.copy_from_slice(&plaintext);

        Ok(Self {
            path,
            dek,
            params,
            salt,
        })
    }

    /// The 32-byte database key.
    pub fn dek(&self) -> &[u8; DEK_LEN] {
        &self.dek
    }

    /// Keystore location.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Changes the passphrase by rewrapping the same data key.
    ///
    /// The new file is sealed and unsealed in memory before anything is written:
    /// if those bytes did not give the same key back, the old keystore stays in
    /// place. Writing first would make a bug in [`Self::seal`] indistinguishable
    /// from losing the archive.
    pub fn change_passphrase(&mut self, new_passphrase: &str) -> Result<(), StoreError> {
        let mut nonce_bytes = [0u8; NONCE_LEN];
        fill_random(&mut nonce_bytes);
        let bytes = self.seal(new_passphrase, &nonce_bytes)?;

        let candidate = Self::from_bytes(self.path.clone(), &bytes, new_passphrase)?;
        if candidate.dek() != self.dek() {
            return Err(StoreError::Crypto(
                "the rewrapped keystore does not hold the same data key".to_owned(),
            ));
        }

        write_atomic(&self.path, &bytes)
    }

    fn seal(&self, passphrase: &str, nonce_bytes: &[u8; NONCE_LEN]) -> Result<Vec<u8>, StoreError> {
        let kek = derive_kek(passphrase, &self.salt, self.params)?;
        let cipher = Aes256Gcm::new_from_slice(kek.as_ref()).map_err(|error| StoreError::Crypto(error.to_string()))?;

        let mut header = Vec::with_capacity(HEADER_LEN + NONCE_LEN);
        header.extend_from_slice(MAGIC);
        header.push(VERSION);
        header.extend_from_slice(&self.params.m_cost_kib.to_le_bytes());
        header.extend_from_slice(&self.params.t_cost.to_le_bytes());
        header.extend_from_slice(&self.params.p_cost.to_le_bytes());
        header.extend_from_slice(&self.salt);
        header.extend_from_slice(nonce_bytes);

        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(nonce_bytes),
                Payload {
                    msg: &*self.dek,
                    aad: &header[..HEADER_LEN],
                },
            )
            .map_err(|error| StoreError::Crypto(error.to_string()))?;

        header.extend_from_slice(&ciphertext);
        Ok(header)
    }
}

impl std::fmt::Debug for KeyStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyStore")
            .field("path", &self.path)
            .field("params", &self.params)
            .finish_non_exhaustive()
    }
}

fn derive_kek(
    passphrase: &str,
    salt: &[u8; SALT_LEN],
    params: KdfParams,
) -> Result<Zeroizing<[u8; DEK_LEN]>, StoreError> {
    let params = Params::new(params.m_cost_kib, params.t_cost, params.p_cost, Some(DEK_LEN))
        .map_err(|error| StoreError::Crypto(error.to_string()))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut kek = Zeroizing::new([0u8; DEK_LEN]);
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, &mut *kek)
        .map_err(|error| StoreError::Crypto(error.to_string()))?;
    Ok(kek)
}

fn fill_random(target: &mut [u8]) {
    use aes_gcm::aead::rand_core::RngCore;
    let mut rng = OsRng;
    rng.fill_bytes(target);
    let _ = Aes256Gcm::generate_nonce(&mut OsRng);
}

fn read_u32(bytes: &[u8], cursor: &mut usize) -> u32 {
    let mut buf = [0u8; 4];
    buf.copy_from_slice(&bytes[*cursor..*cursor + 4]);
    *cursor += 4;
    u32::from_le_bytes(buf)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes)?;
    match fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(_) if path.exists() => {
            // Windows cannot rename over an existing file.
            fs::remove_file(path)?;
            fs::rename(&temporary, path)?;
            Ok(())
        }
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            Err(error.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_store() -> (TempDir, KeyStore) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keystore.bin");
        let store = KeyStore::create(&path, "correct horse battery staple", KdfParams::TEST).unwrap();
        (dir, store)
    }

    #[test]
    fn round_trips_the_data_key() {
        let (dir, store) = temp_store();
        let dek = *store.dek();
        drop(store);
        let reopened = KeyStore::open(dir.path().join("keystore.bin"), "correct horse battery staple").unwrap();
        assert_eq!(reopened.dek(), &dek);
    }

    #[test]
    fn rejects_a_wrong_passphrase() {
        let (dir, store) = temp_store();
        drop(store);
        let result = KeyStore::open(dir.path().join("keystore.bin"), "wrong");
        assert!(matches!(result, Err(StoreError::WrongPassphrase)));
    }

    #[test]
    fn refuses_to_overwrite() {
        let (dir, store) = temp_store();
        let path = store.path().to_path_buf();
        drop(store);
        let result = KeyStore::create(&path, "another", KdfParams::TEST);
        assert!(matches!(result, Err(StoreError::InvalidKeystore)));
        assert!(path.exists());
        let _ = dir;
    }

    #[test]
    fn detects_tampering_with_the_header() {
        let (_dir, store) = temp_store();
        let path = store.path().to_path_buf();
        drop(store);

        let mut bytes = fs::read(&path).unwrap();
        bytes[9] ^= 0xFF; // flip a KDF parameter byte
        fs::write(&path, &bytes).unwrap();

        let result = KeyStore::open(&path, "correct horse battery staple");
        assert!(matches!(result, Err(StoreError::WrongPassphrase)));
    }

    #[test]
    fn rejects_truncated_files() {
        let (dir, store) = temp_store();
        let path = store.path().to_path_buf();
        drop(store);
        fs::write(&path, &fs::read(&path).unwrap()[..40]).unwrap();
        assert!(matches!(
            KeyStore::open(&path, "correct horse battery staple"),
            Err(StoreError::InvalidKeystore)
        ));
        let _ = dir;
    }

    #[test]
    fn changing_the_passphrase_keeps_the_key() {
        let (dir, mut store) = temp_store();
        let dek = *store.dek();
        store.change_passphrase("a new passphrase").unwrap();
        let path = dir.path().join("keystore.bin");
        assert!(KeyStore::open(&path, "correct horse battery staple").is_err());
        let reopened = KeyStore::open(&path, "a new passphrase").unwrap();
        assert_eq!(reopened.dek(), &dek);
    }

    #[test]
    fn debug_never_prints_the_key() {
        let (_dir, store) = temp_store();
        let rendered = format!("{store:?}");
        assert!(!rendered.contains("dek"));
        assert!(rendered.contains("keystore.bin"));
    }
}
