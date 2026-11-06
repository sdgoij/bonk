//! The ledger through the binary: set up a chart, post an issued invoice, read
//! the reports, and correct a posting — the path a bookkeeper actually takes.

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

/// An archive with a seller, a customer, and one issued invoice.
struct Archive {
    dir: TempDir,
    database: String,
    keystore: String,
    out: String,
}

impl Archive {
    fn new() -> Self {
        let dir = TempDir::new().expect("temp dir");
        let database = dir.path().join("bonk.db").to_str().expect("utf-8").to_owned();
        let keystore = dir.path().join("keystore.bin").to_str().expect("utf-8").to_owned();
        let out = dir.path().join("out");
        std::fs::create_dir_all(&out).expect("out dir");
        let out = out.to_str().expect("utf-8").to_owned();

        bonk_ok(&[
            "init",
            "--db",
            &database,
            "--keystore",
            &keystore,
            "--passphrase",
            "pass",
        ]);

        let seller = field(
            &bonk_ok(&[
                "seller-add",
                "--name",
                "Boeken BV",
                "--city",
                "Utrecht",
                "--street",
                "Main 1",
                "--postal",
                "1000 AA",
                "--vat",
                "NL123456782B01",
                "--kvk",
                "12345678",
                "--peppol",
                "0106:12345678",
                "--db",
                &database,
                "--keystore",
                &keystore,
                "--passphrase",
                "pass",
            ]),
            "seller:",
        );
        let customer = field(
            &bonk_ok(&[
                "customer-add",
                "--name",
                "Klant BV",
                "--city",
                "Amsterdam",
                "--street",
                "Buyer 1",
                "--postal",
                "3000 AA",
                "--peppol",
                "0106:87654321",
                "--db",
                &database,
                "--keystore",
                &keystore,
                "--passphrase",
                "pass",
            ]),
            "customer:",
        );

        let draft = field(
            &bonk_ok(&[
                "draft-new",
                "--seller",
                &seller,
                "--customer",
                &customer,
                "--date",
                "2026-02-10",
                "--due",
                "2026-03-12",
                "--buyer-ref",
                "PO-1",
                "--line",
                "Consulting|1|100.00|21",
                "--db",
                &database,
                "--keystore",
                &keystore,
                "--passphrase",
                "pass",
            ]),
            "draft:",
        );
        bonk_ok(&[
            "issue",
            &draft,
            "--out",
            &out,
            "--db",
            &database,
            "--keystore",
            &keystore,
            "--passphrase",
            "pass",
        ]);

        Self {
            dir,
            database,
            keystore,
            out,
        }
    }

    fn args(&self, command: &[&str]) -> Vec<String> {
        let mut args: Vec<String> = command.iter().map(|value| (*value).to_owned()).collect();
        args.extend([
            "--db".to_owned(),
            self.database.clone(),
            "--keystore".to_owned(),
            self.keystore.clone(),
            "--passphrase".to_owned(),
            "pass".to_owned(),
        ]);
        args
    }

    fn run(&self, command: &[&str]) -> String {
        let args = self.args(command);
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        bonk_ok(&borrowed)
    }

