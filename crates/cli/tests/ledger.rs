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
    seller: String,
    customer: String,
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
            seller,
            customer,
        }
    }

    /// Issues one more invoice for a single `Consulting` line, returning its number.
    fn sell(&self, net: &str) -> String {
        let draft = field(
            &self.run(&[
                "draft-new",
                "--seller",
                &self.seller,
                "--customer",
                &self.customer,
                "--date",
                "2026-02-11",
                "--due",
                "2026-03-13",
                "--buyer-ref",
                "PO-2",
                "--line",
                &format!("Consulting|1|{net}|21"),
            ]),
            "draft:",
        );
        let out = format!("{}/extra.xml", self.out);
        field(&self.run(&["issue", &draft, "--out", &out]), "issued:")
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
fn exchange_rates_are_set_listed_and_imported() {
    let archive = Archive::new();

    let said = archive.run(&[
        "ledger",
        "fx",
        "set",
        "--currency",
        "GBP",
        "--date",
        "2026-02-01",
        "--rate",
        "1.17",
    ]);
    assert!(said.contains("GBP from 2026-02-01"), "{said}");

    let said = archive.run(&["ledger", "fx", "list"]);
    assert!(said.contains("GBP"), "{said}");
    assert!(said.contains("manual"), "{said}");

    let file = archive.dir.path().join("eurofxref-hist.csv");
    std::fs::write(&file, "Date,USD\n2026-02-01,1.08\n").expect("the rate file is written");
    let said = archive.run(&["ledger", "fx", "import", file.to_str().expect("utf-8")]);
    assert!(said.contains("written:    1"), "{said}");
    assert!(said.contains("currencies: 1"), "{said}");

    let said = archive.run(&["ledger", "fx", "list", "--currency", "USD"]);
    assert!(said.contains("USD"), "{said}");
    assert!(!said.contains("GBP"), "the filter keeps the other currency out: {said}");

    // An invoice in the reporting currency needs no rate, and the command says so.
    let number = archive.sell("50.00");
    let said = archive.run(&["ledger", "fx", "show", &number]);
    assert!(said.contains("reporting"), "{said}");
    let refused = archive.fail(&["ledger", "fx", "override", &number, "--rate", "1.20"]);
    assert!(refused.contains("needs no rate"), "{refused}");
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
fn an_mt940_statement_settles_the_same_invoice_by_its_remittance_text() {
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

    // The same payment the CAMT fixture carries, as the bank's own export.
    // The counterparty is not a field MT940 has; the invoice number arrives in the
    // `:86:` text, which is where the matcher looks for it.
    let statement = archive.dir.path().join("feb.940");
    std::fs::write(
        &statement,
        ":20:BONK-2026-02
:25:NL91ABNA0417164300
:28C:1/1
:60F:C260201EUR0,00
:61:2602200220C121,00NTRFNONREF//BANK-1
:86:Betaling factuur 2026-0001
:62F:C260228EUR121,00
",
    )
    .expect("statement");

    let said = archive.run(&["ledger", "bank", "import", statement.to_str().expect("utf-8")]);
    assert!(said.contains("1 imported"), "{said}");
    let said = archive.run(&["ledger", "bank", "import", statement.to_str().expect("utf-8")]);
    assert!(said.contains("already imported"), "{said}");

    let said = archive.run(&["ledger", "bank", "transactions"]);
    assert!(
        said.contains("Betaling factuur 2026-0001"),
        "the :86: text is kept: {said}"
    );

    let said = archive.run(&["ledger", "bank", "auto-match"]);
    assert!(said.contains("matched 2026-0001"), "{said}");
    let said = archive.run(&["ledger", "open-items"]);
    assert!(said.contains("no open receivable"), "{said}");
}

#[test]
fn a_statement_that_does_not_add_up_is_refused_with_the_difference_named() {
    let archive = Archive::new();
    archive.run(&["ledger", "setup"]);
    archive.run(&[
        "ledger",
        "bank",
        "add",
        "--name",
        "Zakelijk",
        "--iban",
        "NL91ABNA0417164300",
    ]);

    let statement = archive.dir.path().join("broken.940");
    std::fs::write(
        &statement,
        ":20:BONK-2026-02
:60F:C260201EUR100,00
:61:260220C121,00NTRF
:62F:C260228EUR121,00
",
    )
    .expect("statement");

    let said = archive.fail(&["ledger", "bank", "import", statement.to_str().expect("utf-8")]);
    assert!(said.contains("does not add up"), "{said}");
    assert!(said.contains("100000000"), "the difference is named: {said}");
}

#[test]
fn one_payment_settles_two_invoices_with_one_entry() {
    let archive = Archive::new();
    archive.run(&["ledger", "setup"]);
    let second = archive.sell("50.00");
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

    // 121.00 and 60.50 arrive as one transfer that names both.
    let statement = archive.dir.path().join("two.940");
    std::fs::write(
        &statement,
        ":20:BONK-2026-02\n:60F:C260201EUR0,00\n:61:260220C181,50NTRF\n:86:Facturen 2026-0001 en 2026-0002\n:62F:C260228EUR181,50\n",
    )
    .expect("statement");
    archive.run(&["ledger", "bank", "import", statement.to_str().expect("utf-8")]);

    // The queue sees a pair, and one `--item` per invoice settles both.
    let queue = archive.run(&["ledger", "bank", "transactions", "--status", "new"]);
    let transaction = queue
        .lines()
        .nth(1)
        .and_then(|row| row.split_whitespace().next())
        .expect("the queue prints the transaction it wants matched")
        .to_owned();

    let said = archive.run(&["ledger", "bank", "propose", &transaction]);
    assert!(said.contains("2026-0001 + 2026-0002"), "{said}");
    assert!(said.contains("group"), "{said}");

    let items = archive.run(&["ledger", "open-items"]);
    let ids: Vec<String> = items
        .lines()
        .filter(|line| line.contains("2026-0001") || line.contains(&second))
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_owned)
        .collect();
    assert_eq!(ids.len(), 2, "both invoices are open: {items}");

    let mut match_args = vec!["ledger", "bank", "match", transaction.as_str()];
    for id in &ids {
        match_args.push("--item");
        match_args.push(id);
    }
    let said = archive.run(&match_args);
    assert!(said.contains("one payment settled 2 invoices"), "{said}");

    let said = archive.run(&["ledger", "open-items"]);
    assert!(said.contains("no open receivable"), "{said}");

    // One payment is one line of the bank account, whatever it settled.
    let said = archive.run(&["ledger", "trial-balance", "--from", "2026-01-01", "--to", "2026-12-31"]);
    assert!(said.trim_end().ends_with("balanced"), "{said}");
    assert!(
        archive
            .run(&["ledger", "bank", "transactions", "--status", "matched"])
            .contains("matched"),
        "the transaction left the queue"
    );
}

const RABO_PROFILE: &str = r#"
delimiter = ";"
decimal_separator = ","
date_format = "%d-%m-%Y"

[columns]
date = "Datum"
amount = "Bedrag"
counterparty = "Tegenpartij"
remittance = "Omschrijving"
"#;

#[test]
fn a_csv_statement_is_read_through_the_profile_beside_it() {
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

    let statement = archive.dir.path().join("feb.csv");
    std::fs::write(
        &statement,
        "Datum;Bedrag;Tegenpartij;Omschrijving\n20-02-2026;121,00;Klant BV;Betaling factuur 2026-0001\n",
    )
    .expect("statement");
    std::fs::write(archive.dir.path().join("feb.toml"), RABO_PROFILE).expect("profile");

    let said = archive.run(&["ledger", "bank", "import", statement.to_str().expect("utf-8")]);
    assert!(said.contains("profile: feb.toml"), "{said}");
    assert!(said.contains("1 imported"), "{said}");

    let said = archive.run(&["ledger", "bank", "auto-match"]);
    assert!(said.contains("matched 2026-0001"), "{said}");
    let said = archive.run(&["ledger", "open-items"]);
    assert!(said.contains("no open receivable"), "{said}");
}

#[test]
fn a_stored_profile_is_used_when_the_account_names_it() {
    let archive = Archive::new();
    archive.run(&["ledger", "setup"]);
    archive.run(&[
        "ledger",
        "bank",
        "add",
        "--name",
        "Zakelijk",
        "--iban",
        "NL91ABNA0417164300",
    ]);

    let profile = archive.dir.path().join("rabo.toml");
    std::fs::write(&profile, RABO_PROFILE).expect("profile");
    let said = archive.run(&[
        "ledger",
        "bank",
        "profile",
        "put",
        "rabo",
        "--file",
        profile.to_str().expect("utf-8"),
    ]);
    assert!(said.contains("profile:   rabo"), "{said}");
    let said = archive.run(&["ledger", "bank", "profile", "show", "rabo"]);
    assert!(
        said.contains("decimal_separator"),
        "what is stored is what it read: {said}"
    );

    let said = archive.run(&["ledger", "bank", "profile", "use", "Zakelijk", "rabo"]);
    assert!(said.contains("rabo"), "{said}");

    // No sidecar and no flag: the account's profile reads the file.
    let statement = archive.dir.path().join("maart.csv");
    std::fs::write(
        &statement,
        "Datum;Bedrag;Tegenpartij;Omschrijving\n05-03-2026;30,00;Energie BV;Voorschot\n",
    )
    .expect("statement");
    let said = archive.run(&["ledger", "bank", "import", statement.to_str().expect("utf-8")]);
    assert!(said.contains("profile: rabo"), "{said}");
    assert!(said.contains("1 imported"), "{said}");

    // The flag names a stored profile just as well, on a second file.
    let second = archive.dir.path().join("april.csv");
    std::fs::write(
        &second,
        "Datum;Bedrag;Tegenpartij;Omschrijving\n05-04-2026;40,00;Energie BV;Voorschot\n",
    )
    .expect("statement");
    let said = archive.run(&[
        "ledger",
        "bank",
        "import",
        second.to_str().expect("utf-8"),
        "--profile",
        "rabo",
    ]);
    assert!(said.contains("profile: rabo"), "{said}");
    assert!(said.contains("1 imported"), "{said}");
}

#[test]
fn a_csv_without_a_profile_is_refused_in_plain_words() {
    let archive = Archive::new();
    archive.run(&["ledger", "setup"]);
    archive.run(&["ledger", "bank", "add", "--name", "Zakelijk"]);

    let statement = archive.dir.path().join("raadsel.csv");
    std::fs::write(&statement, "Datum;Bedrag\n20-02-2026;121,00\n").expect("statement");

    let said = archive.fail(&["ledger", "bank", "import", statement.to_str().expect("utf-8")]);
    assert!(said.contains("needs an import profile"), "{said}");
    assert!(said.contains("raadsel.toml"), "it says where the profile goes: {said}");
}

#[test]
fn a_profile_that_cannot_be_read_is_refused_when_it_is_stored() {
    let archive = Archive::new();
    let profile = archive.dir.path().join("kapot.toml");
    std::fs::write(
        &profile,
        "date_format = \"%d %B %Y\"\n\n[columns]\ndate = \"Datum\"\namount = \"Bedrag\"\n",
    )
    .expect("profile");

    let said = archive.fail(&[
        "ledger",
        "bank",
        "profile",
        "put",
        "kapot",
        "--file",
        profile.to_str().expect("utf-8"),
    ]);
    assert!(said.contains("%d-%m-%Y"), "the layouts it does read are named: {said}");

    let said = archive.run(&["ledger", "bank", "profile", "list"]);
    assert!(said.contains("no import profiles"), "nothing was stored: {said}");
}

#[test]
fn removing_a_profile_leaves_the_account_saying_so() {
    let archive = Archive::new();
    archive.run(&["ledger", "setup"]);
    archive.run(&["ledger", "bank", "add", "--name", "Zakelijk"]);
    let profile = archive.dir.path().join("rabo.toml");
    std::fs::write(&profile, RABO_PROFILE).expect("profile");
    archive.run(&[
        "ledger",
        "bank",
        "profile",
        "put",
        "rabo",
        "--file",
        profile.to_str().expect("utf-8"),
    ]);
    archive.run(&["ledger", "bank", "profile", "use", "Zakelijk", "rabo"]);
    archive.run(&["ledger", "bank", "profile", "rm", "rabo"]);

    let statement = archive.dir.path().join("mei.csv");
    std::fs::write(&statement, "Datum;Bedrag\n05-05-2026;10,00\n").expect("statement");
    let said = archive.fail(&["ledger", "bank", "import", statement.to_str().expect("utf-8")]);
    assert!(said.contains("no longer exists"), "{said}");
}

#[test]
fn a_rule_books_a_bank_fee_without_an_invoice() {
    let archive = Archive::new();
    archive.run(&["ledger", "setup"]);
    archive.run(&[
        "ledger",
        "bank",
        "add",
        "--name",
        "Zakelijk",
        "--iban",
        "NL91ABNA0417164300",
    ]);

    // A fee, which no invoice will ever match, so it would sit in the queue.
    let statement = archive.dir.path().join("feb.940");
    std::fs::write(
        &statement,
        ":20:BONK-2026-02\n:60F:C260201EUR100,00\n:61:260220D12,50NTRF\n:86:Bankkosten februari\n:62F:C260228EUR87,50\n",
    )
    .expect("statement");
    archive.run(&["ledger", "bank", "import", statement.to_str().expect("utf-8")]);

    let said = archive.run(&[
        "ledger",
        "bank",
        "rule",
        "add",
        "--name",
        "bankkosten",
        "--remittance",
        "bankkosten",
        "--direction",
        "out",
        "--account",
        "7100",
    ]);
    assert!(said.contains("rule:    bankkosten"), "{said}");
    let said = archive.run(&["ledger", "bank", "rule", "list"]);
    assert!(said.contains("7100"), "{said}");
    assert!(said.contains("money out"), "{said}");

    // The dry run is the safe first look at a new rule.
    let said = archive.run(&["ledger", "bank", "classify", "--dry-run"]);
    assert!(said.contains("dry run: nothing was posted"), "{said}");
    assert!(said.contains("7100"), "it says where it would go: {said}");
    let said = archive.run(&["ledger", "bank", "transactions", "--status", "new"]);
    assert!(
        said.contains("Bankkosten februari"),
        "the line is still waiting: {said}"
    );

    let said = archive.run(&["ledger", "bank", "classify"]);
    assert!(said.contains("posted: 1"), "{said}");

    let queue = archive.run(&["ledger", "bank", "transactions", "--status", "posted"]);
    assert!(queue.contains("Bankkosten februari"), "{queue}");
    assert!(
        archive
            .run(&["ledger", "bank", "transactions", "--status", "new"])
            .contains("no bank transactions"),
        "the queue is empty"
    );

    // The cost is in the books and they still balance.
    let said = archive.run(&["ledger", "trial-balance", "--from", "2026-01-01", "--to", "2026-12-31"]);
    assert!(said.contains("7100"), "{said}");
    assert!(said.trim_end().ends_with("balanced"), "{said}");

    // A person changes their mind: the entry is reversed and the line is back.
    let transaction = queue
        .lines()
        .nth(1)
        .and_then(|row| row.split_whitespace().next())
        .expect("the queue prints the transaction")
        .to_owned();
    let said = archive.run(&[
        "ledger",
        "bank",
        "unclassify",
        &transaction,
        "--reason",
        "verkeerde rekening",
    ]);
    assert!(said.contains("the line is back in the queue"), "{said}");
    assert!(
        said.contains("still fits this line"),
        "it says the rule would take it again: {said}"
    );
    let said = archive.run(&["ledger", "bank", "transactions", "--status", "new"]);
    assert!(said.contains("Bankkosten februari"), "{said}");
    let said = archive.run(&["ledger", "trial-balance", "--from", "2026-01-01", "--to", "2026-12-31"]);
    // The account stays on the balance after a reversal — a correction is an entry,
    // not a delete — but the cost and its reversal cancel out.
    let row = said
        .lines()
        .find(|line| line.starts_with("7100"))
        .expect("the account stays on the balance");
    let columns: Vec<&str> = row.split_whitespace().collect();
    assert_eq!(
        columns[columns.len() - 2],
        columns[columns.len() - 1],
        "the cost and its reversal cancel: {row}"
    );
    assert!(said.trim_end().ends_with("balanced"), "{said}");

    let said = archive.run(&["ledger", "bank", "rule", "rm", "bankkosten"]);
    assert!(said.contains("removed rule bankkosten"), "{said}");
    assert!(
        archive
            .run(&["ledger", "bank", "rule", "list"])
            .contains("no bank rules")
    );
}

#[test]
fn a_purchase_is_put_on_an_expense_account_before_it_is_posted() {
    let archive = Archive::new();
    archive.run(&["ledger", "setup"]);

    // The archive's own invoice, imported back under a supplier's number, so there
    // is a received document to place.
    let issued = std::fs::read_to_string(format!("{}/2026-0001.xml", archive.out)).expect("the issued UBL");
    let received = archive.dir.path().join("inkoop.xml");
    std::fs::write(&received, issued.replacen("2026-0001", "INK-1", 1)).expect("the inbound UBL");
    let said = archive.run(&["import", received.to_str().expect("utf-8")]);
    assert!(said.contains("2026-0001") || said.contains("INK-1"), "{said}");

    // Until somebody decides, the cost has nowhere to go.
    let said = archive.run(&["ledger", "account", "INK-1"]);
    assert!(said.contains("none"), "{said}");
    assert!(said.contains("9999"), "{said}");

    let said = archive.run(&["ledger", "account", "INK-1", "7100"]);
    assert!(said.contains("account:   7100"), "{said}");

    let said = archive.run(&["ledger", "post"]);
    assert!(said.contains("posted INK-1"), "{said}");

    // The cost is on the chosen account, and nothing is left unclassified.
    let said = archive.run(&["ledger", "trial-balance", "--from", "2026-01-01", "--to", "2026-12-31"]);
    assert!(said.contains("7100"), "{said}");
    assert!(!said.contains("9999"), "nothing unclassified: {said}");
    assert!(said.trim_end().ends_with("balanced"), "{said}");

    // The entry snapshotted the account, so it cannot be changed afterwards.
    let said = archive.fail(&["ledger", "account", "INK-1", "7000"]);
    assert!(said.contains("already posted"), "{said}");
}

/// A purchase posted before anybody decided where its cost goes can still be put
/// on an account afterwards: the refusal names the remedy, so the remedy has to
/// exist in the interface too.
#[test]
fn a_posted_purchase_is_moved_to_another_account() {
    let archive = Archive::new();
    archive.run(&["ledger", "setup"]);

    // The archive's own invoice, imported back under a supplier's number and posted
    // with no account chosen, so its base sits on 9999.
    let issued = std::fs::read_to_string(format!("{}/2026-0001.xml", archive.out)).expect("the issued UBL");
    let received = archive.dir.path().join("inkoop.xml");
    std::fs::write(&received, issued.replacen("2026-0001", "INK-1", 1)).expect("the inbound UBL");
    archive.run(&["import", received.to_str().expect("utf-8")]);
    archive.run(&["ledger", "post"]);

    // Dated in the month it corrects: a correction booked elsewhere leaves the
    // balance it was meant to fix exactly where it was.
    let said = archive.run(&[
        "ledger",
        "reclassify",
        "IK:2026-0001",
        "--account",
        "7100",
        "--reason",
        "verkeerde rekening",
        "--date",
        "2026-02-28",
    ]);
    assert!(said.contains("reversed 2026-0001 as IK 2026-0002"), "{said}");
    assert!(said.contains("moved    IK 2026-0003 to 7100"), "{said}");
    assert!(said.contains("dated 2026-02-28"), "{said}");

    let said = archive.run(&["ledger", "trial-balance", "--from", "2026-01-01", "--to", "2026-12-31"]);
    let cost = said
        .lines()
        .find(|line| line.starts_with("7100"))
        .expect("the cost is on 7100");
    assert!(cost.contains("100.00"), "the invoice's net is on 7100 now: {cost}");
    let guard = said
        .lines()
        .find(|line| line.starts_with("9999"))
        .expect("the guard account keeps its history");
    let mut amounts = guard.split_whitespace().filter(|token| token.contains('.'));
    let debit = amounts.next().expect("a debit is printed");
    let credit = amounts.next().expect("a credit is printed");
    assert_eq!(debit, credit, "the guard's two sides cancel: {guard}");
    assert_eq!(amounts.next(), None, "so it has no balance left: {guard}");
    assert!(said.trim_end().ends_with("balanced"), "{said}");

    // The entry is append-only, so the same correction cannot be written twice.
    let said = archive.fail(&[
        "ledger",
        "reclassify",
        "IK:2026-0001",
        "--account",
        "7000",
        "--reason",
        "again",
        "--date",
        "2026-02-28",
    ]);
    assert!(said.contains("already reversed"), "{said}");

    // And the month it was posted in can close now.
    let said = archive.run(&["ledger", "close", "--year", "2026", "--month", "2"]);
    assert!(said.contains("closed:  2026-02 by cli"), "{said}");
}

/// A year closes from the command line: the result lands on equity in the
/// adjustment period, the months lock, and a document nobody posted stops it.
#[test]
fn a_year_closes_and_reopens_from_the_command_line() {
    let archive = Archive::new();
    archive.run(&["ledger", "setup"]);
    archive.run(&["ledger", "post"]);

    // An issued invoice nobody posted means the year's result is not the year's
    // result yet, and the close names it.
    archive.sell("50.00");
    let said = archive.run(&["ledger", "close-year", "--year", "2026", "--dry-run"]);
    assert!(said.contains("close:    blocked"), "{said}");
    assert!(
        said.contains("1 document(s) dated in 2026 have not been posted: 2026-0002"),
        "{said}"
    );
    let said = archive.fail(&["ledger", "close-year", "--year", "2026"]);
    assert!(said.contains("was not closed"), "{said}");

    archive.run(&["ledger", "post"]);
    let said = archive.run(&["ledger", "close-year", "--year", "2026", "--dry-run"]);
    assert!(said.contains("result:   150.00 EUR to 0500 Eigen vermogen"), "{said}");
    assert!(said.contains("close:    nothing blocks it"), "{said}");
    assert!(
        said.contains("warn:     the btw-aangifte for 2026-02 has not been prepared"),
        "a filing gap is reported, not enforced: {said}"
    );

    let said = archive.run(&["ledger", "close-year", "--year", "2026"]);
    assert!(said.contains("posted:   MEM 2026-0001 resultaatbestemming"), "{said}");
    assert!(
        said.contains("closed:   2026; 1 month(s) locked by this close"),
        "{said}"
    );

    // The year takes no postings now, and it cannot be closed twice.
    let said = archive.fail(&[
        "ledger",
        "post-manual",
        "--date",
        "2026-02-20",
        "--description",
        "Late",
        "--line",
        "7100|debit|10.00",
        "--line",
        "1100|credit|10.00",
    ]);
    assert!(said.contains("fiscal year 2026 is closed"), "{said}");
    let said = archive.fail(&["ledger", "close-year", "--year", "2026"]);
    assert!(said.contains("already closed"), "{said}");

    // Reopening reverses the resultaatbestemming; the months stay closed, so the
    // month a correction belongs in is reopened on its own.
    let said = archive.run(&[
        "ledger",
        "reopen-year",
        "--year",
        "2026",
        "--reason",
        "vergeten factuur",
    ]);
    assert!(said.contains("reversed MEM 2026-0002 resultaatbestemming"), "{said}");
    assert!(said.contains("reopened: 2026 by cli: vergeten factuur"), "{said}");
    let said = archive.run(&[
        "ledger",
        "profit-and-loss",
        "--from",
        "2026-01-01",
        "--to",
        "2026-12-31",
    ]);
    assert!(said.contains("150.00"), "the revenue is back: {said}");

    archive.run(&[
        "ledger",
        "reopen",
        "--year",
        "2026",
        "--month",
        "2",
        "--reason",
        "vergeten factuur",
    ]);
    archive.run(&[
        "ledger",
        "post-manual",
        "--date",
        "2026-02-20",
        "--description",
        "Bankkosten",
        "--line",
        "7100|debit|10.00",
        "--line",
        "1100|credit|10.00",
    ]);
    let said = archive.run(&["ledger", "close-year", "--year", "2026", "--dry-run"]);
    assert!(said.contains("result:   140.00 EUR"), "{said}");
    assert!(said.contains("close:    nothing blocks it"), "{said}");

    let said = archive.run(&["ledger", "close-year", "--year", "2026"]);
    assert!(said.contains("posted:   MEM 2026-0004 resultaatbestemming"), "{said}");
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
    archive.run(&["ledger", "setup"]);
    archive.run(&["ledger", "post"]);

    let said = archive.run(&["ledger", "tax-codes"]);
    assert!(said.contains("NL-AE-SERVICES"), "{said}");
    assert!(said.contains("NL-AE-GOODS"), "{said}");
    assert!(said.contains("4b"), "the box a code resolves is shown: {said}");

    // Reading what a document carries needs nothing posted and says when none is.
    let said = archive.run(&["ledger", "tax-code", "2026-0001"]);
    assert!(said.contains("none"), "{said}");

    // Only received documents take a code: a sale is placed by its own category.
    let said = archive.fail(&["ledger", "tax-code", "2026-0001", "NL-AE-GOODS"]);
    assert!(said.contains("we issued"), "{said}");

    // And no cost account either, for the same reason.
    let said = archive.fail(&["ledger", "account", "2026-0001", "7100"]);
    assert!(said.contains("we issued"), "{said}");

    // The same applies to a correction: there is no box to move on a sale.
    let said = archive.fail(&["ledger", "reclassify", "VK:2026-0001", "NL-AE-GOODS", "--reason", "x"]);
    assert!(said.contains("only a purchase"), "{said}");
}

/// A month is closed from the command line, refuses postings while it is closed,
/// and opens again with a recorded reason.
#[test]
fn a_month_closes_and_reopens_from_the_command_line() {
    let archive = Archive::new();
    archive.run(&["ledger", "setup"]);
    archive.run(&["ledger", "post"]);

    // The archive's invoice is dated 10 February, so that is the month.
    let said = archive.run(&["ledger", "close", "--year", "2026", "--month", "2", "--dry-run"]);
    assert!(
        said.contains("period:  2026-02 (2026-02-01 to 2026-02-28), open"),
        "{said}"
    );
    assert!(said.contains("close:   nothing blocks it"), "{said}");

    let said = archive.run(&["ledger", "close", "--year", "2026", "--month", "2"]);
    assert!(said.contains("closed:  2026-02 by cli"), "{said}");

    let said = archive.fail(&["ledger", "close", "--year", "2026", "--month", "2"]);
    assert!(said.contains("already closed"), "{said}");

    // While it is closed, nothing can be posted into it.
    let said = archive.fail(&[
        "ledger",
        "post-manual",
        "--date",
        "2026-02-15",
        "--description",
        "Bankkosten",
        "--line",
        "7100|debit|12.50",
        "--line",
        "1100|credit|12.50",
    ]);
    assert!(said.contains("2026-02 is locked"), "{said}");

    // Reopening needs a reason, and the reason is printed back.
    let said = archive.fail(&["ledger", "reopen", "--year", "2026", "--month", "2"]);
    assert!(said.contains("--reason"), "{said}");

    let said = archive.run(&[
        "ledger",
        "reopen",
        "--year",
        "2026",
        "--month",
        "2",
        "--reason",
        "vergeten factuur",
    ]);
    assert!(said.contains("reopened: 2026-02 by cli: vergeten factuur"), "{said}");

    let said = archive.run(&[
        "ledger",
        "post-manual",
        "--date",
        "2026-02-15",
        "--description",
        "Bankkosten",
        "--line",
        "7100|debit|12.50",
        "--line",
        "1100|credit|12.50",
    ]);
    assert!(said.contains("posted MEM 2026-0001"), "{said}");

    // Reopening a month that is already open changes nothing and says so.
    let said = archive.fail(&[
        "ledger", "reopen", "--year", "2026", "--month", "2", "--reason", "again",
    ]);
    assert!(said.contains("not closed"), "{said}");

    let said = archive.run(&["ledger", "close", "--year", "2026", "--month", "2"]);
    assert!(said.contains("closed:  2026-02 by cli"), "{said}");
}

/// A close whose blocker cannot be had says which month is missing, and a month
/// that holds unplaced costs refuses to close.
#[test]
fn a_close_names_what_stands_in_the_way() {
    let archive = Archive::new();
    archive.run(&["ledger", "setup"]);

    let said = archive.fail(&["ledger", "close", "--year", "2026", "--month", "2"]);
    assert!(said.contains("no period for 2026-02"), "{said}");

    // The archive's own invoice, imported back under a supplier's number, is a
    // purchase nobody has placed an account on: its base lands on 9999.
    let issued = std::fs::read_to_string(format!("{}/2026-0001.xml", archive.out)).expect("the issued UBL");
    let received = archive.dir.path().join("inkoop.xml");
    std::fs::write(&received, issued.replacen("2026-0001", "INK-1", 1)).expect("the inbound UBL");
    archive.run(&["import", received.to_str().expect("utf-8")]);
    archive.run(&["ledger", "post"]);

    let said = archive.run(&["ledger", "close", "--year", "2026", "--month", "2", "--dry-run"]);
    assert!(said.contains("close:   blocked"), "{said}");
    assert!(said.contains("9999"), "the account is named: {said}");
    assert!(said.contains("memorial entry"), "the remedy is named: {said}");

    let said = archive.fail(&["ledger", "close", "--year", "2026", "--month", "2"]);
    assert!(said.contains("was not closed"), "{said}");
    assert!(said.contains("listed above"), "{said}");
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
