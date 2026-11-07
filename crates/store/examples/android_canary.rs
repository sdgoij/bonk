//! S1 spike: prove SQLCipher works in a cross-built Android binary, and that
//! an archive written on one platform opens on another.
//!
//! ```text
//! android_canary create <archive-directory>
//! android_canary open   <archive-directory>
//! ```
//!
//! `create` writes a canary as a seller name, scans the raw database and
//! sidecars for it, then reopens the archive. `open` only verifies an existing
//! archive, which is how the host and the device are checked against each
//! other.
//!
//! The canary is a fixed string, never user data. Finding it in the raw bytes
//! means SQLCipher is not encrypting what this test thinks it is encrypting.

use std::error::Error;
use std::path::{Path, PathBuf};

use crux::{Address, CountryCode, Currency, Party, SellerId};
use store::{Database, KdfParams, KeyStore, SellerRecord};

const PASSPHRASE: &str = "s1-spike-passphrase";
const CANARY: &str = "S1-CANARY-7f3c1a9e";

fn main() {
    if let Err(error) = run() {
        eprintln!("android canary: FAILED: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    if !store::cipher_enabled() {
        return Err("build with the cipher feature; this test is meaningless without encryption".into());
    }

    let mut arguments = std::env::args().skip(1);
    let mode = arguments
        .next()
        .ok_or("usage: android_canary <create|open> <archive-directory>")?;
    let directory = arguments
        .next()
        .ok_or("usage: android_canary <create|open> <archive-directory>")?;
    let directory = PathBuf::from(directory);

    match mode.as_str() {
        "create" => create(&directory)?,
        "open" => verify(&directory)?,
        other => return Err(format!("unknown mode: {other}").into()),
    }
    Ok(())
}

fn create(directory: &Path) -> Result<(), Box<dyn Error>> {
    store::permissions::private_directory(directory)?;

    let database_path = directory.join("bonk.db");
    let keystore_path = directory.join("keystore.bin");
    remove_previous(&database_path);
    remove_previous(&keystore_path);

    let keystore = KeyStore::create(&keystore_path, PASSPHRASE, KdfParams::DEFAULT)?;
    let database = Database::create(&database_path, &keystore)?;
    let seller = SellerRecord {
        id: SellerId::new(),
        party: party(),
        default_currency: Currency::EUR,
        default_locale: "nl-NL".to_owned(),
    };
    let now = store::now_rfc3339();
    store::insert_seller(database.connection(), &seller, &now)?;
    drop(database);
    drop(keystore);

    let checked = check(directory)?;
    println!("android canary: PASS (create)");
    print_check(&checked);
    Ok(())
}

fn verify(directory: &Path) -> Result<(), Box<dyn Error>> {
    let checked = check(directory)?;
    println!("android canary: PASS (open)");
    print_check(&checked);
    Ok(())
}

struct Checked {
    scanned: u64,
    database_bytes: u64,
    seller: String,
    currency: String,
}

fn check(directory: &Path) -> Result<Checked, Box<dyn Error>> {
    let database_path = directory.join("bonk.db");
    let keystore_path = directory.join("keystore.bin");
    let scanned = scan(&database_path)?;

    let keystore = KeyStore::open(&keystore_path, PASSPHRASE)?;
    let database = Database::open(&database_path, &keystore)?;
    database.integrity_check()?;
    let stored = store::first_seller(database.connection())?.ok_or("the seller did not survive the round trip")?;
    if stored.party.legal_name != CANARY {
        return Err("the canary did not round-trip".into());
    }

    Ok(Checked {
        scanned,
        database_bytes: std::fs::metadata(&database_path)?.len(),
        seller: stored.party.legal_name,
        currency: stored.default_currency.as_str().to_owned(),
    })
}

fn print_check(checked: &Checked) {
    println!("  database: {} bytes", checked.database_bytes);
    println!("  scanned: {} bytes across db/wal/shm", checked.scanned);
    println!("  encrypted: canary absent from raw bytes");
    println!("  reopened: {} / {}", checked.seller, checked.currency);
}

fn scan(database_path: &Path) -> Result<u64, Box<dyn Error>> {
    let canary = CANARY.as_bytes();
    let mut scanned = 0_u64;
    for path in [
        database_path.to_path_buf(),
        sidecar(database_path, "-wal"),
        sidecar(database_path, "-shm"),
    ] {
        if !path.exists() {
            continue;
        }
        let bytes = std::fs::read(&path)?;
        scanned += bytes.len() as u64;
        if contains(&bytes, canary) {
            return Err(format!("canary found in plaintext in {}", path.display()).into());
        }
    }
    if scanned == 0 {
        return Err("no database bytes were written".into());
    }
    Ok(scanned)
}

fn party() -> Party {
    let mut party = Party::new(CANARY, Address::new("Amsterdam", CountryCode::NL));
    party.vat_id = Some("NL123456782B01".to_owned());
    party.kvk_number = Some("12345678".to_owned());
    party
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    PathBuf::from(format!("{}{suffix}", path.display()))
}

fn remove_previous(path: &Path) {
    let _ = std::fs::remove_file(path);
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|window| window == needle)
}
