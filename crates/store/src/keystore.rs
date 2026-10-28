//! Key wrapping: Argon2id-derived KEKs encrypt a random data encryption key.
//!
//! Why wrap instead of using the passphrase directly as the SQLCipher key:
//!
//! - Changing a passphrase rewraps 32 bytes instead of re-encrypting the whole
//!   database.
//! - The KDF parameters are ours and auditable, not SQLCipher's defaults.
//! - A second wrap can open the same key by another route, which is what the
//!   recovery key is: a 160-bit secret shown once, for the day the passphrase is
//!   gone.
//!
//! File layout (little endian), 97 bytes with a passphrase wrap only:
//!
//! ```text
//! magic "BNKKEY01" (8) | version (1) | m_cost_kib (4) | t_cost (4) |
//! p_cost (4) | salt (16) | nonce (12) | ciphertext (32) | tag (16)
//! ```
//!
//! Version 2 appends one more wrap, so the file is 185 bytes:
//!
//! ```text
//! … same passphrase wrap … | m_cost_kib (4) | t_cost (4) | p_cost (4) |
//! recovery salt (16) | nonce (12) | ciphertext (32) | tag (16)
//! ```
//!
//! Each wrap authenticates its own header (magic, version, parameters, salt) as
//! additional data, so parameter or salt tampering fails decryption rather than
//! silently producing a different key. The recovery secret itself is never
//! stored: only its sealed wrap, which the passphrase can rewrite and the
//! recovery key can open.

use std::fs;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use zeroize::Zeroizing;

use crate::error::StoreError;

const MAGIC: &[u8; 8] = b"BNKKEY01";
/// A passphrase wrap on its own: keystores written before recovery keys.
const VERSION_PASSPHRASE_ONLY: u8 = 1;
/// A passphrase wrap plus a recovery wrap.
const VERSION_WITH_RECOVERY: u8 = 2;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const DEK_LEN: usize = 32;
const TAG_LEN: usize = 16;
/// Magic, version, KDF parameters, and salt.
const HEADER_LEN: usize = MAGIC.len() + 1 + 4 + 4 + 4 + SALT_LEN;
/// One sealed wrap: parameters, salt, nonce, ciphertext, tag.
const WRAP_LEN: usize = 12 + SALT_LEN + NONCE_LEN + DEK_LEN + TAG_LEN;
const FILE_LEN_PASSPHRASE_ONLY: usize = HEADER_LEN + NONCE_LEN + DEK_LEN + TAG_LEN;
const FILE_LEN_WITH_RECOVERY: usize = FILE_LEN_PASSPHRASE_ONLY + WRAP_LEN;

/// Length of a recovery key in bytes: 160 bits, or 32 base32 symbols.
pub const RECOVERY_KEY_LEN: usize = 20;

/// Crockford base32: no `I`, `L`, `O`, or `U`, so a misread symbol is a typo
/// rather than a different key.
const RECOVERY_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

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
    /// The sealed recovery wrap, kept verbatim so the passphrase can change
    /// without the recovery secret ever being held in memory.
    recovery: Option<[u8; WRAP_LEN]>,
}

