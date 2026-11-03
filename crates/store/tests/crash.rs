#![allow(clippy::unwrap_used)]
//! The M0 promise: a write that is interrupted must not damage the archive.
//!
//! `crates/store/src/bin/crash-writer.rs` keeps committing drafts until the test
//! kills it. SQLite's WAL is supposed to make an interrupted transaction look
//! like it never ran, and the audit chain is supposed to keep linking to the
//! links that did commit. Both are checked here, on the same archive, across
//! several kills.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use store::{Database, KdfParams, KeyStore, audit, integrity};
use tempfile::TempDir;

/// Shared with the child, which receives it on the command line.
const PASS: &str = "crash-torture";

/// Enough kills that some land mid-transaction; small enough to stay quick.
const ROUNDS: usize = 12;

/// Drafts the child commits before it says it is ready.
const PRE_READY: i64 = 5;

struct Archive {
    _dir: TempDir,
    database: PathBuf,
    keystore: PathBuf,
}

fn new_archive() -> Archive {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("bonk.db");
    let keystore = dir.path().join("keystore.bin");

    let key_store = KeyStore::create(&keystore, PASS, KdfParams::TEST).unwrap();
    drop(Database::create(&database, &key_store).unwrap());

    Archive {
        _dir: dir,
        database,
        keystore,
    }
}

fn spawn(archive: &Archive) -> (Child, mpsc::Receiver<String>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_crash-writer"))
        .arg("--db")
        .arg(&archive.database)
        .arg("--keystore")
        .arg(&archive.keystore)
        .arg("--passphrase")
        .arg(PASS)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("the crash writer should start");

    let stdout = child.stdout.take().expect("stdout is piped");
    let (sender, receiver) = mpsc::channel();
    // Drain for the life of the process: a full pipe would stall the writer in
    // `println!` instead of in the database, which is the point being tested.
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if sender.send(line).is_err() {
                break;
            }
        }
    });

    (child, receiver)
}

fn wait_for_ready(child: &mut Child, lines: &mpsc::Receiver<String>) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match lines.recv_timeout(left) {
            Ok(line) if line.trim() == "ready" => return,
            Ok(line) => eprintln!("crash-writer: {line}"),
            Err(error) => {
                let _ = child.kill();
                panic!("the writer never reported ready: {error}");
            }
        }
    }
}

/// xorshift, so kills land at different points without pulling in `rand`.
fn mix(mut seed: u64) -> u64 {
    seed ^= seed << 13;
    seed ^= seed >> 7;
    seed ^= seed << 17;
    seed
}

#[test]
fn a_kill_during_a_write_leaves_a_readable_archive() {
    let archive = new_archive();
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    let mut interrupted_writes = 0_usize;

    for round in 0..ROUNDS {
        let (mut child, lines) = spawn(&archive);
        wait_for_ready(&mut child, &lines);

        // The child is mid-round now. Kill it somewhere inside the next few
        // writes, then keep the pipe drained until the process is gone.
        seed = mix(seed);
        std::thread::sleep(Duration::from_micros(200 + seed % 30_000));
        child.kill().unwrap();
        let _ = child.wait().unwrap();

        // A non-empty write-ahead log at the moment of death is what makes this
        // a crash test and not a restart test: the next open has to replay or
        // discard those frames before the archive is usable again.
        if wal_len(&archive) > 0 {
            interrupted_writes += 1;
        }

        assert_archive(&archive, round);
    }

    assert!(
        interrupted_writes > 0,
        "no kill left a write-ahead log behind, so nothing was interrupted"
    );
    eprintln!("kills with a live write-ahead log: {interrupted_writes}/{ROUNDS}");
}

fn wal_len(archive: &Archive) -> u64 {
    let mut path = archive.database.clone().into_os_string();
    path.push("-wal");
    std::fs::metadata(PathBuf::from(path))
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

fn assert_archive(archive: &Archive, round: usize) {
    let keystore = KeyStore::open(&archive.keystore, PASS).unwrap();
    let database = Database::open(&archive.database, &keystore)
        .unwrap_or_else(|error| panic!("round {round}: the archive did not reopen: {error}"));

    database
        .integrity_check()
        .unwrap_or_else(|error| panic!("round {round}: integrity_check failed: {error}"));

    let head = audit::head(database.connection())
        .unwrap()
        .unwrap_or_else(|| panic!("round {round}: the audit chain lost its head"));
    let verified = audit::verify_chain(database.connection())
        .unwrap_or_else(|error| panic!("round {round}: the audit chain broke: {error}"));
    assert!(verified > 0, "round {round}: nothing was verified");
    // A rolled-back insert may consume a sequence number, but it can never leave
    // a link ahead of the rows that survived.
    assert!(
        i64::try_from(verified).unwrap() <= head.seq,
        "round {round}: {verified} links but the head claims sequence {}",
        head.seq
    );

    let report = integrity::audit(database.connection())
        .unwrap_or_else(|error| panic!("round {round}: the archive audit failed: {error}"));
    assert_eq!(report.orphaned_blobs, 0, "round {round}: a kill orphaned a blob");
    assert_eq!(
        report.artefacts, 1,
        "round {round}: the issued artefact did not survive"
    );

    let drafts = database.draft_count().unwrap();
    assert!(
        drafts >= PRE_READY,
        "round {round}: only {drafts} drafts survived, so the kill did not interrupt real work"
    );

    // A draft and its lines are written in one transaction. Whatever survives
    // must therefore have every line its own note claims.
    let mut statement = database
        .connection()
        .prepare(
            "SELECT d.notes_json, count(l.id)
             FROM invoice_draft d
             LEFT JOIN invoice_draft_line l ON l.draft_id = d.id
             GROUP BY d.id",
        )
        .unwrap();
    let mut rows = statement.query([]).unwrap();
    let mut checked = 0;
    while let Some(row) = rows.next().unwrap() {
        let notes: Option<String> = row.get(0).unwrap();
        let actual: i64 = row.get(1).unwrap();
        let Some(expected) = expected_lines(notes.as_deref()) else {
            continue;
        };
        assert_eq!(
            actual, expected,
            "round {round}: a draft kept {actual} of the {expected} lines its note records"
        );
        checked += 1;
    }
    assert!(
        checked >= PRE_READY,
        "round {round}: only {checked} drafts carried a line count to check"
    );
}

fn expected_lines(notes: Option<&str>) -> Option<i64> {
    let value: serde_json::Value = serde_json::from_str(notes?).ok()?;
    value.get("note")?.as_str()?.strip_prefix("lines=")?.parse().ok()
}

/// Proves the checks above can fail: the same open-and-verify that passes on a
/// recovered archive must reject a page header that was actually overwritten.
#[test]
fn a_damaged_page_header_is_not_a_usable_archive() {
    let archive = new_archive();
    let keystore = KeyStore::open(&archive.keystore, PASS).unwrap();
    // The control is only meaningful if the clean file passes.
    drop(Database::open(&archive.database, &keystore).unwrap());

    let mut bytes = std::fs::read(&archive.database).unwrap();
    // Offset 16 is the page-size field in a plain SQLite header, and the first
    // ciphertext byte of page 1 in SQLCipher. Neither survives the flip.
    bytes[16] ^= 0xff;
    std::fs::write(&archive.database, &bytes).unwrap();

    let detected = match Database::open(&archive.database, &keystore) {
        Err(_) => true,
        Ok(database) => database.integrity_check().is_err(),
    };
    assert!(detected, "a database with a damaged page header opened cleanly");
}
