//! S2 spike: a minimal UniFFI surface for the mobile hello world.
//!
//! The real boundary (`Archive`, `AppError`, the application services) replaces
//! this in M11.1. For S2 the only goal is to prove that Rust, UniFFI-generated
//! Kotlin, Compose Multiplatform, JNA, and R8 work together on a device.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::sync::{Arc, Mutex};
use std::time::Instant;

use argon2::{Algorithm, Argon2, Params, Version};
use crux::{
    Address, CountryCode, Currency, DocType, DraftLine, LineId, LocalDate, Money, Party, Quantity, TaxTreatment,
    UnitCode, VatRate, calculate_totals,
};
use render::{InvoiceData, Language, LineData, PartyData, PdfRenderer, Renderer};
use sha2::{Digest, Sha256};
use syntax::{UblInvoice, UblLine, write_invoice};
use zeroize::Zeroizing;

uniffi::setup_scaffolding!();

/// Errors crossing the FFI boundary.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum AppError {
    /// The platform secret store failed.
    #[error("device secret store: {detail}")]
    Store { detail: String },
    /// The platform key was invalidated (for example, biometric enrolment changed).
    #[error("the device secret was invalidated; use the passphrase")]
    Invalidated,
    /// The user cancelled or failed device authentication.
    #[error("device authentication failed: {detail}")]
    AuthFailed { detail: String },
    /// The UBL or PDF artefact pipeline failed.
    #[error("artefact: {detail}")]
    Artefact { detail: String },
}

/// The platform credential-store port.
///
/// Android implements this over the Keystore and `BiometricPrompt`; a future
/// iOS implementation uses the Keychain. The callback is synchronous, so the
/// host may block while it shows an authentication prompt.
#[uniffi::export(callback_interface)]
pub trait DeviceSecretStore: Send + Sync {
    /// Stores or replaces a secret.
    fn put(&self, account: String, secret: Vec<u8>) -> Result<(), AppError>;
    /// Reads a secret, or `None` when the account has no entry.
    fn get(&self, account: String) -> Result<Option<Vec<u8>>, AppError>;
    /// Removes an entry; false when there was nothing to remove.
    fn delete(&self, account: String) -> Result<bool, AppError>;
}

/// The result of the S4 KDF benchmark.
#[derive(uniffi::Record)]
pub struct Argon2Report {
    /// First run, including cold page faults.
    pub first_ms: u64,
    /// Second run, with the allocator and pages warm.
    pub second_ms: u64,
    /// Peak resident set size after the runs, when `/proc` is available.
    pub max_rss_kib: u64,
}

/// Times the exact Argon2id parameters the keystore uses: 64 MiB, t=3, p=1,
/// 32-byte output, 16-byte salt, Argon2id v0x13.
#[uniffi::export]
pub fn bench_argon2() -> Result<Argon2Report, AppError> {
    const SALT: [u8; 16] = [0x42; 16];
    const PASSWORD: &[u8] = b"s4-argon2-benchmark";

    let params = Params::new(65_536, 3, 1, Some(32)).map_err(|error| AppError::Store {
        detail: error.to_string(),
    })?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0_u8; 32]);

    let started = Instant::now();
    argon2
        .hash_password_into(PASSWORD, &SALT, &mut *key)
        .map_err(|error| AppError::Store {
            detail: error.to_string(),
        })?;
    let first_ms = started.elapsed().as_millis() as u64;

    let started = Instant::now();
    argon2
        .hash_password_into(PASSWORD, &SALT, &mut *key)
        .map_err(|error| AppError::Store {
            detail: error.to_string(),
        })?;
    let second_ms = started.elapsed().as_millis() as u64;

    Ok(Argon2Report {
        first_ms,
        second_ms,
        max_rss_kib: max_rss_kib(),
    })
}

fn max_rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status.lines().find_map(|line| {
                let value = line.strip_prefix("VmHWM:")?;
                value.split_whitespace().next()?.parse().ok()
            })
        })
        .unwrap_or(0)
}