impl KeyStore {
    /// Creates a new keystore, refusing to overwrite an existing file.
    ///
    /// The file starts without a recovery key; `enable_recovery` adds one.
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
            recovery: None,
        };
        let bytes = store.render(passphrase, &nonce_bytes)?;
        write_atomic(&store.path, &bytes)?;
        Ok(store)
    }

    /// Opens an existing keystore with its passphrase.
    pub fn open(path: impl AsRef<Path>, passphrase: &str) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        let bytes = fs::read(&path)?;
        Self::from_bytes(path, &bytes, passphrase)
    }

    /// Opens an existing keystore with its recovery key.
    ///
    /// The key is the text handed out once by [`Self::enable_recovery`]; it is
    /// accepted with or without the grouping dashes, in either case.
    pub fn open_with_recovery(path: impl AsRef<Path>, recovery_key: &str) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        let bytes = fs::read(&path)?;
        let head = Layout::parse(&bytes)?;
        let block = head.recovery.ok_or(StoreError::NoRecoveryKey)?;

        let secret = parse_recovery_key(recovery_key)?;
        let wrap = read_wrap(block)?;
        let aad = wrap_header(head.version, wrap.params, &wrap.salt);
        let dek = unseal(&secret, wrap.params, &wrap.salt, &wrap.nonce, wrap.sealed, &aad)?;

        Ok(Self {
            path,
            dek,
            params: head.params,
            salt: head.salt,
            recovery: Some(block.try_into().map_err(|_| StoreError::InvalidKeystore)?),
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

    /// True when a recovery key can open this archive.
    pub fn recovery_enabled(&self) -> bool {
        self.recovery.is_some()
    }

    /// Adds a recovery key, replacing any existing one, and returns it.
    ///
    /// The passphrase is required and checked against the file on disk: without
    /// that, anyone holding a copy of the keystore could add a recovery wrap of
    /// their own and open the archive later. The key is shown once and never
    /// stored in readable form — only its sealed wrap is kept.
    pub fn enable_recovery(&mut self, passphrase: &str) -> Result<String, StoreError> {
        let current = Self::from_bytes(self.path.clone(), &fs::read(&self.path)?, passphrase)?;
        if current.dek() != self.dek() {
            return Err(StoreError::InvalidKeystore);
        }

        let mut secret = [0u8; RECOVERY_KEY_LEN];
        fill_random(&mut secret);
        let block = self.seal_recovery(&secret)?;

        let mut nonce_bytes = [0u8; NONCE_LEN];
        fill_random(&mut nonce_bytes);
        let bytes = self.render_with(passphrase, &nonce_bytes, Some(&block))?;

        // Prove the file we are about to write unwraps to the same key, under
        // both routes, before it replaces the only copy of the key.
        let candidate = Self::from_bytes(self.path.clone(), &bytes, passphrase)?;
        if candidate.dek() != self.dek() {
            return Err(StoreError::Crypto(
                "the keystore with a recovery key does not hold the same data key".to_owned(),
            ));
        }
        let via_recovery = candidate
            .recovery
            .ok_or(StoreError::InvalidKeystore)
            .and_then(|block| {
                let wrap = read_wrap(&block)?;
                let aad = wrap_header(VERSION_WITH_RECOVERY, wrap.params, &wrap.salt);
                unseal(&secret, wrap.params, &wrap.salt, &wrap.nonce, wrap.sealed, &aad)
            })?;
        if *via_recovery != *self.dek() {
            return Err(StoreError::Crypto(
                "the recovery key does not unwrap the data key".to_owned(),
            ));
        }

        write_atomic(&self.path, &bytes)?;
        self.recovery = Some(block);
        Ok(format_recovery_key(&secret))
    }

    /// Changes the passphrase by rewrapping the same data key.
    ///
    /// The new file is sealed and unsealed in memory before anything is written:
    /// if those bytes did not give the same key back, the old keystore stays in
    /// place. Writing first would make a bug in the sealing indistinguishable
    /// from losing the archive. The recovery wrap, if any, is carried over
    /// untouched.
    pub fn change_passphrase(&mut self, new_passphrase: &str) -> Result<(), StoreError> {
        let mut nonce_bytes = [0u8; NONCE_LEN];
        fill_random(&mut nonce_bytes);
        let bytes = self.render(new_passphrase, &nonce_bytes)?;

        let candidate = Self::from_bytes(self.path.clone(), &bytes, new_passphrase)?;
        if candidate.dek() != self.dek() {
            return Err(StoreError::Crypto(
                "the rewrapped keystore does not hold the same data key".to_owned(),
            ));
        }

        write_atomic(&self.path, &bytes)
    }

    /// Opens keystore bytes that are already in memory.
    ///
    /// Separate from [`Self::open`] so a rewritten file can be checked before it
    /// replaces the one on disk.
    fn from_bytes(path: PathBuf, bytes: &[u8], passphrase: &str) -> Result<Self, StoreError> {
        let head = Layout::parse(bytes)?;
        let dek = unseal(
            passphrase.as_bytes(),
            head.params,
            &head.salt,
            &head.nonce,
            head.sealed,
            head.aad,
        )?;
        Ok(Self {
            path,
            dek,
            params: head.params,
            salt: head.salt,
            recovery: head
                .recovery
                .map(|block| block.try_into().map_err(|_| StoreError::InvalidKeystore))
                .transpose()?,
        })
    }

    /// Renders the whole file, keeping the current recovery wrap.
    fn render(&self, passphrase: &str, nonce_bytes: &[u8; NONCE_LEN]) -> Result<Vec<u8>, StoreError> {
        self.render_with(passphrase, nonce_bytes, self.recovery.as_ref())
    }

    fn render_with(
        &self,
        passphrase: &str,
        nonce_bytes: &[u8; NONCE_LEN],
        recovery: Option<&[u8; WRAP_LEN]>,
    ) -> Result<Vec<u8>, StoreError> {
        let version = if recovery.is_some() {
            VERSION_WITH_RECOVERY
        } else {
            VERSION_PASSPHRASE_ONLY
        };
        let aad = wrap_header(version, self.params, &self.salt);
        let sealed = seal(
            passphrase.as_bytes(),
            self.params,
            &self.salt,
            nonce_bytes,
            &self.dek,
            &aad,
        )?;

        let mut bytes = Vec::with_capacity(FILE_LEN_WITH_RECOVERY);
        bytes.extend_from_slice(&aad);
        bytes.extend_from_slice(nonce_bytes);
        bytes.extend_from_slice(&sealed);
        if let Some(block) = recovery {
            bytes.extend_from_slice(block);
        }
        Ok(bytes)
    }

    /// Seals the data key under a fresh recovery secret.
    fn seal_recovery(&self, secret: &[u8; RECOVERY_KEY_LEN]) -> Result<[u8; WRAP_LEN], StoreError> {
        let mut salt = [0u8; SALT_LEN];
        let mut nonce_bytes = [0u8; NONCE_LEN];
        fill_random(&mut salt);
        fill_random(&mut nonce_bytes);

        let aad = wrap_header(VERSION_WITH_RECOVERY, self.params, &salt);
        let sealed = seal(secret, self.params, &salt, &nonce_bytes, &self.dek, &aad)?;

        let mut block = [0u8; WRAP_LEN];
        let mut cursor = 0;
        for value in [self.params.m_cost_kib, self.params.t_cost, self.params.p_cost] {
            block[cursor..cursor + 4].copy_from_slice(&value.to_le_bytes());
            cursor += 4;
        }
        block[cursor..cursor + SALT_LEN].copy_from_slice(&salt);
        cursor += SALT_LEN;
        block[cursor..cursor + NONCE_LEN].copy_from_slice(&nonce_bytes);
        cursor += NONCE_LEN;
        block[cursor..].copy_from_slice(&sealed);
        Ok(block)
    }
}

impl std::fmt::Debug for KeyStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyStore")
            .field("path", &self.path)
            .field("params", &self.params)
            .field("recovery", &self.recovery.is_some())
            .finish_non_exhaustive()
    }
}