    fn fail(&self, command: &[&str]) -> String {
        let args = self.args(command);
        let output = bonk().args(&args).output().expect("binary runs");
        assert!(
            !output.status.success(),
            "command unexpectedly succeeded: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        String::from_utf8_lossy(&output.stderr).into_owned()
    }
}

#[test]
fn the_ledger_runs_from_setup_to_a_reversal() {
    let archive = Archive::new();

    // Posting before the chart exists says what to do about it.
    let said = archive.fail(&["ledger", "post"]);
    assert!(said.contains("bonk ledger setup"), "{said}");

    let said = archive.run(&["ledger", "setup"]);
    assert!(said.contains("accounts added:"), "{said}");
    let said = archive.run(&["ledger", "setup"]);
    assert!(said.contains("already in place"), "{said}");

    let said = archive.run(&["ledger", "chart"]);
    assert!(said.contains("1300"), "{said}");
    assert!(
        said.contains("control"),
        "the debtors account is a control account: {said}"
    );

    // Posting everything, then finding nothing left to post.
    let said = archive.run(&["ledger", "post"]);
    assert!(said.contains("posted 2026-0001 VK 2026-0001"), "{said}");
    let said = archive.run(&["ledger", "post"]);
    assert!(said.contains("nothing to post"), "{said}");

    // The matrix row: 100 net at 21 % is 121 gross.
    let said = archive.run(&["ledger", "trial-balance", "--from", "2026-01-01", "--to", "2026-12-31"]);
    assert!(said.contains("proefbalans 2026-01-01 to 2026-12-31 (EUR)"), "{said}");
    assert!(said.contains("1300") && said.contains("121.00"), "{said}");
    assert!(said.contains("100.00"), "{said}");
    assert!(said.contains("21.00"), "{said}");
    assert!(said.trim_end().ends_with("balanced"), "{said}");

    let said = archive.run(&[
        "ledger",
        "profit-and-loss",
        "--from",
        "2026-01-01",
        "--to",
        "2026-12-31",
    ]);
    assert!(said.contains("resultaat"), "{said}");
    assert!(said.contains("100.00"), "{said}");

    let said = archive.run(&["ledger", "balance-sheet", "--as-of", "2026-12-31"]);
    assert!(
        said.contains("balanced: activa equals schulden plus vermogen"),
        "{said}"
    );

    // The window matters: a year before the invoice has nothing in it.
    let said = archive.run(&["ledger", "trial-balance", "--from", "2025-01-01", "--to", "2025-12-31"]);
    assert!(said.contains("total") && said.contains("0.00"), "{said}");

    // A correction is a new entry that names the old one.
    let said = archive.run(&[
        "ledger",
        "reverse",
        "VK:2026-0001",
        "--reason",
        "wrong customer",
        "--date",
        "2026-03-01",
    ]);
    assert!(said.contains("reversed VK:2026-0001 as VK 2026-0002"), "{said}");

    let said = archive.run(&["ledger", "trial-balance", "--from", "2026-01-01", "--to", "2026-12-31"]);
    assert!(said.contains("121.00"), "{said}");
    assert!(said.trim_end().ends_with("balanced"), "{said}");

    // Reversing the same entry twice is refused, with the reason.
    let said = archive.fail(&[
        "ledger",
        "reverse",
        "VK:2026-0001",
        "--reason",
        "again",
        "--date",
        "2026-03-02",
    ]);
    assert!(said.contains("already reversed"), "{said}");
}

#[test]
fn posting_names_what_it_cannot_find() {
    let archive = Archive::new();
    archive.run(&["ledger", "setup"]);

    let said = archive.fail(&["ledger", "post", "2026-9999"]);
    assert!(said.contains("no issued document"), "{said}");

    let said = archive.fail(&["ledger", "reverse", "VK:2026-9999", "--reason", "x"]);
    assert!(said.contains("no entry"), "{said}");

    let said = archive.fail(&["ledger", "reverse", "XX:2026-0001", "--reason", "x"]);
    assert!(said.contains("unknown journal"), "{said}");
    let _ = &archive.out;
    let _ = &archive.dir;
}

#[test]
fn the_vat_return_is_read_from_the_ledger_and_then_prepared() {
    let archive = Archive::new();
    archive.run(&["ledger", "setup"]);
    archive.run(&["ledger", "post"]);

    // A preview writes nothing and says what the boxes hold.
    let said = archive.run(&["ledger", "vat-return", "--year", "2026", "--month", "2"]);
    assert!(
        said.contains("btw-aangifte 2026-02 (2026-02-01 to 2026-02-28)"),
        "{said}"
    );
    assert!(said.contains("1a"), "{said}");
    assert!(said.contains("100.00"), "{said}");
    assert!(said.contains("21.00"), "{said}");
    assert!(said.contains("5a"), "{said}");
    assert!(said.contains("5b"), "{said}");
    assert!(said.contains("5c"), "{said}");
    assert!(said.contains("fileable:    yes"), "{said}");
    assert!(
        !said.contains("not checked"),
        "every check can run now that open items exist: {said}"
    );
    assert!(said.contains("mapping      nl-2026.1"), "{said}");
    assert!(
        !said.contains("prepared:"),
        "a preview must not claim to have written anything: {said}"
    );

    // Preparing stores it, and preparing again supersedes the first.
    let said = archive.run(&["ledger", "vat-return", "--year", "2026", "--month", "2", "--prepare"]);
    assert!(said.contains("prepared:"), "{said}");
    assert!(said.contains("basis:      1 entries"), "{said}");

    let said = archive.run(&["ledger", "vat-return", "--year", "2026", "--month", "2", "--prepare"]);
    assert!(said.contains("superseded:"), "{said}");

    let said = archive.fail(&["ledger", "vat-return", "--month", "13"]);
    assert!(said.contains("not 1-12"), "{said}");
}

#[test]
fn a_bank_statement_settles_an_invoice_end_to_end() {
    let archive = Archive::new();
    archive.run(&["ledger", "setup"]);
    archive.run(&["ledger", "post"]);
    archive.run(&[
        "ledger",
        "bank",
        "add",
        "--name",
        "Zakelijk",
        "--iban",
        "NL91ABNA0417164300",
    ]);

    let statement = archive.dir.path().join("feb.camt.xml");
    std::fs::write(
        &statement,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.02">
  <BkToCstmrStmt>
    <Stmt>
      <Id>FEB-2026</Id>
      <Acct><Id><IBAN>NL91ABNA0417164300</IBAN></Id><Ccy>EUR</Ccy></Acct>
      <Ntry>
        <Amt Ccy="EUR">121.00</Amt>
        <CdtDbtInd>CRDT</CdtDbtInd>
        <BookgDt><Dt>2026-02-20</Dt></BookgDt>
        <NtryDtls><TxDtls>
          <RltdPties><Dbtr><Nm>Klant BV</Nm></Dbtr></RltdPties>
          <RmtInf><Ustrd>Betaling factuur 2026-0001</Ustrd></RmtInf>
        </TxDtls></NtryDtls>
      </Ntry>
    </Stmt>
  </BkToCstmrStmt>
</Document>"#,
    )
    .expect("statement");

    let said = archive.run(&["ledger", "bank", "import", statement.to_str().expect("utf-8")]);
    assert!(said.contains("1 imported"), "{said}");
    let said = archive.run(&["ledger", "bank", "import", statement.to_str().expect("utf-8")]);
    assert!(said.contains("already imported"), "{said}");

    let said = archive.run(&["ledger", "bank", "auto-match"]);
    assert!(said.contains("matched 2026-0001"), "{said}");

    let said = archive.run(&["ledger", "open-items"]);
    assert!(said.contains("no open receivable"), "{said}");

    // The payment is in the books and the reconciliation is still happy.
    let said = archive.run(&["ledger", "trial-balance", "--from", "2026-01-01", "--to", "2026-12-31"]);
    assert!(said.contains("1100"), "the bank moved: {said}");
    assert!(said.trim_end().ends_with("balanced"), "{said}");
    let said = archive.run(&["ledger", "vat-return", "--year", "2026", "--month", "2"]);
    assert!(said.contains("fileable:    yes"), "{said}");

    // The transaction now reports as matched.
    let said = archive.run(&["ledger", "bank", "transactions", "--status", "matched"]);
    assert!(said.contains("Klant BV"), "{said}");
}

#[test]
fn a_memorial_entry_is_written_and_must_balance() {
    let archive = Archive::new();
    archive.run(&["ledger", "setup"]);

    let said = archive.run(&[
        "ledger",
        "post-manual",
        "--date",
        "2026-03-01",
        "--description",
        "Bankkosten maart",
        "--line",
        "7100|debit|12.50",
        "--line",
        "1100|credit|12.50",
    ]);
    assert!(said.contains("posted MEM 2026-0001 (2 lines)"), "{said}");

    let said = archive.run(&["ledger", "trial-balance", "--from", "2026-01-01", "--to", "2026-12-31"]);
    assert!(said.trim_end().ends_with("balanced"), "{said}");
    assert!(said.contains("7100"), "{said}");

    // An entry with one side is refused before anything is written.
    let said = archive.fail(&[
        "ledger",
        "post-manual",
        "--date",
        "2026-03-01",
        "--description",
        "Half",
        "--line",
        "7100|debit|12.50",
    ]);
    assert!(said.contains("does not balance"), "{said}");

    // So is a line that is not in the documented shape.
    let said = archive.fail(&[
        "ledger",
        "post-manual",
        "--date",
        "2026-03-01",
        "--description",
        "Typo",
        "--line",
        "7100 debit 12.50",
    ]);
    assert!(said.contains("account|side|amount"), "{said}");

    // A tagged line with a code reaches a box through the same path a purchase
    // uses; a correction of a mis-boxed purchase is exactly this.
    let said = archive.run(&[
        "ledger",
        "post-manual",
        "--date",
        "2026-03-01",
        "--description",
        "Herindeling",
        "--line",
        "9999|debit|200.00|AE|21|NL-AE-GOODS",
        "--line",
        "1500|debit|42.00|AE|21|NL-AE-GOODS",
        "--line",
        "1620|credit|42.00|AE|21|NL-AE-GOODS",
        "--line",
        "1400|credit|200.00",
    ]);
    assert!(said.contains("posted MEM 2026-0002 (4 lines)"), "{said}");

    let said = archive.run(&["ledger", "vat-return", "--year", "2026", "--month", "3"]);
    assert!(said.contains("4b"), "the correction reached its box: {said}");
    assert!(said.contains("fileable:    yes"), "{said}");
}

#[test]
fn tax_codes_are_listed_and_a_sale_has_none_to_choose() {
    let archive = Archive::new();

    let said = archive.run(&["ledger", "tax-codes"]);
    assert!(said.contains("NL-AE-SERVICES"), "{said}");
    assert!(said.contains("NL-AE-GOODS"), "{said}");
    assert!(said.contains("4b"), "the box a code resolves is shown: {said}");

    // Reading what a document carries needs nothing posted and says when none is.
    let said = archive.run(&["ledger", "tax-code", "2026-0001"]);
    assert!(said.contains("none"), "{said}");

    // Only received documents take a code: a sale is placed by its own category.
    let said = archive.fail(&["ledger", "tax-code", "2026-0001", "NL-AE-GOODS"]);
    assert!(said.contains("received documents"), "{said}");
}

#[test]
fn the_subledger_can_be_rebuilt_after_the_fact() {
    let archive = Archive::new();
    archive.run(&["ledger", "setup"]);
    archive.run(&["ledger", "post"]);

    // Posting already wrote this invoice's item, so the rebuild is a no-op that
    // says so. The command exists for archives posted before items did.
    let said = archive.run(&["ledger", "rebuild-open-items"]);
    assert!(said.contains("open items: 0 added, 1 already present"), "{said}");
    assert!(said.contains("every posted document has a subledger item"), "{said}");

    let said = archive.run(&["ledger", "open-items"]);
    assert!(said.contains("2026-0001"), "{said}");
}

#[test]
fn an_evidence_bundle_carries_the_books_the_invoice_landed_in() {
    let archive = Archive::new();
    archive.run(&["ledger", "setup"]);
    archive.run(&["ledger", "post"]);
    archive.run(&["ledger", "vat-return", "--year", "2026", "--month", "2", "--prepare"]);

    let bundle = archive.dir.path().join("evidence");
    let said = archive.run(&["evidence", "2026-0001", "--out", bundle.to_str().expect("utf-8")]);
    assert!(said.contains("ledger:    1 entries, 1 VAT return(s)"), "{said}");
    assert!(bundle.join("ledger.jsonl").exists());
    assert!(bundle.join("vat-returns").is_dir(), "the filed return travels with it");

    // The verifier needs neither the archive nor the keystore: the bundle
    // explains its own books.
    let said = bonk_ok(&["evidence-verify", bundle.to_str().expect("utf-8")]);
    assert!(said.contains("bundle verified"), "{said}");
    assert!(said.contains("1 entries, 1 VAT return(s)"), "{said}");
}