/// Writes a secret through the platform store.
#[uniffi::export]
pub fn write_device_secret(
    store: Box<dyn DeviceSecretStore>,
    account: String,
    secret: Vec<u8>,
) -> Result<(), AppError> {
    if secret.len() != 32 {
        return Err(AppError::Store {
            detail: "expected a 32-byte secret".to_owned(),
        });
    }
    store.put(account, secret)
}

/// Reads a secret through the platform store.
#[uniffi::export]
pub fn read_device_secret(store: Box<dyn DeviceSecretStore>, account: String) -> Result<Option<Vec<u8>>, AppError> {
    store.get(account)
}

/// Forgets a secret through the platform store.
#[uniffi::export]
pub fn forget_device_secret(store: Box<dyn DeviceSecretStore>, account: String) -> Result<bool, AppError> {
    store.delete(account)
}

/// Drives the callback from Rust: put, get, delete, get, and report the result.
#[uniffi::export]
pub fn probe_device_store(store: Box<dyn DeviceSecretStore>, account: String) -> Result<String, AppError> {
    let secret = vec![7_u8; 32];
    store.put(account.clone(), secret.clone())?;
    let got = store.get(account.clone())?;
    if got.as_deref() != Some(secret.as_slice()) {
        return Err(AppError::Store {
            detail: "get did not return the stored secret".to_owned(),
        });
    }
    let deleted = store.delete(account.clone())?;
    let after = store.get(account)?;
    Ok(format!(
        "put=ok get=ok delete={deleted} after={}",
        if after.is_none() { "none" } else { "present" }
    ))
}

/// SHA-256 digests and lengths of the two artefacts the S5 probe builds.
///
/// Host and device must return identical values; a difference means the UBL
/// writer or the PDF layout is not byte-deterministic across the ABIs.
#[derive(uniffi::Record)]
pub struct ArtefactHashes {
    /// Hex-encoded SHA-256 of the UBL 2.1 XML.
    pub ubl_sha256: String,
    /// Hex-encoded SHA-256 of the PDF.
    pub pdf_sha256: String,
    /// UBL byte length.
    pub ubl_len: u64,
    /// PDF byte length.
    pub pdf_len: u64,
}

/// Builds the fixed sample invoice and hashes its UBL and PDF output.
///
/// The invoice is built through the same domain and renderer paths the desktop
/// client uses, so a mismatch here is a real portability defect rather than a
/// difference in a probe-only code path.
#[uniffi::export]
pub fn artefact_hashes() -> Result<ArtefactHashes, AppError> {
    let (ubl, pdf) = sample_artefacts()?;
    Ok(ArtefactHashes {
        ubl_sha256: sha256_hex(ubl.as_bytes()),
        pdf_sha256: sha256_hex(&pdf),
        ubl_len: ubl.len() as u64,
        pdf_len: pdf.len() as u64,
    })
}