/// The parts of a keystore file needed to open it.
struct Layout<'a> {
    version: u8,
    params: KdfParams,
    salt: [u8; SALT_LEN],
    nonce: [u8; NONCE_LEN],
    sealed: &'a [u8],
    /// Authenticated header of the passphrase wrap.
    aad: &'a [u8],
    recovery: Option<&'a [u8]>,
}

impl<'a> Layout<'a> {
    fn parse(bytes: &'a [u8]) -> Result<Self, StoreError> {
        if &bytes[..bytes.len().min(MAGIC.len())] != MAGIC {
            return Err(StoreError::InvalidKeystore);
        }
        let has_recovery = match bytes.len() {
            FILE_LEN_PASSPHRASE_ONLY => false,
            FILE_LEN_WITH_RECOVERY => true,
            _ => return Err(StoreError::InvalidKeystore),
        };
        let version = bytes[MAGIC.len()];
        let expected = if has_recovery {
            VERSION_WITH_RECOVERY
        } else {
            VERSION_PASSPHRASE_ONLY
        };
        if version != expected {
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
        let aad = &bytes[..cursor];

        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&bytes[cursor..cursor + NONCE_LEN]);
        cursor += NONCE_LEN;
        let sealed = &bytes[cursor..cursor + DEK_LEN + TAG_LEN];
        cursor += DEK_LEN + TAG_LEN;

        let recovery = has_recovery.then(|| &bytes[cursor..cursor + WRAP_LEN]);
        Ok(Self {
            version,
            params,
            salt,
            nonce,
            sealed,
            aad,
            recovery,
        })
    }
}

