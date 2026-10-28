//! Integration test: the binary must initialise and verify a database without
//! ever touching a GUI.

use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

fn bonk() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bonk"))
}

fn bonk_ok(args: &[&str]) -> String {
    let output = bonk().args(args).output().expect("binary runs");
    assert!(
        output.status.success(),
        "command failed: {}\nstdout: {}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn field(output: &str, prefix: &str) -> String {
    output
        .lines()
        .find_map(|line| line.strip_prefix(prefix))
        .map(str::trim)
        .unwrap_or_else(|| panic!("missing {prefix:?} in {output}"))
        .to_owned()
}

fn run_init(dir: &TempDir) -> (String, String) {
    let database = dir.path().join("bonk.db");
    let keystore = dir.path().join("keystore.bin");

    let output = bonk()
        .args([
            "init",
            "--db",
            database.to_str().expect("temp path is UTF-8"),
            "--keystore",
            keystore.to_str().expect("temp path is UTF-8"),
            "--passphrase",
            "correct horse battery staple",
        ])
        .output()
        .expect("binary runs");

    assert!(
        output.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    (
        database.to_str().expect("temp path is UTF-8").to_owned(),
        keystore.to_str().expect("temp path is UTF-8").to_owned(),
    )
}

fn typst_available() -> bool {
    let binary = std::env::var("BONK_TYPST").unwrap_or_else(|_| "typst".to_owned());
    Command::new(binary)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

#[test]
fn changing_the_passphrase_keeps_the_archive_openable() {
    let dir = TempDir::new().expect("temp dir");
    let (database, keystore) = run_init(&dir);
    let old = "correct horse battery staple";
    let new = "a much better passphrase";

    let output = bonk()
        .args([
            "passphrase-change",
            "--db",
            &database,
            "--keystore",
            &keystore,
            "--passphrase",
            old,
            "--new-passphrase",
            new,
        ])
        .output()
        .expect("binary runs");
    assert!(
        output.status.success(),
        "change failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let said = String::from_utf8_lossy(&output.stdout);
    assert!(said.contains("passphrase changed"), "{said}");

    // The archive opens with the new passphrase, and the change is on the chain.
    let output = bonk()
        .args([
            "verify",
            "--db",
            &database,
            "--keystore",
            &keystore,
            "--passphrase",
            new,
        ])
        .output()
        .expect("binary runs");
    assert!(output.status.success());
    let verified = String::from_utf8_lossy(&output.stdout);
    assert!(verified.contains("integrity check passed"), "{verified}");
    assert!(verified.contains("audit:    1 events verified"), "{verified}");

    // The old passphrase does not.
    let output = bonk()
        .args([
            "verify",
            "--db",
            &database,
            "--keystore",
            &keystore,
            "--passphrase",
            old,
        ])
        .output()
        .expect("binary runs");
    assert!(
        !output.status.success(),
        "the old passphrase must no longer open the keystore"
    );

    // A refused change leaves the keystore alone.
    let output = bonk()
        .args([
            "passphrase-change",
            "--db",
            &database,
            "--keystore",
            &keystore,
            "--passphrase",
            "not the passphrase",
            "--new-passphrase",
            "another passphrase",
        ])
        .output()
        .expect("binary runs");
    assert!(!output.status.success());
    let output = bonk()
        .args([
            "verify",
            "--db",
            &database,
            "--keystore",
            &keystore,
            "--passphrase",
            new,
        ])
        .output()
        .expect("binary runs");
    assert!(
        output.status.success(),
        "a failed change must leave the keystore usable with the current passphrase"
    );
}

#[test]
fn a_recovery_key_is_printed_once_and_the_passphrase_still_works() {
    let dir = TempDir::new().expect("temp dir");
    let (database, keystore) = run_init(&dir);
    let passphrase = "correct horse battery staple";

    let size_before = std::fs::metadata(&keystore).expect("keystore").len();
    let output = bonk()
        .args([
            "recovery-add",
            "--db",
            &database,
            "--keystore",
            &keystore,
            "--passphrase",
            passphrase,
        ])
        .output()
        .expect("binary runs");
    assert!(
        output.status.success(),
        "recovery-add failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let said = String::from_utf8_lossy(&output.stdout);
    let key = said
        .lines()
        .find_map(|line| line.strip_prefix("recovery key: "))
        .expect("the key is printed")
        .trim()
        .to_owned();
    assert_eq!(key.split('-').count(), 8, "eight groups of four: {key}");
    assert!(
        key.chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-'),
        "{key}"
    );
    assert!(said.contains("shown once"), "{said}");

    let size_after = std::fs::metadata(&keystore).expect("keystore").len();
    assert!(size_after > size_before, "the recovery wrap is appended");

    // The passphrase keeps working, and the chain records the new route.
    let output = bonk()
        .args([
            "verify",
            "--db",
            &database,
            "--keystore",
            &keystore,
            "--passphrase",
            passphrase,
        ])
        .output()
        .expect("binary runs");
    assert!(output.status.success());
    let verified = String::from_utf8_lossy(&output.stdout);
    assert!(verified.contains("integrity check passed"), "{verified}");
    assert!(verified.contains("audit:    1 events verified"), "{verified}");
}

#[test]
fn a_lost_passphrase_can_be_reset_with_the_recovery_key() {
    let dir = TempDir::new().expect("temp dir");
    let (database, keystore) = run_init(&dir);
    let old_passphrase = "correct horse battery staple";
    let new_passphrase = "a fresh passphrase";

    let output = bonk()
        .args([
            "recovery-add",
            "--db",
            &database,
            "--keystore",
            &keystore,
            "--passphrase",
            old_passphrase,
        ])
        .output()
        .expect("binary runs");
    assert!(output.status.success());
    let key = String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("recovery key: "))
        .expect("the key is printed")
        .trim()
        .to_owned();

    // Forget the passphrase and come back with the key instead.
    let output = bonk()
        .args([
            "passphrase-reset",
            "--db",
            &database,
            "--keystore",
            &keystore,
            "--recovery-key",
            &key,
            "--new-passphrase",
            new_passphrase,
        ])
        .output()
        .expect("binary runs");
    assert!(
        output.status.success(),
        "reset failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let said = String::from_utf8_lossy(&output.stdout);
    let rotated = said
        .lines()
        .find_map(|line| line.strip_prefix("new recovery key: "))
        .expect("the rotated key is printed")
        .trim()
        .to_owned();
    assert_ne!(rotated, key, "the used key is rotated away");
    assert!(said.contains("no longer works"), "{said}");

    // The new passphrase opens the archive; the old one does not.
    let output = bonk()
        .args([
            "verify",
            "--db",
            &database,
            "--keystore",
            &keystore,
            "--passphrase",
            new_passphrase,
        ])
        .output()
        .expect("binary runs");
    assert!(output.status.success());
    let verified = String::from_utf8_lossy(&output.stdout);
    assert!(verified.contains("audit:    2 events verified"), "{verified}");

    let output = bonk()
        .args([
            "verify",
            "--db",
            &database,
            "--keystore",
            &keystore,
            "--passphrase",
            old_passphrase,
        ])
        .output()
        .expect("binary runs");
    assert!(!output.status.success(), "the forgotten passphrase stays forgotten");

    // The key that was used for the reset no longer opens the archive...
    let output = bonk()
        .args([
            "passphrase-reset",
            "--db",
            &database,
            "--keystore",
            &keystore,
            "--recovery-key",
            &key,
            "--new-passphrase",
            "third passphrase",
        ])
        .output()
        .expect("binary runs");
    assert!(!output.status.success(), "a used recovery key is rotated away");

    // ...and the rotated one does.
    let output = bonk()
        .args([
            "passphrase-reset",
            "--db",
            &database,
            "--keystore",
            &keystore,
            "--recovery-key",
            &rotated,
            "--new-passphrase",
            "third passphrase",
        ])
        .output()
        .expect("binary runs");
    assert!(
        output.status.success(),
        "the rotated key must open the archive: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn reset_without_a_recovery_key_says_so() {
    let dir = TempDir::new().expect("temp dir");
    let (database, keystore) = run_init(&dir);

    let output = bonk()
        .args([
            "passphrase-reset",
            "--db",
            &database,
            "--keystore",
            &keystore,
            "--recovery-key",
            &"0".repeat(32),
            "--new-passphrase",
            "a fresh passphrase",
        ])
        .output()
        .expect("binary runs");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no recovery key"), "{stderr}");
}

#[test]
fn a_support_bundle_can_be_written_and_previewed() {
    let dir = TempDir::new().expect("temp dir");
    let (database, keystore) = run_init(&dir);
    let bundle = dir.path().join("support");

    let output = bonk()
        .args([
            "support-bundle",
            "--db",
            &database,
            "--keystore",
            &keystore,
            "--passphrase",
            "correct horse battery staple",
            "--out",
            bundle.to_str().expect("utf-8"),
        ])
        .output()
        .expect("binary runs");
    assert!(
        output.status.success(),
        "support-bundle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let said = String::from_utf8_lossy(&output.stdout);
    assert!(said.contains("bonk support bundle"), "{said}");
    assert!(
        said.contains(&format!("version:    {}", env!("CARGO_PKG_VERSION"))),
        "the bundle must name the version it came from: {said}"
    );
    assert!(said.contains("schema:     3"), "{said}");
    assert!(said.contains("No names, amounts"), "{said}");
    assert!(bundle.join("summary.txt").exists());
    assert!(bundle.join("summary.json").exists());
}

#[test]
fn init_then_verify() {
    let dir = TempDir::new().expect("temp dir");
    let (database, keystore) = run_init(&dir);

    let output = bonk()
        .args([
            "verify",
            "--db",
            &database,
            "--keystore",
            &keystore,
            "--passphrase",
            "correct horse battery staple",
        ])
        .output()
        .expect("binary runs");

    assert!(
        output.status.success(),
        "verify failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("integrity check passed"), "stdout: {stdout}");
    assert!(stdout.contains("schema:   3"), "stdout: {stdout}");
    if cfg!(feature = "cipher") {
        assert!(stdout.contains("encryption: sqlcipher"), "stdout: {stdout}");
    } else {
        assert!(stdout.contains("encryption: none (test build)"), "stdout: {stdout}");
    }
}

#[test]
fn migrate_down_is_guarded_and_respects_irreversible_migrations() {
    let dir = TempDir::new().expect("temp dir");
    let (database, keystore) = run_init(&dir);
    let common = [
        "--db",
        database.as_str(),
        "--keystore",
        keystore.as_str(),
        "--passphrase",
        "correct horse battery staple",
    ];

    let mut args = vec!["migrate-down", "--to", "2"];
    args.extend(common);
    let output = bonk().args(&args).output().expect("binary runs");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("without --yes"));

    let mut args = vec!["migrate-down", "--to", "2", "--yes"];
    args.extend(common);
    let output = bonk_ok(&args);
    assert!(output.contains("rolled back: 1 migration(s)"), "{output}");
    assert!(output.contains("schema:   2"), "{output}");

    // 0002 creates the document tables; it has no safe inverse.
    let mut args = vec!["migrate-down", "--to", "1", "--yes"];
    args.extend(common);
    let output = bonk().args(&args).output().expect("binary runs");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no down migration"));

    let mut args = vec!["verify"];
    args.extend(common);
    assert!(bonk_ok(&args).contains("schema:   3"), "opening migrates forward again");
}

#[test]
fn verify_rejects_a_wrong_passphrase() {
    let dir = TempDir::new().expect("temp dir");
    let (database, keystore) = run_init(&dir);

    let output = bonk()
        .args([
            "verify",
            "--db",
            &database,
            "--keystore",
            &keystore,
            "--passphrase",
            "wrong",
        ])
        .output()
        .expect("binary runs");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("wrong passphrase"));
}

#[test]
fn verify_never_creates_a_missing_database() {
    let dir = TempDir::new().expect("temp dir");
    let (_, keystore) = run_init(&dir);
    let missing = dir.path().join("nope.db");

    let output = bonk()
        .args([
            "verify",
            "--db",
            missing.to_str().expect("temp path is UTF-8"),
            "--keystore",
            &keystore,
            "--passphrase",
            "correct horse battery staple",
        ])
        .output()
        .expect("binary runs");

    assert!(!output.status.success());
    assert!(!Path::new(&missing).exists());
}

#[test]
fn full_slice_through_the_cli() {
    let dir = TempDir::new().expect("temp dir");
    let database = dir.path().join("bonk.db");
    let keystore = dir.path().join("keystore.bin");
    let output_dir = dir.path().join("out");
    let db = database.to_str().expect("utf-8");
    let key = keystore.to_str().expect("utf-8");
    let out = output_dir.to_str().expect("utf-8");
    let common = ["--db", db, "--keystore", key, "--passphrase", "pass"];

    let mut args = vec!["init"];
    args.extend(common);
    bonk_ok(&args);

    let mut args = vec![
        "seller-add",
        "--name",
        "Seller BV",
        "--city",
        "Amsterdam",
        "--street",
        "Main 1",
        "--postal",
        "1000 AA",
        "--peppol",
        "0106:12345678",
        "--vat",
        "NL123456782B01",
        "--kvk",
        "12345678",
    ];
    args.extend(common);
    let seller = field(&bonk_ok(&args), "seller:");

    let mut args = vec![
        "customer-add",
        "--name",
        "Customer BV",
        "--city",
        "Rotterdam",
        "--street",
        "Buyer 1",
        "--postal",
        "3000 AA",
        "--peppol",
        "0106:87654321",
    ];
    args.extend(common);
    let customer = field(&bonk_ok(&args), "customer:");

    let mut args = vec![
        "draft-new",
        "--seller",
        &seller,
        "--customer",
        &customer,
        "--date",
        "2026-10-27",
        "--due",
        "2026-11-26",
        "--buyer-ref",
        "PO-1",
        "--line",
        "Consulting|2|50.00|21",
    ];
    args.extend(common);
    let draft_output = bonk_ok(&args);
    let draft = field(&draft_output, "draft:");
    assert!(draft_output.contains("gross: 121.00"), "{draft_output}");

    let mut args = vec!["issue", &draft, "--out", out];
    args.extend(common);
    let issued_output = bonk_ok(&args);
    assert!(issued_output.contains("issued: 2026-0001"), "{issued_output}");

    let xml = std::fs::read_to_string(output_dir.join("2026-0001.xml")).expect("ubl file");
    assert!(xml.contains("<cbc:ID>2026-0001</cbc:ID>"), "{xml}");
    assert!(
        xml.contains("<cbc:PayableAmount currencyID=\"EUR\">121.00</cbc:PayableAmount>"),
        "{xml}"
    );

    let pdf_path = output_dir.join("2026-0001.pdf");
    let pdf_expected = typst_available();
    if pdf_expected {
        let pdf = std::fs::read(&pdf_path).expect("pdf file");
        assert!(pdf.starts_with(b"%PDF-"), "not a PDF");
        assert!(issued_output.contains("pdf:"), "{issued_output}");
    } else {
        assert!(!pdf_path.exists(), "PDF written without typst");
        assert!(issued_output.contains("warning: PDF not rendered"), "{issued_output}");
    }

    // Evidence for the invoice we just issued, taken while the number still
    // points at one document; the check needs no keystore at all.
    let evidence_dir = dir.path().join("evidence");
    let mut args = vec!["evidence", "2026-0001", "--out", evidence_dir.to_str().expect("utf-8")];
    args.extend(common);
    let said = bonk_ok(&args);
    assert!(said.contains("verified:  ok"), "{said}");
    assert!(said.contains("document:  2026-0001 (outbound, issued)"), "{said}");

    let args = vec!["evidence-verify", evidence_dir.to_str().expect("utf-8")];
    let said = bonk_ok(&args);
    assert!(said.contains("bundle verified"), "{said}");
    assert!(said.contains("files:"), "{said}");

    let mut args = vec!["credit-note", "2026-0001", "--date", "2026-12-15"];
    args.extend(common);
    let credit_output = bonk_ok(&args);
    assert!(credit_output.contains("corrects:    2026-0001"), "{credit_output}");
    assert!(credit_output.contains("gross:       121.00"), "{credit_output}");
    let credit = field(&credit_output, "credit note:");

    let mut args = vec!["issue", &credit, "--out", out];
    args.extend(common);
    let credit_issued = bonk_ok(&args);
    assert!(credit_issued.contains("issued: 2026-C0001"), "{credit_issued}");

    let credit_xml = std::fs::read_to_string(output_dir.join("2026-C0001.xml")).expect("credit note UBL file");
    assert!(credit_xml.contains("<CreditNote xmlns="), "{credit_xml}");
    assert!(
        credit_xml.contains("<cbc:CreditNoteTypeCode>381</cbc:CreditNoteTypeCode>"),
        "{credit_xml}"
    );
    assert!(
        credit_xml.contains("<cbc:ID>2026-0001</cbc:ID>"),
        "BG-3 names the corrected invoice: {credit_xml}"
    );
    assert!(credit_xml.contains("<cac:BillingReference>"), "{credit_xml}");
    assert!(credit_xml.contains("<cac:CreditNoteLine>"), "{credit_xml}");
    assert!(!credit_xml.contains("<cbc:DueDate>"), "{credit_xml}");

    let mut args = vec!["verify"];
    args.extend(common);
    let verified = bonk_ok(&args);
    assert!(verified.contains("audit:    4 events verified"), "{verified}");
    assert!(verified.contains("hashes verified"), "{verified}");
    if pdf_expected {
        assert!(verified.contains("artefacts: 4 hashes verified"), "{verified}");
    }

    // Importing our own file back proves the inbound path end to end: it lands in
    // the same table with the same number and does not collide with the outbound
    // document.
    let invoice_file = output_dir.join("2026-0001.xml");
    let invoice_path = invoice_file.to_str().expect("utf-8");
    let mut args = vec!["import", invoice_path];
    args.extend(common);
    let imported = bonk_ok(&args);
    assert!(imported.contains("imported: 2026-0001"), "{imported}");
    assert!(imported.contains("supplier: unmatched"), "{imported}");

    let mut args = vec!["import", invoice_path];
    args.extend(common);
    let again = bonk_ok(&args);
    assert!(again.contains("duplicate: 2026-0001"), "{again}");

    // The same number now names two documents, so evidence export refuses to
    // pick one for us.
    let ambiguous_dir = dir.path().join("evidence-ambiguous");
    let mut args = vec!["evidence", "2026-0001", "--out", ambiguous_dir.to_str().expect("utf-8")];
    args.extend(common);
    let output = bonk().args(&args).output().expect("binary runs");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("matches 2 documents"), "{stderr}");

    let mut args = vec!["received"];
    args.extend(common);
    let listed = bonk_ok(&args);
    assert!(listed.contains("2026-0001"), "{listed}");
    assert!(listed.contains("received"), "{listed}");
    assert!(listed.contains("supplier not matched"), "{listed}");

    let mut args = vec!["match", "2026-0001", "Customer BV"];
    args.extend(common);
    let matched = bonk_ok(&args);
    assert!(matched.contains("matched: 2026-0001 -> Customer BV"), "{matched}");

    let mut args = vec!["received"];
    args.extend(common);
    let listed = bonk_ok(&args);
    assert!(listed.contains("Customer BV"), "{listed}");
    assert!(!listed.contains("supplier not matched"), "{listed}");

    // Search reaches both directions and follows the supplier decision.
    let mut args = vec!["search", "customer"];
    args.extend(common);
    let found = bonk_ok(&args);
    assert!(found.contains("2026-0001"), "{found}");
    assert!(found.contains("inbound"), "{found}");
    assert!(found.contains("outbound"), "{found}");

    let mut args = vec!["search", "customer", "--direction", "inbound", "--status", "received"];
    args.extend(common);
    let found = bonk_ok(&args);
    assert!(found.contains("inbound"), "{found}");
    assert!(!found.contains("outbound"), "{found}");

    let mut args = vec!["search", "nothing-like-this"];
    args.extend(common);
    assert!(bonk_ok(&args).contains("no documents match"));

    // Export is the same rows in a shape someone else can read.
    let mut args = vec!["export"];
    args.extend(common);
    let csv = bonk_ok(&args);
    assert!(
        csv.starts_with("number,direction,party,issue_date,currency,gross,status"),
        "{csv}"
    );
    assert!(
        csv.contains("2026-0001,outbound,Customer BV,2026-10-27,EUR,121.00,issued"),
        "{csv}"
    );
    assert!(
        csv.contains("2026-0001,inbound,Customer BV,2026-10-27,EUR,121.00,received"),
        "{csv}"
    );

    let mut args = vec!["export", "--format", "json", "--direction", "inbound"];
    args.extend(common);
    let json = bonk_ok(&args);
    assert!(json.contains("\"direction\": \"inbound\""), "{json}");
    assert!(!json.contains("outbound"), "{json}");
    assert!(json.contains("\"gross\": \"121.00\""), "amounts are strings: {json}");

    let exported = dir.path().join("export.csv");
    let mut args = vec!["export", "--out", exported.to_str().expect("utf-8")];
    args.extend(common);
    let said = bonk_ok(&args);
    assert!(said.contains("exported: 3 documents"), "{said}");
    assert!(
        std::fs::read_to_string(&exported)
            .expect("export")
            .contains("2026-0001")
    );

    let mut args = vec!["approve", "2026-0001"];
    args.extend(common);
    let approved = bonk_ok(&args);
    assert!(approved.contains("approved: 2026-0001"), "{approved}");

    let mut args = vec!["received", "--status", "paid"];
    args.extend(common);
    assert!(bonk_ok(&args).contains("no received documents"));

    let mut args = vec!["mark-paid", "2026-0001"];
    args.extend(common);
    let paid = bonk_ok(&args);
    assert!(paid.contains("paid: 2026-0001"), "{paid}");

    let mut args = vec!["received"];
    args.extend(common);
    assert!(bonk_ok(&args).contains("paid"), "the register shows the new status");

    let mut args = vec!["approve", "2026-0001"];
    args.extend(common);
    let refused = bonk().args(&args).output().expect("binary runs");
    assert!(!refused.status.success(), "statuses do not move backwards");
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("already paid"),
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );

    let mut args = vec!["verify"];
    args.extend(common);
    let verified = bonk_ok(&args);
    assert!(verified.contains("audit:    8 events verified"), "{verified}");
    assert!(verified.contains("head:"), "{verified}");
    assert!(verified.contains("payloads verified"), "{verified}");
    assert!(
        !verified.contains("orphans:"),
        "every stored payload is referenced: {verified}"
    );

    // A backup must verify and be restorable with the same keystore.
    let backups = dir.path().join("backups");
    let mut args = vec!["backup", "--to", backups.to_str().expect("utf-8")];
    args.extend(common);
    let said = bonk_ok(&args);
    assert!(said.contains("verified"), "{said}");

    let backup_file = std::fs::read_dir(&backups)
        .expect("backup directory")
        .next()
        .expect("one backup")
        .expect("readable entry")
        .path();
    let mut args = vec!["verify-backup", backup_file.to_str().expect("utf-8")];
    args.extend(common);
    let said = bonk_ok(&args);
    assert!(said.contains("backup verified"), "{said}");
    assert!(said.contains("events verified"), "{said}");
    assert!(said.contains("head:"), "{said}");
}

#[test]
fn conformance_findings_stop_issue_creation() {
    let dir = TempDir::new().expect("temp dir");
    let database = dir.path().join("bonk.db");
    let keystore = dir.path().join("keystore.bin");
    let output_dir = dir.path().join("out");
    let db = database.to_str().expect("utf-8");
    let key = keystore.to_str().expect("utf-8");
    let out = output_dir.to_str().expect("utf-8");
    let common = ["--db", db, "--keystore", key, "--passphrase", "pass"];

    let mut args = vec!["init"];
    args.extend(common);
    bonk_ok(&args);

    let mut args = vec![
        "seller-add",
        "--name",
        "Seller BV",
        "--city",
        "Amsterdam",
        "--street",
        "Main 1",
        "--postal",
        "1000 AA",
        "--peppol",
        "0106:12345678",
        "--vat",
        "NL123456782B01",
        "--kvk",
        "12345678",
    ];
    args.extend(common);
    let seller = field(&bonk_ok(&args), "seller:");

    let mut args = vec![
        "customer-add",
        "--name",
        "Customer BV",
        "--city",
        "Rotterdam",
        "--street",
        "Buyer 1",
        "--postal",
        "3000 AA",
        "--peppol",
        "0106:87654321",
    ];
    args.extend(common);
    let customer = field(&bonk_ok(&args), "customer:");

    let mut args = vec![
        "draft-new",
        "--seller",
        &seller,
        "--customer",
        &customer,
        "--date",
        "2026-10-27",
        "--due",
        "2026-11-26",
        "--buyer-ref",
        "PO-1",
        "--line",
        "Export consulting|1|100.00|0|G",
    ];
    args.extend(common);
    let draft = field(&bonk_ok(&args), "draft:");

    let mut args = vec!["issue", &draft, "--out", out];
    args.extend(common);
    let output = bonk().args(&args).output().expect("binary runs");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("BR-G-10"), "{stderr}");
    assert!(stderr.contains("nothing was issued"), "{stderr}");
    assert!(!output_dir.join("2026-0001.xml").exists());

    let mut args = vec![
        "draft-new",
        "--seller",
        &seller,
        "--customer",
        &customer,
        "--date",
        "2026-10-27",
        "--due",
        "2026-11-26",
        "--buyer-ref",
        "PO-2",
        "--line",
        "Consulting|2|50.00|21",
    ];
    args.extend(common);
    let valid_draft = field(&bonk_ok(&args), "draft:");

    let mut args = vec!["issue", &valid_draft, "--out", out];
    args.extend(common);
    let issued = bonk_ok(&args);
    assert!(issued.contains("issued: 2026-0001"), "{issued}");
}

#[test]
fn blocking_findings_stop_issue_creation() {
    let dir = TempDir::new().expect("temp dir");
    let database = dir.path().join("bonk.db");
    let keystore = dir.path().join("keystore.bin");
    let db = database.to_str().expect("utf-8");
    let key = keystore.to_str().expect("utf-8");
    let common = ["--db", db, "--keystore", key, "--passphrase", "pass"];

    let mut args = vec!["init"];
    args.extend(common);
    bonk_ok(&args);

    let mut args = vec!["seller-add", "--name", "Seller BV", "--city", "Amsterdam"];
    args.extend(common);
    let seller = field(&bonk_ok(&args), "seller:");

    let mut args = vec!["customer-add", "--name", "Customer BV", "--city", "Rotterdam"];
    args.extend(common);
    let customer = field(&bonk_ok(&args), "customer:");

    let mut args = vec![
        "draft-new",
        "--seller",
        &seller,
        "--customer",
        &customer,
        "--date",
        "2026-10-27",
        "--line",
        "Bad|0|10.00|21",
    ];
    args.extend(common);
    let output = bonk().args(&args).output().expect("binary runs");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("blocking"), "{stderr}");
    assert!(stderr.contains("T1-003"), "{stderr}");
}

#[test]
fn a_weak_passphrase_is_reported_but_not_refused() {
    let dir = TempDir::new().expect("temp dir");
    let database = dir.path().join("bonk.db");
    let keystore = dir.path().join("keystore.bin");

    let output = bonk()
        .args([
            "init",
            "--db",
            database.to_str().expect("utf-8"),
            "--keystore",
            keystore.to_str().expect("utf-8"),
            "--passphrase",
            "pass",
        ])
        .output()
        .expect("binary runs");

    assert!(
        output.status.success(),
        "advice must not stop the archive being created: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("this passphrase is weak"), "{stderr}");
    assert!(database.exists());
}

#[test]
fn a_strong_passphrase_gets_no_warning() {
    let dir = TempDir::new().expect("temp dir");
    let database = dir.path().join("bonk.db");
    let keystore = dir.path().join("keystore.bin");

    let output = bonk()
        .args([
            "init",
            "--db",
            database.to_str().expect("utf-8"),
            "--keystore",
            keystore.to_str().expect("utf-8"),
            "--passphrase",
            "correct horse battery staple",
        ])
        .output()
        .expect("binary runs");

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    // The `plain` build prints its own warning about encryption, so the
    // assertion is about this one message rather than any stderr at all.
    assert!(!stderr.contains("passphrase is weak"), "{stderr}");
}