fn sample_artefacts() -> Result<(String, Vec<u8>), AppError> {
    let seller = sample_seller();
    let customer = sample_customer();
    let lines = sample_lines()?;
    let totals = calculate_totals(&lines).map_err(|error| AppError::Artefact {
        detail: error.to_string(),
    })?;
    let issue_date = LocalDate::parse_iso("2026-11-07").map_err(|error| AppError::Artefact {
        detail: error.to_string(),
    })?;
    let due_date = LocalDate::parse_iso("2026-12-07").map_err(|error| AppError::Artefact {
        detail: error.to_string(),
    })?;
    let note = "Thank you for your business.";

    let ubl_lines = lines
        .iter()
        .map(|line| {
            Ok(UblLine {
                id: line.position,
                name: &line.name,
                description: line.description.as_deref(),
                quantity: line.quantity,
                unit: line.unit,
                unit_price: line.unit_price,
                base_quantity: line.price_base_quantity,
                line_net: line.net().map_err(|error| AppError::Artefact {
                    detail: error.to_string(),
                })?,
                treatment: &line.treatment,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;

    let ubl = write_invoice(&UblInvoice {
        number: "2026-0001",
        issue_date,
        due_date: Some(due_date),
        delivery_date: None,
        doc_type: DocType::Invoice,
        currency: Currency::EUR,
        buyer_reference: Some("PO-2026-42"),
        order_reference: None,
        note: Some(note),
        preceding_invoice: None,
        seller: &seller,
        customer: &customer,
        lines: &ubl_lines,
        totals: &totals,
    })
    .map_err(|error| AppError::Artefact {
        detail: error.to_string(),
    })?;

    let data = InvoiceData {
        title: Language::English.document_title(false).to_owned(),
        labels: Language::English.labels(),
        number: "2026-0001".to_owned(),
        issue_date: issue_date.to_iso_string(),
        due_date: Some(due_date.to_iso_string()),
        currency: Currency::EUR.to_string(),
        seller: party_data(&seller),
        customer: party_data(&customer),
        lines: lines
            .iter()
            .map(|line| {
                Ok(LineData {
                    name: line.name.clone(),
                    qty: line.quantity.to_trimmed_string(),
                    price: line.unit_price.to_trimmed_string(),
                    vat: line.treatment.rate.percent_string(),
                    total: fixed(line.net().map_err(|error| AppError::Artefact {
                        detail: error.to_string(),
                    })?)?,
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?,
        net: fixed(totals.net)?,
        tax: fixed(totals.tax)?,
        gross: fixed(totals.gross)?,
        notes: Some(note.to_owned()),
    };
    let pdf = Renderer::new().render(&data).map_err(|error| AppError::Artefact {
        detail: error.to_string(),
    })?;

    Ok((ubl, pdf))
}

fn sample_seller() -> Party {
    let mut party = Party::new("Bonk Demo BV", Address::new("Amsterdam", CountryCode::NL));
    party.vat_id = Some("NL123456789B01".to_owned());
    party.kvk_number = Some("12345678".to_owned());
    party
}

fn sample_customer() -> Party {
    Party::new("Voorbeeld Klant BV", Address::new("Rotterdam", CountryCode::NL))
}

fn sample_lines() -> Result<Vec<DraftLine>, AppError> {
    let treatment = TaxTreatment::standard(VatRate::STANDARD_NL);
    let money = |units: i64| {
        Money::from_major_units(units, Currency::EUR).map_err(|error| AppError::Artefact {
            detail: error.to_string(),
        })
    };
    let quantity = |value: &str| {
        Quantity::parse(value).map_err(|error| AppError::Artefact {
            detail: error.to_string(),
        })
    };

    Ok(vec![
        DraftLine {
            id: LineId::new(),
            position: 1,
            name: "Architecture review".to_owned(),
            description: Some("Discovery workshop".to_owned()),
            quantity: quantity("3")?,
            unit: UnitCode::PIECE,
            unit_price: money(150)?,
            price_base_quantity: Quantity::ONE,
            treatment: treatment.clone(),
        },
        DraftLine {
            id: LineId::new(),
            position: 2,
            name: "Implementation".to_owned(),
            description: None,
            quantity: quantity("12.5")?,
            unit: UnitCode::HOUR,
            unit_price: money(95)?,
            price_base_quantity: Quantity::ONE,
            treatment,
        },
    ])
}

fn party_data(party: &Party) -> PartyData {
    let mut parts = Vec::new();
    if let Some(street) = &party.address.street {
        parts.push(street.clone());
    }
    parts.push(party.address.city.clone());
    parts.push(party.address.country.to_string());
    PartyData {
        name: party.legal_name.clone(),
        address: parts.join(", "),
        kvk: party.kvk_number.clone(),
        vat: party.vat_id.clone(),
    }
}

fn fixed(money: Money) -> Result<String, AppError> {
    money.to_fixed_string(2).map_err(|error| AppError::Artefact {
        detail: error.to_string(),
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Greets a caller, proving the Rust library was loaded and called.
#[uniffi::export]
pub fn hello(name: String) -> String {
    format!("Hello from Rust, {name}!")
}

/// Returns the bytes unchanged, proving byte buffers cross the boundary.
#[uniffi::export]
pub fn echo_bytes(bytes: Vec<u8>) -> Vec<u8> {
    bytes
}

/// A tiny stateful object, so the spike exercises the object lifecycle as well
/// as a plain function.
#[derive(uniffi::Object)]
pub struct Counter {
    value: Mutex<u64>,
}

#[uniffi::export]
impl Counter {
    /// Starts at zero.
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self { value: Mutex::new(0) })
    }

    /// Increments and returns the new value.
    pub fn increment(&self) -> u64 {
        let mut value = self.value.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *value += 1;
        *value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct MemoryStore {
        entries: Mutex<std::collections::HashMap<String, Vec<u8>>>,
    }

    impl DeviceSecretStore for MemoryStore {
        fn put(&self, account: String, secret: Vec<u8>) -> Result<(), AppError> {
            self.entries
                .lock()
                .expect("the test store is not poisoned")
                .insert(account, secret);
            Ok(())
        }

        fn get(&self, account: String) -> Result<Option<Vec<u8>>, AppError> {
            Ok(self
                .entries
                .lock()
                .expect("the test store is not poisoned")
                .get(&account)
                .cloned())
        }

        fn delete(&self, account: String) -> Result<bool, AppError> {
            Ok(self
                .entries
                .lock()
                .expect("the test store is not poisoned")
                .remove(&account)
                .is_some())
        }
    }

    #[test]
    fn greets_with_the_name() {
        assert_eq!(hello("Android".to_owned()), "Hello from Rust, Android!");
    }

    #[test]
    fn bytes_round_trip() {
        assert_eq!(echo_bytes(vec![1, 2, 3]), vec![1, 2, 3]);
        assert!(echo_bytes(Vec::new()).is_empty());
    }

    #[test]
    fn the_counter_counts() {
        let counter = Counter::new();
        assert_eq!(counter.increment(), 1);
        assert_eq!(counter.increment(), 2);
    }

    #[test]
    fn the_device_store_probe_round_trips() {
        let report = probe_device_store(Box::new(MemoryStore::default()), "s3-account".to_owned());
        assert_eq!(report.unwrap(), "put=ok get=ok delete=true after=none");
    }

    #[test]
    fn artefact_hashes_are_stable_across_calls() {
        let first = artefact_hashes().expect("the sample invoice builds");
        let second = artefact_hashes().expect("the sample invoice builds again");
        assert_eq!(first.ubl_sha256, second.ubl_sha256);
        assert_eq!(first.pdf_sha256, second.pdf_sha256);
        assert_eq!(first.ubl_len, second.ubl_len);
        assert_eq!(first.pdf_len, second.pdf_len);
        assert_eq!(first.ubl_sha256.len(), 64);
        assert_eq!(first.pdf_sha256.len(), 64);
        assert!(first.ubl_len > 0);
        assert!(first.pdf_len > 0);
    }

    #[test]
    fn artefact_hashes_match_the_golden_values() {
        let hashes = artefact_hashes().expect("the sample invoice builds");
        assert_eq!(
            hashes.ubl_sha256,
            "7d7f70eb279208887daae6a0099bf1c61359a709f7fc0e6fb565ea7e9c5f7020"
        );
        assert_eq!(
            hashes.pdf_sha256,
            "b81c0e35d9c11e1fa4c421fba60af47748a6bf949576dba38d2a66038d63e512"
        );
        assert_eq!(hashes.ubl_len, 4305);
        assert_eq!(hashes.pdf_len, 25846);
    }

    #[test]
    fn a_short_secret_is_refused() {
        let result = write_device_secret(Box::new(MemoryStore::default()), "s3-account".to_owned(), vec![1; 31]);
        assert!(matches!(result, Err(AppError::Store { .. })));
    }
}