/// Derives the KEK from a secret and unwraps the data key.
fn unseal(
    secret: &[u8],
    params: KdfParams,
    salt: &[u8; SALT_LEN],
    nonce_bytes: &[u8; NONCE_LEN],
    sealed: &[u8],
    aad: &[u8],
) -> Result<Zeroizing<[u8; DEK_LEN]>, StoreError> {
    let kek = derive_kek(secret, salt, params)?;
    let cipher = Aes256Gcm::new_from_slice(kek.as_ref()).map_err(|error| StoreError::Crypto(error.to_string()))?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), Payload { msg: sealed, aad })
        .map_err(|_| StoreError::WrongPassphrase)?;
    if plaintext.len() != DEK_LEN {
        return Err(StoreError::InvalidKeystore);
    }
    let mut dek = Zeroizing::new([0u8; DEK_LEN]);
    dek.copy_from_slice(&plaintext);
    Ok(dek)
}

/// Wraps the data key under `secret`, authenticating `aad`.
fn seal(
    secret: &[u8],
    params: KdfParams,
    salt: &[u8; SALT_LEN],
    nonce_bytes: &[u8; NONCE_LEN],
    dek: &[u8; DEK_LEN],
    aad: &[u8],
) -> Result<Vec<u8>, StoreError> {
    let kek = derive_kek(secret, salt, params)?;
    let cipher = Aes256Gcm::new_from_slice(kek.as_ref()).map_err(|error| StoreError::Crypto(error.to_string()))?;
    cipher
        .encrypt(Nonce::from_slice(nonce_bytes), Payload { msg: &dek[..], aad })
        .map_err(|error| StoreError::Crypto(error.to_string()))
}

/// The authenticated header of one wrap: magic, version, parameters, salt.
fn wrap_header(version: u8, params: KdfParams, salt: &[u8; SALT_LEN]) -> Vec<u8> {
    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(MAGIC);
    header.push(version);
    header.extend_from_slice(&params.m_cost_kib.to_le_bytes());
    header.extend_from_slice(&params.t_cost.to_le_bytes());
    header.extend_from_slice(&params.p_cost.to_le_bytes());
    header.extend_from_slice(salt);
    header
}

/// One sealed wrap, split into its parts.
struct Wrap<'a> {
    params: KdfParams,
    salt: [u8; SALT_LEN],
    nonce: [u8; NONCE_LEN],
    sealed: &'a [u8],
}

/// Splits a recovery wrap into its parts.
fn read_wrap(block: &[u8]) -> Result<Wrap<'_>, StoreError> {
    if block.len() != WRAP_LEN {
        return Err(StoreError::InvalidKeystore);
    }
    let mut cursor = 0;
    let params = KdfParams {
        m_cost_kib: read_u32(block, &mut cursor),
        t_cost: read_u32(block, &mut cursor),
        p_cost: read_u32(block, &mut cursor),
    };
    let mut salt = [0u8; SALT_LEN];
    salt.copy_from_slice(&block[cursor..cursor + SALT_LEN]);
    cursor += SALT_LEN;
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&block[cursor..cursor + NONCE_LEN]);
    cursor += NONCE_LEN;
    Ok(Wrap {
        params,
        salt,
        nonce,
        sealed: &block[cursor..],
    })
}

/// Renders a recovery key in the form the user writes down: Crockford base32 in
/// groups of four, 32 symbols for 160 bits.
fn format_recovery_key(secret: &[u8; RECOVERY_KEY_LEN]) -> String {
    let mut symbols = Vec::with_capacity(32);
    let mut buffer = 0_u32;
    let mut bits = 0_u32;
    for byte in secret {
        buffer = (buffer << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            symbols.push(RECOVERY_ALPHABET[((buffer >> bits) & 0x1f) as usize] as char);
        }
    }
    symbols
        .chunks(4)
        .map(|chunk| chunk.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

/// Reads a recovery key, tolerating lowercase, spaces, and missing dashes.
///
/// Crockford's substitutions are accepted because people transcribe by eye:
/// `O` reads as zero, `I` and `L` as one. `U` is excluded from the alphabet and
/// rejected rather than guessed at.
fn parse_recovery_key(text: &str) -> Result<[u8; RECOVERY_KEY_LEN], StoreError> {
    let mut symbols = Vec::with_capacity(32);
    for character in text.chars() {
        if character == '-' || character.is_whitespace() {
            continue;
        }
        let upper = character.to_ascii_uppercase();
        let symbol = match upper {
            '0' | 'O' => 0,
            '1' | 'I' | 'L' => 1,
            'U' => return Err(StoreError::InvalidRecoveryKey),
            other => RECOVERY_ALPHABET
                .iter()
                .position(|candidate| char::from(*candidate) == other)
                .ok_or(StoreError::InvalidRecoveryKey)? as u8,
        };
        symbols.push(symbol);
    }
    if symbols.len() != 32 {
        return Err(StoreError::InvalidRecoveryKey);
    }

    let mut secret = [0u8; RECOVERY_KEY_LEN];
    let mut buffer = 0_u32;
    let mut bits = 0_u32;
    let mut index = 0;
    for symbol in symbols {
        buffer = (buffer << 5) | u32::from(symbol);
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            secret[index] = ((buffer >> bits) & 0xff) as u8;
            index += 1;
        }
    }
    Ok(secret)
}

fn derive_kek(secret: &[u8], salt: &[u8; SALT_LEN], params: KdfParams) -> Result<Zeroizing<[u8; DEK_LEN]>, StoreError> {
    let params = Params::new(params.m_cost_kib, params.t_cost, params.p_cost, Some(DEK_LEN))
        .map_err(|error| StoreError::Crypto(error.to_string()))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut kek = Zeroizing::new([0u8; DEK_LEN]);
    argon2
        .hash_password_into(secret, salt, &mut *kek)
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

    // Unix renames over the destination atomically. Windows refuses, and the
    // usual remove-then-rename would leave a window in which the keystore does
    // not exist at all — and the keystore is the only copy of the wrapped key.
    // So the previous file is parked beside it and removed only once the new one
    // is in place.
    match fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(_) if path.exists() => {
            let previous = path.with_extension("old");
            fs::copy(path, &previous)?;
            fs::remove_file(path)?;
            match fs::rename(&temporary, path) {
                Ok(()) => {
                    let _ = fs::remove_file(&previous);
                    Ok(())
                }
                Err(error) => {
                    // Put the old file back rather than leave the archive keyless.
                    let _ = fs::rename(&previous, path);
                    let _ = fs::remove_file(&temporary);
                    Err(error.into())
                }
            }
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

    const PASS: &str = "correct horse battery staple";

    fn temp_store() -> (TempDir, KeyStore) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keystore.bin");
        let store = KeyStore::create(&path, PASS, KdfParams::TEST).unwrap();
        (dir, store)
    }

    #[test]
    fn round_trips_the_data_key() {
        let (dir, store) = temp_store();
        let dek = *store.dek();
        drop(store);
        let reopened = KeyStore::open(dir.path().join("keystore.bin"), PASS).unwrap();
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

        let result = KeyStore::open(&path, PASS);
        assert!(matches!(result, Err(StoreError::WrongPassphrase)));
    }

    #[test]
    fn rejects_truncated_files() {
        let (dir, store) = temp_store();
        let path = store.path().to_path_buf();
        drop(store);
        fs::write(&path, &fs::read(&path).unwrap()[..40]).unwrap();
        assert!(matches!(KeyStore::open(&path, PASS), Err(StoreError::InvalidKeystore)));
        let _ = dir;
    }

    #[test]
    fn changing_the_passphrase_keeps_the_key() {
        let (dir, mut store) = temp_store();
        let dek = *store.dek();
        store.change_passphrase("a new passphrase").unwrap();
        let path = dir.path().join("keystore.bin");
        assert!(KeyStore::open(&path, PASS).is_err());
        let reopened = KeyStore::open(&path, "a new passphrase").unwrap();
        assert_eq!(reopened.dek(), &dek);
    }

    #[test]
    fn a_rewrite_leaves_no_stray_keystores_behind() {
        let (dir, mut store) = temp_store();
        store.change_passphrase("a new passphrase").unwrap();

        // The only file that may remain is the keystore itself: a parked copy or
        // a temporary would be a second copy of the wrapped key lying around.
        let mut names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["keystore.bin".to_owned()]);
    }

    #[test]
    fn debug_never_prints_the_key() {
        let (_dir, store) = temp_store();
        let rendered = format!("{store:?}");
        assert!(!rendered.contains("dek"));
        assert!(rendered.contains("keystore.bin"));
    }

    #[test]
    fn a_new_keystore_has_no_recovery_key() {
        let (dir, store) = temp_store();
        assert!(!store.recovery_enabled());
        let bytes = fs::read(dir.path().join("keystore.bin")).unwrap();
        assert_eq!(bytes.len(), FILE_LEN_PASSPHRASE_ONLY, "version 1 on disk");

        drop(store);
        let error = KeyStore::open_with_recovery(dir.path().join("keystore.bin"), &"0".repeat(32)).unwrap_err();
        assert!(matches!(error, StoreError::NoRecoveryKey), "{error}");
    }

    #[test]
    fn a_recovery_key_opens_the_same_data_key() {
        let (dir, mut store) = temp_store();
        let dek = *store.dek();
        let key = store.enable_recovery(PASS).unwrap();
        assert!(store.recovery_enabled());
        drop(store);

        let path = dir.path().join("keystore.bin");
        assert_eq!(fs::read(&path).unwrap().len(), FILE_LEN_WITH_RECOVERY);

        let via_recovery = KeyStore::open_with_recovery(&path, &key).unwrap();
        assert_eq!(via_recovery.dek(), &dek);

        // The passphrase still works, and both routes agree afterwards.
        let via_passphrase = KeyStore::open(&path, PASS).unwrap();
        assert_eq!(via_passphrase.dek(), &dek);
    }

    #[test]
    fn a_wrong_recovery_key_is_refused() {
        let (dir, mut store) = temp_store();
        store.enable_recovery(PASS).unwrap();
        drop(store);

        let error = KeyStore::open_with_recovery(dir.path().join("keystore.bin"), &"0".repeat(32)).unwrap_err();
        assert!(matches!(error, StoreError::WrongPassphrase), "{error}");
    }

    #[test]
    fn a_recovery_key_survives_a_passphrase_change() {
        let (dir, mut store) = temp_store();
        let dek = *store.dek();
        let key = store.enable_recovery(PASS).unwrap();
        store.change_passphrase("a new passphrase").unwrap();
        drop(store);

        let path = dir.path().join("keystore.bin");
        assert!(KeyStore::open(&path, PASS).is_err());
        let reopened = KeyStore::open_with_recovery(&path, &key).unwrap();
        assert_eq!(reopened.dek(), &dek);
    }

    #[test]
    fn adding_a_recovery_key_requires_the_passphrase() {
        let (dir, mut store) = temp_store();
        let path = dir.path().join("keystore.bin");
        let before = fs::read(&path).unwrap();

        let error = store.enable_recovery("not the passphrase").unwrap_err();
        assert!(matches!(error, StoreError::WrongPassphrase), "{error}");
        assert_eq!(
            fs::read(&path).unwrap(),
            before,
            "a refused change leaves the file alone"
        );
        assert!(!store.recovery_enabled());
    }

    #[test]
    fn a_recovery_key_can_be_replaced() {
        let (dir, mut store) = temp_store();
        let first = store.enable_recovery(PASS).unwrap();
        let second = store.enable_recovery(PASS).unwrap();
        assert_ne!(first, second);
        drop(store);

        let path = dir.path().join("keystore.bin");
        assert!(KeyStore::open_with_recovery(&path, &first).is_err(), "the old key");
        assert!(KeyStore::open_with_recovery(&path, &second).is_ok());
    }

    #[test]
    fn recovery_keys_round_trip_and_tolerate_transcription() {
        let secret: [u8; RECOVERY_KEY_LEN] = std::array::from_fn(|index| (index as u8) * 7);
        let text = format_recovery_key(&secret);
        assert_eq!(text.chars().filter(|character| *character != '-').count(), 32);
        assert_eq!(text.chars().filter(|character| *character == '-').count(), 7);

        assert_eq!(parse_recovery_key(&text).unwrap(), secret);
        assert_eq!(parse_recovery_key(&text.to_lowercase()).unwrap(), secret);
        assert_eq!(parse_recovery_key(&text.replace('-', " ")).unwrap(), secret);
        assert_eq!(parse_recovery_key(&text.replace('-', "")).unwrap(), secret);

        // Crockford reads O as zero and I/L as one, because people transcribe
        // by eye; U is not in the alphabet and is refused.
        assert_eq!(parse_recovery_key(&"O".repeat(32)).unwrap(), [0u8; RECOVERY_KEY_LEN]);
        assert_eq!(parse_recovery_key(&"0".repeat(32)).unwrap(), [0u8; RECOVERY_KEY_LEN]);
        assert!(parse_recovery_key(&"U".repeat(32)).is_err());
        assert!(parse_recovery_key("too-short").is_err());
    }
}
