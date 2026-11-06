#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! The `bonk` executable: GUI by default, headless subcommands for support and CI.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use app::{Session, strength, text};
use clap::{Parser, Subcommand};
use crux::{
    AccountCode, Address, CountryCode, Currency, CustomerId, DocumentId, DraftLine, Finding, InvoiceDraft, Journal,
    JournalEntryId, LedgerLine, LineId, LocalDate, Money, Party, Quantity, SellerId, Severity, Side, TaxCode,
    TaxTreatment, UnitCode, VatCategory, VatRate, calculate_totals, has_blocking, validate_draft,
};
use store::{
    CustomerRecord, DocumentStatus, IssueRequest, KeyStore, SearchRequest, SellerRecord, StoreError, advance_status,
    credit_note_for, first_seller, insert_customer, insert_draft, insert_seller, issue_draft, list_received,
    load_customer, load_draft, load_seller, resolve_customer, resolve_document, resolve_received, set_supplier_match,
};
use zeroize::Zeroizing;

mod demo;

#[derive(Parser)]
#[command(name = "bonk", version, about = "Local-first e-invoicing client (working name)")]
struct Cli {
    /// Database file. Defaults to the platform data directory.
    #[arg(long, global = true)]
    db: Option<PathBuf>,

    /// Keystore file. Defaults to `keystore.bin` beside the database.
    #[arg(long, global = true)]
    keystore: Option<PathBuf>,

    /// Passphrase. Prefer `BONK_PASSPHRASE` or the interactive prompt.
    #[arg(long, global = true)]
    passphrase: Option<String>,

    /// Open the archive with the key this device remembered, instead of a
    /// passphrase. Commands that change credentials still ask for the passphrase.
    #[arg(long, global = true)]
    device: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Create an encrypted database and its keystore.
    Init,
    /// Fill a new archive with demo data for trying the window.
    ///
    /// The data is generated deterministically from `--seed`: a seller, a
    /// catalogue of customers, issued invoices (some corrected by credit
    /// notes), documents received from suppliers, a few drafts, and saved
    /// search views. It is ordinary archive data, so `verify` passes on it.
    #[command(name = "demo-seed")]
    DemoSeed {
        /// Customers to create; the catalogue holds 18.
        #[arg(long, default_value_t = 18)]
        customers: usize,
        /// Issued documents, credit notes included.
        #[arg(long, default_value_t = 320)]
        invoices: usize,
        /// Documents received from suppliers.
        #[arg(long, default_value_t = 45)]
        received: usize,
        /// Generator seed; the same seed and day produce the same data.
        #[arg(long, default_value_t = 2026)]
        seed: u64,
        /// Replace an existing archive at the target paths.
        #[arg(long)]
        force: bool,
    },
    /// Remember this device: keep the archive key in the OS credential store.
    ///
    /// Afterwards `--device` opens the archive without a passphrase. While it is
    /// on, the archive is protected by your operating system login rather than by
    /// the passphrase; `forget` undoes it.
    Remember,
    /// Forget this device, so the passphrase is needed again.
    ///
    /// Works with `--device` too, which is the case that matters: revoking the
    /// convenience should not need the passphrase it replaced.
    Forget,
    /// Check integrity, the audit chain, and every stored artefact.
    Verify {
        /// Check the chain against checkpoints recorded outside the archive.
        #[arg(long)]
        anchor: Option<PathBuf>,
    },
    /// Record the audit chain head in a file kept outside the archive.
    ///
    /// A hash chain cannot prove it is the original: whoever rewrites every row
    /// produces a chain that verifies against itself. A copy of a head kept
    /// elsewhere is what closes that gap, so this appends one line per new head
    /// to a file you keep apart from the database — printed, mailed to yourself,
    /// or in a repository. `bonk verify --anchor <file>` checks it later.
    Anchor {
        /// File holding the checkpoints. Created on first use.
        file: PathBuf,
    },
    /// Watch a folder and import invoices dropped into it.
    ///
    /// Files already in the folder when the watcher starts are left alone; it acts
    /// on new or changed files. Polling rather than change notifications is
    /// deliberate: drop folders are often network shares, where notifications do
    /// not work, and what matters is that the writer has finished.
    Watch {
        /// Folder to watch.
        directory: PathBuf,
        /// Seconds between polls.
        #[arg(long, default_value_t = 2)]
        every: u64,
        /// Seconds a file must be untouched before it is read.
        #[arg(long, default_value_t = 5)]
        quiet_for: u64,
        /// Read the folder once and exit, for cron or a deliberate batch.
        #[arg(long)]
        once: bool,
    },
    /// Write a verified, encrypted backup and prune older ones.
    Backup {
        /// Directory for backups. Defaults to `backups` beside the database.
        #[arg(long)]
        to: Option<PathBuf>,
        /// How many backups to keep.
        #[arg(long, default_value_t = store::backup::RETENTION)]
        keep: usize,
    },
    /// Write a copy of the archive key, with instructions, for a restore.
    ///
    /// The database backups do not contain the key: a restore needs both, from
    /// two places, so this belongs on a different medium than the archive.
    #[command(name = "recovery-kit")]
    RecoveryKit {
        /// Directory to write the kit into. Must not be the archive directory.
        directory: PathBuf,
    },
    /// Verify an evidence bundle without the archive.
    #[command(name = "evidence-verify")]
    EvidenceVerify {
        /// Bundle directory.
        directory: PathBuf,
    },
    /// Write a verifiable evidence bundle for one document.
    Evidence {
        /// Document number or identifier.
        document: String,
        /// Directory to write the bundle into; it must not already hold one.
        #[arg(long)]
        out: PathBuf,
    },
    /// Write a support bundle: counts, versions, and integrity state, no data.
    #[command(name = "support-bundle")]
    SupportBundle {
        /// Directory to write into. Defaults to `support-bundle` beside the database.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Verify a backup without opening it as the live database.
    #[command(name = "verify-backup")]
    VerifyBackup {
        /// Backup file to check.
        path: PathBuf,
    },
    /// Rewrap the keystore with a new passphrase, keeping the database key.
    #[command(name = "passphrase-change")]
    PassphraseChange {
        /// New passphrase. A flag is for tests and scripts; prefer the prompt or
        /// `BONK_NEW_PASSPHRASE`.
        #[arg(long)]
        new_passphrase: Option<String>,
    },
    /// Add a recovery key that can open the archive without the passphrase.
    ///
    /// The key is printed once. Replacing an existing recovery key is allowed;
    /// the old one stops working.
    #[command(name = "recovery-add")]
    RecoveryAdd,
    /// Replace a lost passphrase with the recovery key, then rotate that key.
    #[command(name = "passphrase-reset")]
    PassphraseReset {
        /// Recovery key. A flag is for tests and scripts; prefer the prompt or
        /// `BONK_RECOVERY_KEY`.
        #[arg(long)]
        recovery_key: Option<String>,
        /// New passphrase. A flag is for tests and scripts; prefer the prompt or
        /// `BONK_NEW_PASSPHRASE`.
        #[arg(long)]
        new_passphrase: Option<String>,
    },
    /// Roll back reversible migrations. Refuses to cross an irreversible one.
    #[command(name = "migrate-down")]
    MigrateDown {
        /// Target schema version.
        #[arg(long)]
        to: i64,
        /// Confirm the rollback. Required because down migrations can drop data.
        #[arg(long)]
        yes: bool,
    },
    /// Add a seller and print its identifier.
    #[command(name = "seller-add")]
    SellerAdd {
        #[arg(long)]
        name: String,
        #[arg(long)]
        city: String,
        #[arg(long, default_value = "NL")]
        country: String,
        #[arg(long)]
        vat: Option<String>,
        #[arg(long)]
        kvk: Option<String>,
        /// Peppol endpoint identifier, `EAS:value`, e.g. `0106:12345678`.
        #[arg(long)]
        peppol: Option<String>,
        #[arg(long)]
        street: Option<String>,
        #[arg(long)]
        postal: Option<String>,
        #[arg(long)]
        email: Option<String>,
        #[arg(long)]
        contact: Option<String>,
        #[arg(long, default_value = "EUR")]
        currency: String,
        #[arg(long, default_value = "nl-NL")]
        locale: String,
    },
    /// Add a customer and print its identifier.
    #[command(name = "customer-add")]
    CustomerAdd {
        #[arg(long)]
        name: String,
        #[arg(long)]
        city: String,
        #[arg(long, default_value = "NL")]
        country: String,
        #[arg(long)]
        vat: Option<String>,
        #[arg(long)]
        kvk: Option<String>,
        /// Peppol endpoint identifier, `EAS:value`, e.g. `0106:12345678`.
        #[arg(long)]
        peppol: Option<String>,
        #[arg(long)]
        street: Option<String>,
        #[arg(long)]
        postal: Option<String>,
        #[arg(long)]
        email: Option<String>,
        #[arg(long)]
        contact: Option<String>,
    },
    /// Create a draft. Each `--line` is `name|quantity|unit_price|rate[|category[|unit[|exemption_code[|exemption_reason]]]]`.
    #[command(name = "draft-new")]
    DraftNew {
        #[arg(long)]
        seller: String,
        #[arg(long)]
        customer: String,
        /// `YYYY-MM-DD`; defaults to today.
        #[arg(long)]
        date: Option<String>,
        /// `YYYY-MM-DD`.
        #[arg(long)]
        due: Option<String>,
        /// Actual delivery date, `YYYY-MM-DD`; required for intra-community supply.
        #[arg(long)]
        delivery: Option<String>,
        #[arg(long = "line", required = true)]
        lines: Vec<String>,
        #[arg(long)]
        note: Option<String>,
        #[arg(long)]
        buyer_ref: Option<String>,
        #[arg(long)]
        order_ref: Option<String>,
        /// Layout the PDF is drawn with, by template file name: `--template compact`
        /// means `templates/compact.json`. Omit for the archive default.
        #[arg(long)]
        template: Option<String>,
    },
    /// Create a credit note (381) that copies an issued invoice.
    #[command(name = "credit-note")]
    CreditNote {
        /// Number of the issued invoice to correct, e.g. `2026-0001`.
        invoice: String,
        /// Seller identifier; defaults to the first seller.
        #[arg(long)]
        seller: Option<String>,
        /// `YYYY-MM-DD`; defaults to today.
        #[arg(long)]
        date: Option<String>,
        /// Replaces the copied note.
        #[arg(long)]
        note: Option<String>,
        /// Print the draft without storing it.
        #[arg(long)]
        dry_run: bool,
    },
    /// Import invoices received from suppliers. Each argument is a UBL file.
    Import {
        /// Files to import.
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// Owner; defaults to the first seller.
        #[arg(long)]
        seller: Option<String>,
    },
    /// List documents received from suppliers.
    #[command(name = "received")]
    Received {
        /// Only show documents in this status.
        #[arg(long)]
        status: Option<String>,
    },
    /// Match a received document to a customer (supplier).
    #[command(name = "match")]
    Match {
        /// Document number or identifier.
        document: String,
        /// Customer identifier or display name.
        customer: String,
    },
    /// Search documents by number, party, reference, or note.
    Search {
        /// Words to look for. Every word must appear.
        query: String,
        /// Restrict to documents we issued, or to those we received.
        #[arg(long, value_parser = ["outbound", "inbound"])]
        direction: Option<String>,
        /// Earliest issue date, `YYYY-MM-DD`.
        #[arg(long)]
        from: Option<String>,
        /// Latest issue date, `YYYY-MM-DD`.
        #[arg(long)]
        to: Option<String>,
        /// Only this status: issued, received, approved, paid, or archived.
        #[arg(long)]
        status: Option<String>,
    },
    /// Time the search query against the archive (M5 exit criterion).
    ///
    /// The archive is unlocked once, outside the timed loop: the criterion is
    /// about the query, not about the Argon2id run that opens the key.
    #[command(name = "bench-search")]
    BenchSearch {
        /// Words to look for; an empty query lists everything.
        #[arg(default_value = "")]
        query: String,
        /// Samples to take; the p95 is taken over these.
        #[arg(long, default_value_t = 50)]
        repeat: usize,
    },
    /// Export documents as CSV or JSON.
    Export {
        /// Words to narrow the export; omit to export everything.
        #[arg(default_value = "")]
        query: String,
        /// Output format.
        #[arg(long, default_value = "csv", value_parser = ["csv", "json"])]
        format: String,
        /// Restrict to documents we issued, or to those we received.
        #[arg(long, value_parser = ["outbound", "inbound"])]
        direction: Option<String>,
        /// Earliest issue date, `YYYY-MM-DD`.
        #[arg(long)]
        from: Option<String>,
        /// Latest issue date, `YYYY-MM-DD`.
        #[arg(long)]
        to: Option<String>,
        /// Only this status: issued, received, approved, paid, or archived.
        #[arg(long)]
        status: Option<String>,
        /// File to write; standard output when omitted.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Approve a received document for payment.
    Approve {
        /// Document number or identifier.
        document: String,
    },
    /// Mark a received document as paid.
    #[command(name = "mark-paid")]
    MarkPaid {
        /// Document number or identifier.
        document: String,
    },
    /// Validate and issue a draft; writes the UBL artefact to `--out`.
    Issue {
        /// Draft identifier.
        draft: String,
        #[arg(long, default_value = ".")]
        out: PathBuf,
        #[arg(long, default_value = "cli")]
        actor: String,
        /// Name of the device that recorded the issue, for the audit chain.
        #[arg(long = "device-name", default_value = "cli")]
        device_name: String,
        /// Language of the printed document: `en` or `nl`. Defaults to the
        /// seller's locale.
        #[arg(long, value_parser = ["en", "nl"])]
        language: Option<String>,
    },
    /// The general ledger: the books derived from the archive.
    Ledger {
        #[command(subcommand)]
        command: LedgerCommand,
    },
    /// Open the desktop UI (also the default with no subcommand).
    Gui,
}

/// Subcommands of [`LedgerCommand::Bank`].
#[derive(Debug, Subcommand)]
enum BankCommand {
    /// Add a bank account whose statements will be imported.
    Add {
        /// What the user calls it.
        #[arg(long)]
        name: String,
        /// IBAN, when there is one.
        #[arg(long)]
        iban: Option<String>,
        /// The ledger account the balance lives on.
        #[arg(long, default_value = "1100")]
        ledger: String,
    },
    /// List the bank accounts of these books.
    Accounts,
    /// Import a CAMT.053 statement file.
    ///
    /// A file already imported is a no-op, and a payment that also appears in an
    /// overlapping statement is stored once; a statement whose own balances do
    /// not add up is refused whole.
    Import {
        /// The statement file.
        file: PathBuf,
        /// Bank account id or name; inferred when there is only one.
        #[arg(long)]
        account: Option<String>,
    },
    /// List imported transactions, newest first.
    Transactions {
        /// Only this status: new, matched, partially_matched, posted, or ignored.
        #[arg(long)]
        status: Option<String>,
    },
    /// Show what a transaction could settle.
    Propose {
        /// The transaction identifier.
        transaction: String,
    },
    /// Match every payment whose remittance names the invoice.
    ///
    /// Only a reference-and-amount match is automatic; an amount that fits two
    /// invoices is left for a person.
    #[command(name = "auto-match")]
    AutoMatch,
    /// Match one transaction to one open item.
    Match {
        /// The transaction identifier.
        transaction: String,
        /// The open item identifier, as `open-items` prints it.
        #[arg(long = "item")]
        item: String,
    },
    /// Undo a match, appending its reversal.
    Unmatch {
        /// The allocation identifier, as a match reports it.
        allocation: String,
    },
}

/// Subcommands of [`Command::Ledger`].
#[derive(Debug, Subcommand)]
enum LedgerCommand {
    /// Seed the starter chart of accounts.
    ///
    /// Accounts already present are left alone, so this is safe to re-run after
    /// a rename. Nothing is posted until the chart exists.
    Setup,
    /// Post documents that have no entry yet.
    ///
    /// With no arguments every unposted document of the books is posted, both the
    /// invoices we issued and the ones we received. Naming numbers posts only
    /// those, and only numbers we issued: a supplier's number is theirs and does
    /// not identify a document on its own.
    Post {
        /// Document numbers, e.g. `2026-0001`.
        documents: Vec<String>,
        #[arg(long, default_value = "cli")]
        actor: String,
    },
    /// Show what the chart of accounts holds.
    Chart {
        /// Include archived accounts.
        #[arg(long)]
        all: bool,
    },
    /// Print the trial balance (proefbalans) for a window.
    #[command(name = "trial-balance")]
    TrialBalance {
        /// First day, `YYYY-MM-DD`; defaults to 1 January of the current year.
        #[arg(long)]
        from: Option<String>,
        /// Last day, `YYYY-MM-DD`; defaults to today.
        #[arg(long)]
        to: Option<String>,
    },
    /// Print the profit and loss statement (winst- en verliesrekening).
    #[command(name = "profit-and-loss")]
    ProfitAndLoss {
        /// First day, `YYYY-MM-DD`; defaults to 1 January of the current year.
        #[arg(long)]
        from: Option<String>,
        /// Last day, `YYYY-MM-DD`; defaults to today.
        #[arg(long)]
        to: Option<String>,
    },
    /// Print the balance sheet (balans) at a date.
    #[command(name = "balance-sheet")]
    BalanceSheet {
        /// The date, `YYYY-MM-DD`; defaults to today.
        #[arg(long)]
        as_of: Option<String>,
    },
    /// Preview or prepare the VAT return (btw-aangifte) for a month.
    ///
    /// Without `--prepare` nothing is written: the figures are computed from the
    /// ledger and printed. With it, the return is stored as prepared together with
    /// the entries it was computed from, which is what makes it evidence.
    #[command(name = "vat-return")]
    VatReturn {
        /// Year; defaults to the month being reported on.
        #[arg(long)]
        year: Option<i32>,
        /// Month 1-12; defaults to the month before this one.
        #[arg(long)]
        month: Option<u8>,
        /// Store the return instead of only printing it.
        #[arg(long)]
        prepare: bool,
        #[arg(long, default_value = "cli")]
        actor: String,
    },
    /// Import bank statements and see what is unmatched.
    Bank {
        #[command(subcommand)]
        command: BankCommand,
    },
    /// List open items, or age them (openstaande posten).
    #[command(name = "open-items")]
    OpenItems {
        /// `receivable` (default) or `payable`.
        #[arg(long)]
        kind: Option<String>,
        /// Show the aging buckets instead of the items.
        #[arg(long)]
        aging: bool,
        /// The date to age at, `YYYY-MM-DD`; defaults to today.
        #[arg(long)]
        as_of: Option<String>,
    },
    /// Write the subledger for documents posted before it existed.
    ///
    /// An archive posted before migration 0013 has journal entries but no open
    /// items, so the debtors and creditors accounts cannot be reconciled, and
    /// re-posting does not help because posting is idempotent per document.
    /// This fills the gap from the documents. It only adds: an item that already
    /// exists is never rewritten, because allocations may point at it.
    #[command(name = "rebuild-open-items")]
    RebuildOpenItems {
        #[arg(long, default_value = "cli")]
        actor: String,
    },
    /// List the treatments a purchase can be filed under.
    #[command(name = "tax-codes")]
    TaxCodes,
    /// Show or choose the treatment a received document is filed under.
    ///
    /// An EN 16931 category cannot say whether an `AE` purchase is goods or
    /// services, and the Dutch return asks for those in different boxes. The
    /// choice has to be made before the document is posted, because the entry
    /// snapshots it: a posted treatment is not rewritten.
    #[command(name = "tax-code")]
    TaxCode {
        /// The document, by number or identifier.
        document: String,
        /// The code to assign; omit it to show what is assigned.
        #[arg(conflicts_with = "clear")]
        code: Option<String>,
        /// Return the document to the treatment its own category implies.
        #[arg(long)]
        clear: bool,
        #[arg(long, default_value = "cli")]
        actor: String,
    },
    /// Post a balanced entry a person wrote (memoriaal).
    ///
    /// The escape hatch for a correction no rule can derive. A line is
    /// `account|side|amount`, optionally followed by `|category|rate` and then a
    /// tax code; the code is what puts a VAT line in the box the category alone
    /// cannot name, so a mis-boxed purchase is corrected by reversing its entry
    /// and posting the corrected amounts under the right code.
    #[command(name = "post-manual")]
    PostManual {
        /// The date the entry is booked, `YYYY-MM-DD`; it decides the period.
        #[arg(long)]
        date: String,
        /// What happened, in words; it is what a reader of the journal sees.
        #[arg(long)]
        description: String,
        /// A line: `account|side|amount`, optionally `|category|rate` and `|code`.
        #[arg(long = "line", required = true)]
        line: Vec<String>,
        #[arg(long, default_value = "cli")]
        actor: String,
    },
    /// File a posted purchase under a different treatment.
    ///
    /// Reverses the entry and posts the same amounts again with the code that
    /// names the right box, in one transaction, so a correction never leaves the
    /// supply counted twice or missing. Use it when a purchase was posted before
    /// its treatment was known; afterwards `bonk ledger tax-code` is refused.
    #[command(name = "reclassify")]
    Reclassify {
        /// The entry, as `IK:2026-0001` or by identifier.
        entry: String,
        /// The code that names where it belongs, e.g. NL-AE-GOODS.
        code: String,
        /// Why it is being corrected; it travels in both descriptions.
        #[arg(long)]
        reason: String,
        /// The date the correction is booked; defaults to today.
        #[arg(long)]
        date: Option<String>,
        #[arg(long, default_value = "cli")]
        actor: String,
    },
    /// Reverse one entry, naming it `VK:2026-0001` or by identifier.
    Reverse {
        /// Journal and number, or the entry identifier.
        entry: String,
        /// Why it is being reversed; it travels in the entry's description.
        #[arg(long)]
        reason: String,
        /// The date the correction is booked, `YYYY-MM-DD`; defaults to today.
        #[arg(long)]
        date: Option<String>,
        #[arg(long, default_value = "cli")]
        actor: String,
    },
}

#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Domain(#[from] crux::DomainError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Input(String),
    #[error("could not open the window: {0}")]
    Gui(String),
    #[error("blocking validation findings; nothing was issued")]
    Validation,
    #[error(transparent)]
    Demo(#[from] demo::DemoError),
}

/// Parses `account|side|amount[|category|rate[|code]]`.
fn parse_manual_line(raw: &str, currency: Currency) -> Result<LedgerLine, CliError> {
    const SHAPE: &str = "a line is `account|side|amount`, optionally `|category|rate` and then a tax code";
    let parts: Vec<&str> = raw.split('|').map(str::trim).collect();
    if !matches!(parts.len(), 3 | 5 | 6) {
        return Err(CliError::Input(format!("{raw:?}: {SHAPE}")));
    }
    let explained = |error: crux::DomainError| CliError::Input(format!("{raw:?}: {error}"));

    let account = AccountCode::parse(parts[0]).map_err(explained)?;
    let side = Side::parse(parts[1]).map_err(explained)?;
    let amount = Money::parse_fixed(parts[2], currency).map_err(explained)?;
    let mut line = LedgerLine::new(account, side, amount).map_err(explained)?;
    if parts.len() >= 5 {
        let category = VatCategory::parse(parts[3]).map_err(explained)?;
        let rate = VatRate::parse_percent(parts[4]).map_err(explained)?;
        line = line.tagged(category, rate);
    }
    if parts.len() == 6 {
        line = line.coded(TaxCode::parse(parts[5]).map_err(explained)?);
    }
    Ok(line)
}

fn main() -> ExitCode {
    // Parsing builds the whole subcommand tree as one deeply nested expression,
    // and an unoptimised build keeps every temporary of it on the stack. With
    // this many subcommands that no longer fits the 1 MiB a process's main
    // thread gets, so the parse runs on a thread with room. The window still
    // opens on the main thread, which the operating systems require.
    let cli = std::thread::Builder::new()
        .name("cli".to_owned())
        .stack_size(16 * 1024 * 1024)
        .spawn(Cli::parse)
        .expect("the parser thread starts")
        .join()
        .unwrap_or_else(|_| Cli::parse_from(["bonk"]));

    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), CliError> {
    warn_if_unencrypted();
    let (database, keystore) = paths(&cli);
    let access = Access {
        passphrase: cli.passphrase.clone(),
        device: cli.device,
    };
    match cli.command {
        None | Some(Command::Gui) => app::run(database, keystore).map_err(|error| CliError::Gui(error.to_string())),
        Some(Command::Remember) => {
            if !store::device::available() {
                return Err(CliError::Input(
                    "this system has no credential store, so the key cannot be remembered here".to_owned(),
                ));
            }
            let passphrase = resolve_passphrase(access.passphrase.clone())?;
            let key_store = KeyStore::open(&keystore, &passphrase)?;
            let archive = store::Database::open(&database, &key_store)?;
            store::device::remember(&store::device::Keychain, &keystore, key_store.dek())?;
            if let Err(error) = store::audit::record_device_trust(
                archive.connection(),
                &store::now_rfc3339(),
                "cli",
                &keystore.display().to_string(),
            ) {
                return Err(CliError::Input(format!(
                    "the key was stored for this device, but recording it in the audit chain failed: {error}"
                )));
            }
            println!("remembered: {}", keystore.display());
            println!("this device now opens the archive with --device, without a passphrase");
            println!("while it does, the archive is protected by your login, not the passphrase");
            Ok(())
        }
        Some(Command::Forget) => {
            let session = access.session(&database, &keystore)?;
            let forgotten = store::device::forget(&store::device::Keychain, &keystore)?;
            if forgotten {
                if let Err(error) = store::audit::record_device_forget(
                    session.database().connection(),
                    &store::now_rfc3339(),
                    "cli",
                    &keystore.display().to_string(),
                ) {
                    return Err(CliError::Input(format!(
                        "the key was forgotten, but recording it in the audit chain failed: {error}"
                    )));
                }
                println!("forgotten:  {}", keystore.display());
                println!("the passphrase is needed again on this device");
            } else {
                println!("nothing to forget: this device held no key for that archive");
            }
            Ok(())
        }
        Some(Command::RecoveryKit { directory }) => {
            let report = store::kit::write(&keystore, &database, &directory, &store::now_rfc3339())?;

            println!("kit:          {}", report.directory.display());
            println!("key:          {}", report.key.display());
            println!("instructions: {}", report.instructions.display());
            println!();
            println!("the backups do not contain this key: keep the kit on another medium,");
            println!("and write a new one after a passphrase or recovery-key change");
            Ok(())
        }
        Some(Command::Evidence { document, out }) => {
            let session = access.session(&database, &keystore)?;
            let owner = owner_of(session.database(), None)?;
            let id = resolve_document(session.database().connection(), owner, &document)?
                .ok_or_else(|| CliError::Input(format!("no document matching {document:?}")))?;

            let report = store::evidence::export(
                session.database().connection(),
                owner,
                id,
                &out,
                "cli",
                &store::now_rfc3339(),
            )?;
            println!("evidence:  {}", out.display());
            println!("document:  {} ({}, {})", report.number, report.direction, report.status);
            println!("files:     {} ({} bytes)", report.files, report.bytes);
            match &report.head {
                Some(hash) => println!("chain:     {} events, head {hash}", report.events),
                None => println!("chain:     empty"),
            }
            if report.ledger_entries > 0 || report.vat_returns > 0 {
                println!(
                    "ledger:    {} entries, {} VAT return(s)",
                    report.ledger_entries, report.vat_returns
                );
            }
            println!("verified:  ok");
            Ok(())
        }
        Some(Command::EvidenceVerify { directory }) => {
            // No keystore and no database: the point of the verifier is that a
            // recipient can run it with nothing but the bundle.
            let report = store::evidence::verify(&directory)?;
            println!("bundle verified: {}", directory.display());
            println!("document:  {} ({}, {})", report.number, report.direction, report.status);
            println!("files:     {} ({} bytes)", report.files, report.bytes);
            match &report.head {
                Some(hash) => println!("chain:     {} events, head {hash}", report.events),
                None => println!("chain:     empty"),
            }
            if report.ledger_entries > 0 || report.vat_returns > 0 {
                println!(
                    "ledger:    {} entries, {} VAT return(s)",
                    report.ledger_entries, report.vat_returns
                );
            }
            Ok(())
        }
        Some(Command::SupportBundle { out }) => {
            let session = access.session(&database, &keystore)?;
            let directory = out.unwrap_or_else(|| store::support::default_directory(&database));

            let summary = store::support::write(
                session.database(),
                &store::support::SupportRequest {
                    database_path: &database,
                    keystore_path: &keystore,
                    app_version: env!("CARGO_PKG_VERSION"),
                    directory: &directory,
                    now: &store::now_rfc3339(),
                },
            )?;
            // The preview is the bundle: what is printed is exactly what is
            // written, so nobody sends a file they have not seen.
            println!("{}", summary.render_text());
            println!("written to {}", directory.display());
            Ok(())
        }
        Some(Command::Backup { to, keep }) => {
            let passphrase = resolve_passphrase(access.passphrase.clone())?;
            let key_store = KeyStore::open(&keystore, &passphrase)?;
            let session = Session::unlock(&database, &keystore, &passphrase)?;
            let directory = to.unwrap_or_else(|| {
                database
                    .parent()
                    .map(|parent| parent.join("backups"))
                    .unwrap_or_else(|| PathBuf::from("backups"))
            });

            let mut chain_progress = progress_reporter("audit chain");
            let report = store::backup::backup_with_progress(
                session.database(),
                &key_store,
                &directory,
                &store::now_rfc3339(),
                keep,
                &mut |stage| match stage {
                    store::backup::BackupStage::Copying => eprintln!("copying the archive…"),
                    store::backup::BackupStage::VerifyingStructure => eprintln!("verifying the copy…"),
                    store::backup::BackupStage::VerifyingChain { done, total } => chain_progress(done, total),
                },
            )?;
            println!("backup: {} ({} bytes, verified)", report.path.display(), report.bytes);
            for removed in &report.removed {
                println!("pruned: {}", removed.display());
            }
            Ok(())
        }
        Some(Command::VerifyBackup { path }) => {
            let passphrase = resolve_passphrase(access.passphrase.clone())?;
            let key_store = KeyStore::open(&keystore, &passphrase)?;
            let summary = store::backup::inspect(&path, key_store.dek())?;
            println!("backup verified: {}", path.display());
            println!("schema:          {}", summary.schema_version);
            println!("audit:           {} events verified", summary.events);
            match &summary.head {
                Some(head) => println!("head:            {} {} ({})", head.seq, head.hash, head.at),
                None => println!("head:            empty chain"),
            }
            Ok(())
        }
        Some(Command::PassphraseChange { new_passphrase }) => {
            let passphrase = resolve_passphrase(access.passphrase.clone())?;
            let new_passphrase = resolve_new_passphrase(new_passphrase)?;
            warn_if_weak(&new_passphrase);

            // Unlock first: if the database or the current passphrase is wrong,
            // nothing is rewritten.
            let mut key_store = KeyStore::open(&keystore, &passphrase)?;
            let session = Session::unlock(&database, &keystore, &passphrase)?;

            key_store.change_passphrase(&new_passphrase)?;

            if let Err(error) = store::audit::record_passphrase_change(
                session.database().connection(),
                &store::now_rfc3339(),
                "cli",
                &key_store.path().display().to_string(),
            ) {
                return Err(CliError::Input(format!(
                    "the passphrase was changed, but recording it in the audit chain failed: {error}"
                )));
            }

            println!("passphrase changed: {}", key_store.path().display());
            println!("backups made before this change still need the old passphrase");
            Ok(())
        }
        Some(Command::PassphraseReset {
            recovery_key,
            new_passphrase,
        }) => {
            let recovery_key = resolve_recovery_key(recovery_key)?;
            let new_passphrase = resolve_new_passphrase(new_passphrase)?;
            warn_if_weak(&new_passphrase);

            let mut key_store =
                KeyStore::open_with_recovery(&keystore, &recovery_key).map_err(|error| match error {
                    StoreError::WrongPassphrase => {
                        CliError::Input("that recovery key does not open this archive".to_owned())
                    }
                    other => CliError::from(other),
                })?;
            let archive = store::Database::open(&database, &key_store)?;

            key_store.change_passphrase(&new_passphrase)?;
            // The key that was just typed may now sit in a shell history or a
            // screenshot. Rotating it leaves exactly one working route plus a
            // fresh key the user can put somewhere safe.
            let new_key = key_store.enable_recovery(&new_passphrase)?;

            if let Err(error) = store::audit::record_passphrase_reset(
                archive.connection(),
                &store::now_rfc3339(),
                "cli",
                &key_store.path().display().to_string(),
                true,
            ) {
                return Err(CliError::Input(format!(
                    "the passphrase was reset, but recording it in the audit chain failed: {error}"
                )));
            }

            println!("passphrase reset: {}", key_store.path().display());
            println!();
            println!("new recovery key: {new_key}");
            println!();
            println!("The recovery key used for this reset no longer works. Write the new one");
            println!("down and keep it apart from the database and the passphrase.");
            Ok(())
        }
        Some(Command::RecoveryAdd) => {
            let passphrase = resolve_passphrase(access.passphrase.clone())?;
            let mut key_store = KeyStore::open(&keystore, &passphrase)?;
            let session = Session::unlock(&database, &keystore, &passphrase)?;

            let key = key_store.enable_recovery(&passphrase)?;
            if let Err(error) = store::audit::record_recovery_key(
                session.database().connection(),
                &store::now_rfc3339(),
                "cli",
                &key_store.path().display().to_string(),
            ) {
                return Err(CliError::Input(format!(
                    "the recovery key was written, but recording it in the audit chain failed: {error}"
                )));
            }

            println!("recovery key: {key}");
            println!();
            println!("Write it down and keep it apart from the database and the passphrase.");
            println!("It is shown once and cannot be looked up later; it opens the archive if");
            println!("the passphrase is lost. The passphrase keeps working unchanged.");
            Ok(())
        }
        Some(Command::Init) => {
            let passphrase = resolve_creation_passphrase(access.passphrase.clone())?;
            warn_if_weak(&passphrase);
            let session = Session::create(&database, &keystore, &passphrase)?;
            let summary = session.summary()?;
            println!("database: {}", database.display());
            println!("keystore: {}", keystore.display());
            println!("schema:   {}", summary.schema_version);
            println!("encryption: {}", store::encryption_label());
            Ok(())
        }
        Some(Command::DemoSeed {
            customers,
            invoices,
            received,
            seed,
            force,
        }) => seed_demo(
            &database,
            &keystore,
            access.passphrase.clone(),
            demo::Options {
                customers,
                invoices,
                received,
                seed,
                today: None,
            },
            force,
        ),
        Some(Command::Verify { anchor }) => {
            let session = access.session(&database, &keystore)?;
            eprintln!("checking the file structure…");
            session.integrity_check()?;
            let summary = session.summary()?;
            let mut chain_progress = progress_reporter("audit chain");
            let audit_events =
                store::audit::verify_chain_with_progress(session.database().connection(), &mut chain_progress)?;
            let mut payload_progress = progress_reporter("payloads");
            let mut reference_progress = progress_reporter("references");
            let report =
                store::integrity::audit_with_progress(session.database().connection(), &mut |phase, done, total| {
                    match phase {
                        store::integrity::AuditPhase::Payloads => payload_progress(done, total),
                        store::integrity::AuditPhase::References => reference_progress(done, total),
                    }
                })?;

            println!("ok:       integrity check passed");
            println!("schema:   {}", summary.schema_version);
            println!("encryption: {}", store::encryption_label());
            println!("sellers:  {}", summary.sellers);
            println!("drafts:   {}", summary.drafts);
            println!("audit:    {audit_events} events verified");
            match store::audit::head(session.database().connection())? {
                Some(head) => println!("head:     {} {} ({})", head.seq, head.hash, head.at),
                None => println!("head:     empty chain"),
            }
            println!("artefacts: {} hashes verified", report.artefacts);
            println!(
                "blobs:    {} payloads verified ({} imports)",
                report.blobs, report.imports
            );
            if report.orphaned_blobs > 0 {
                println!(
                    "orphans:  {} unreferenced blob(s); the archive is intact, this is wasted space",
                    report.orphaned_blobs
                );
            }
            // Last, because an anchor can only speak about a chain that verified.
            if let Some(path) = anchor {
                let checked = store::anchor::check(&path, session.database().connection())?;
                println!(
                    "anchor:   {} checkpoint(s) still in the chain, newest {} {}",
                    checked.count, checked.newest.seq, checked.newest.hash
                );
            }
            Ok(())
        }
        Some(Command::Anchor { file }) => {
            let session = access.session(&database, &keystore)?;
            let now = store::now_rfc3339();
            let report = store::anchor::record(&file, session.database().connection(), &now)?;

            println!("anchor file: {}", file.display());
            println!("checked:     {} previous checkpoint(s)", report.checked);
            println!(
                "head:        {} {} ({})",
                report.anchor.seq, report.anchor.hash, report.anchor.at
            );
            if report.appended {
                println!("appended:    yes");
            } else {
                println!("appended:    no, the chain has not moved since the last checkpoint");
            }
            Ok(())
        }
        Some(Command::MigrateDown { to, yes }) => {
            if !yes {
                return Err(CliError::Input(
                    "refusing to roll back migrations without --yes".to_owned(),
                ));
            }
            let mut session = access.session(&database, &keystore)?;
            let rolled_back = store::migrations::rollback_to(session.database_mut().connection_mut(), to)?;
            println!("rolled back: {rolled_back} migration(s)");
            println!("schema:   {}", session.summary()?.schema_version);
            Ok(())
        }
        Some(Command::SellerAdd {
            name,
            city,
            country,
            vat,
            kvk,
            peppol,
            street,
            postal,
            email,
            contact,
            currency,
            locale,
        }) => {
            let session = access.session(&database, &keystore)?;
            let party = party(name, city, country, street, postal, vat, kvk, peppol, email, contact)?;
            let id = SellerId::new();
            insert_seller(
                session.database().connection(),
                &SellerRecord {
                    id,
                    party,
                    default_currency: Currency::parse(&currency)?,
                    default_locale: locale,
                },
                &store::now_rfc3339(),
            )?;
            println!("seller: {id}");
            Ok(())
        }
        Some(Command::CustomerAdd {
            name,
            city,
            country,
            vat,
            kvk,
            peppol,
            street,
            postal,
            email,
            contact,
        }) => {
            let session = access.session(&database, &keystore)?;
            let id = CustomerId::new();
            insert_customer(
                session.database().connection(),
                &CustomerRecord {
                    id,
                    party: party(name, city, country, street, postal, vat, kvk, peppol, email, contact)?,
                },
                &store::now_rfc3339(),
            )?;
            println!("customer: {id}");
            Ok(())
        }
        Some(Command::DraftNew {
            seller,
            customer,
            date,
            due,
            delivery,
            lines,
            note,
            buyer_ref,
            order_ref,
            template,
        }) => {
            let session = access.session(&database, &keystore)?;
            let seller_id = SellerId::parse_str(&seller)?;
            let customer_id = CustomerId::parse_str(&customer)?;
            let seller_record = load_seller(session.database().connection(), seller_id)?;
            let customer_record = load_customer(session.database().connection(), customer_id)?;

            // A name that matches no file is refused here rather than drawn with
            // something else: the point of naming a layout is that it is used.
            if let Some(name) = template.as_deref() {
                let (templates, _) = app::load_templates(session.database_path());
                if !templates.iter().any(|template| template.name == name) {
                    return Err(CliError::Input(format!(
                        "no template named {name}; the archive holds {}",
                        if templates.is_empty() {
                            "none".to_owned()
                        } else {
                            templates
                                .iter()
                                .map(|template| template.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        }
                    )));
                }
            }

            let issue_date = match date {
                Some(value) => LocalDate::parse_iso(&value)?,
                None => LocalDate::today_utc(),
            };
            let mut draft = InvoiceDraft::new(seller_id, customer_id, seller_record.default_currency, issue_date);
            draft.due_date = due.as_deref().map(LocalDate::parse_iso).transpose()?;
            draft.delivery_date = delivery.as_deref().map(LocalDate::parse_iso).transpose()?;
            draft.note = note;
            draft.buyer_reference = buyer_ref;
            draft.order_reference = order_ref;
            draft.template = template;
            for (index, spec) in lines.iter().enumerate() {
                let position = u32::try_from(index + 1).map_err(|_| CliError::Input("too many lines".to_owned()))?;
                draft.lines.push(parse_line(spec, draft.currency, position)?);
            }

            let findings = validate_draft(&draft, &seller_record.party, &customer_record.party);
            print_findings(&findings);
            if has_blocking(&findings) {
                return Err(CliError::Validation);
            }

            let totals = calculate_totals(&draft.lines)?;
            println!("draft: {}", draft.id);
            println!("net:   {}", totals.net.to_fixed_string(2)?);
            println!("tax:   {}", totals.tax.to_fixed_string(2)?);
            println!("gross: {}", totals.gross.to_fixed_string(2)?);

            insert_draft(session.database().connection(), &draft, &store::now_rfc3339())?;
            Ok(())
        }
        Some(Command::CreditNote {
            invoice,
            seller,
            date,
            note,
            dry_run,
        }) => {
            let session = access.session(&database, &keystore)?;
            let connection = session.database().connection();
            let seller_id = match seller {
                Some(value) => SellerId::parse_str(&value)?,
                None => {
                    first_seller(connection)?
                        .ok_or_else(|| CliError::Input("no seller in this database".to_owned()))?
                        .id
                }
            };
            let issue_date = match date {
                Some(value) => LocalDate::parse_iso(&value)?,
                None => LocalDate::today_utc(),
            };

            let mut draft = credit_note_for(connection, seller_id, &invoice, issue_date)?
                .ok_or_else(|| CliError::Input(format!("no issued document numbered {invoice}")))?;
            if let Some(note) = note {
                draft.note = Some(note);
            }

            let seller_record = load_seller(connection, draft.seller_id)?;
            let customer_record = load_customer(connection, draft.customer_id)?;
            let findings = validate_draft(&draft, &seller_record.party, &customer_record.party);
            print_findings(&findings);
            if has_blocking(&findings) {
                return Err(CliError::Validation);
            }

            let totals = calculate_totals(&draft.lines)?;
            println!("credit note: {}", draft.id);
            println!("corrects:    {invoice}");
            println!("net:         {}", totals.net.to_fixed_string(2)?);
            println!("tax:         {}", totals.tax.to_fixed_string(2)?);
            println!("gross:       {}", totals.gross.to_fixed_string(2)?);

            if !dry_run {
                insert_draft(connection, &draft, &store::now_rfc3339())?;
            }
            Ok(())
        }
        Some(Command::Watch {
            directory,
            every,
            quiet_for,
            once,
        }) => {
            let session = access.session(&database, &keystore)?;
            let owner = first_seller(session.database().connection())?
                .ok_or_else(|| CliError::Input("no seller in this database".to_owned()))?
                .id;
            let quiet = std::time::Duration::from_secs(quiet_for);

            if once {
                let files = import::ready_now(&directory, std::time::SystemTime::now(), quiet)?;
                let failed = import_watched(&session, owner, &files)?;
                if failed > 0 {
                    return Err(CliError::Input(format!(
                        "{failed} of {} file(s) could not be imported",
                        files.len()
                    )));
                }
                return Ok(());
            }

            println!("watching: {}", directory.display());
            println!("polling:  every {every}s, reading files untouched for {quiet_for}s");
            println!("note:     files already there are left alone; `bonk import` takes those");

            let mut watcher = import::Watcher::new(quiet);
            loop {
                let files = watcher.poll(&directory, std::time::SystemTime::now())?;
                // A bad file must not stop the watcher: the folder keeps its
                // purpose even when one drop is not an invoice.
                import_watched(&session, owner, &files)?;
                std::thread::sleep(std::time::Duration::from_secs(every.max(1)));
            }
        }
        Some(Command::Import { files, seller }) => {
            let session = access.session(&database, &keystore)?;
            let connection = session.database().connection();
            let owner = match seller {
                Some(value) => SellerId::parse_str(&value)?,
                None => {
                    first_seller(connection)?
                        .ok_or_else(|| CliError::Input("no seller in this database".to_owned()))?
                        .id
                }
            };
            let now = store::now_rfc3339();
            let mut failed = 0_usize;
            for file in &files {
                match import::import_file(session.database(), owner, file, &now) {
                    Ok(outcome) if outcome.duplicate => {
                        println!("duplicate: {}", outcome.number);
                        println!("  document: {}", outcome.document_id);
                        println!("  file:     {}", outcome.path);
                    }
                    Ok(outcome) => {
                        let supplier = outcome
                            .supplier
                            .map(|id| id.to_string())
                            .unwrap_or_else(|| "unmatched".to_owned());
                        println!("imported: {}", outcome.number);
                        println!("  document: {}", outcome.document_id);
                        println!("  supplier: {supplier}");
                        println!("  file:     {}", outcome.path);
                    }
                    Err(error) => {
                        eprintln!("error: {}: {error}", file.display());
                        failed += 1;
                    }
                }
            }
            if failed > 0 {
                return Err(CliError::Input(format!(
                    "{failed} of {} file(s) could not be imported",
                    files.len()
                )));
            }
            Ok(())
        }
        Some(Command::Received { status }) => {
            let session = access.session(&database, &keystore)?;
            let connection = session.database().connection();
            let owner = owner_of(session.database(), None)?;
            let filter = match status {
                Some(value) => Some(DocumentStatus::parse(&value).ok_or_else(|| {
                    CliError::Input(format!(
                        "unknown status {value:?}; use draft, issued, received, approved, paid, or archived"
                    ))
                })?),
                None => None,
            };

            let mut shown = 0_usize;
            for row in list_received(connection, owner)? {
                if filter.is_some_and(|filter| filter != row.status) {
                    continue;
                }
                println!(
                    "{:<14} {:<28} {}  {:>12}  {:<9} {}",
                    row.number,
                    row.supplier,
                    row.issue_date.to_iso_string(),
                    row.gross.to_fixed_string(2)?,
                    row.status,
                    row.due_date
                        .map(|due| format!("due {}", due.to_iso_string()))
                        .unwrap_or_default()
                );
                if !row.matched {
                    println!("               supplier not matched; the document is kept as received");
                }
                shown += 1;
            }
            if shown == 0 {
                println!("no received documents");
            }
            Ok(())
        }
        Some(Command::Search {
            query,
            direction,
            from,
            to,
            status,
        }) => {
            let session = access.session(&database, &keystore)?;
            let connection = session.database().connection();
            let owner = owner_of(session.database(), None)?;
            let filters = filters(direction, from, to, status)?;

            let hits = store::search(
                connection,
                owner,
                &SearchRequest {
                    query: Some(&query),
                    direction: filters.direction,
                    from: filters.from,
                    to: filters.to,
                    status: filters.status,
                },
            )?;
            for hit in &hits {
                println!(
                    "{:<14} {:<8} {:<28} {}  {:>12}  {}",
                    hit.number,
                    hit.direction,
                    hit.party,
                    hit.issue_date.to_iso_string(),
                    hit.gross.to_fixed_string(2)?,
                    hit.status
                );
            }
            if hits.is_empty() {
                println!("no documents match {query:?}");
            }
            Ok(())
        }
        Some(Command::BenchSearch { query, repeat }) => {
            if repeat == 0 {
                return Err(CliError::Input("--repeat must be at least 1".to_owned()));
            }
            let session = access.session(&database, &keystore)?;
            let connection = session.database().connection();
            let owner = owner_of(session.database(), None)?;

            // An empty query is the browse view, which is what the panel shows
            // before anything is typed.
            let words = query.trim();
            let query = (!words.is_empty()).then_some(words);
            // One page, the size the find panel draws: the criterion is about
            // what the window does, not about the largest query the archive can
            // answer.
            const PAGE: usize = 50;
            let mut samples = Vec::with_capacity(repeat);
            let mut rows = 0;
            for _ in 0..repeat {
                let started = std::time::Instant::now();
                let page = store::search_page(
                    connection,
                    owner,
                    &SearchRequest {
                        query,
                        direction: None,
                        from: None,
                        to: None,
                        status: None,
                    },
                    0,
                    PAGE,
                )?;
                samples.push(started.elapsed().as_micros() as u64);
                rows = page.hits.len();
            }

            samples.sort_unstable();
            // Nearest-rank percentile: ceil(0.95 n) - 1, without floating point.
            let rank = (samples.len() * 95).div_ceil(100).clamp(1, samples.len()) - 1;
            println!(
                "query {query:?}: {rows} hit(s), {count} samples, median {median} \u{00b5}s, \
                 p95 {p95} \u{00b5}s, max {max} \u{00b5}s",
                count = samples.len(),
                median = samples[samples.len() / 2],
                p95 = samples[rank],
                max = samples[samples.len() - 1],
            );
            Ok(())
        }
        Some(Command::Export {
            query,
            format,
            direction,
            from,
            to,
            status,
            out,
        }) => {
            let session = access.session(&database, &keystore)?;
            let connection = session.database().connection();
            let owner = owner_of(session.database(), None)?;
            let filters = filters(direction, from, to, status)?;
            let query = query.trim();

            let hits = store::search(
                connection,
                owner,
                &SearchRequest {
                    query: (!query.is_empty()).then_some(query),
                    direction: filters.direction,
                    from: filters.from,
                    to: filters.to,
                    status: filters.status,
                },
            )?;
            let rendered = match format.as_str() {
                "json" => app::export::to_json(&hits)?,
                _ => app::export::to_csv(&hits)?,
            };
            match out {
                Some(path) => {
                    std::fs::write(&path, &rendered)?;
                    println!("exported: {} documents -> {}", hits.len(), path.display());
                }
                None => print!("{rendered}"),
            }
            Ok(())
        }
        Some(Command::Match { document, customer }) => {
            let session = access.session(&database, &keystore)?;
            let connection = session.database().connection();
            let owner = owner_of(session.database(), None)?;
            let id = resolve_received(connection, owner, &document)?
                .ok_or_else(|| CliError::Input(format!("no received document matching {document:?}")))?;
            let customer_id = resolve_customer(connection, &customer)?
                .ok_or_else(|| CliError::Input(format!("no customer matching {customer:?}")))?;
            set_supplier_match(connection, id, customer_id, "cli", &store::now_rfc3339())?;
            println!("matched: {document} -> {customer}");
            Ok(())
        }
        Some(Command::Approve { document }) => {
            settle_received(&access, &database, &keystore, &document, DocumentStatus::Approved)
        }
        Some(Command::MarkPaid { document }) => {
            settle_received(&access, &database, &keystore, &document, DocumentStatus::Paid)
        }
        Some(Command::Issue {
            draft,
            out,
            actor,
            device_name,
            language,
        }) => {
            let mut session = access.session(&database, &keystore)?;
            let draft_id = DocumentId::parse_str(&draft)?;

            let loaded = load_draft(session.database().connection(), draft_id)?
                .ok_or_else(|| CliError::Input(format!("draft {draft_id} does not exist")))?;
            let seller = load_seller(session.database().connection(), loaded.seller_id)?;
            let customer = load_customer(session.database().connection(), loaded.customer_id)?;
            let language = match language.as_deref() {
                Some("nl") => app::DocumentLanguage::Dutch,
                Some("en") => app::DocumentLanguage::English,
                _ => app::DocumentLanguage::from_locale(&seller.default_locale),
            };

            let findings = validate_draft(&loaded, &seller.party, &customer.party);
            print_findings(&findings);
            if has_blocking(&findings) {
                return Err(CliError::Validation);
            }

            let mut rendered: Option<Vec<u8>> = None;
            let issued = match issue_draft(
                session.database_mut().connection_mut(),
                IssueRequest {
                    draft_id,
                    actor: &actor,
                    device: &device_name,
                    renderer_version: "syntax-0.1",
                    now: &store::now_rfc3339(),
                },
                |view| {
                    let bytes = app::render_issue(view)?;
                    rendered = Some(bytes.clone());
                    Ok(bytes)
                },
            ) {
                Ok(issued) => issued,
                Err(StoreError::Conformance(findings)) => {
                    print_findings(&findings);
                    return Err(CliError::Validation);
                }
                Err(error) => return Err(error.into()),
            };

            std::fs::create_dir_all(&out)?;
            let path = out.join(format!("{}.xml", issued.number));
            let bytes = rendered.unwrap_or_default();
            std::fs::write(&path, &bytes)?;

            println!("issued: {}", issued.number);
            println!("file:   {}", path.display());
            println!("sha256: {}", issued.ubl_sha256);
            println!("audit:  {}", issued.audit_head);

            // The draft names the layout it was written for. A name the archive no
            // longer holds falls through to the default, and the artefact records
            // the digest of whichever layout really drew it.
            let (templates, problems) = app::load_templates(session.database_path());
            for problem in &problems {
                eprintln!("template ignored: {problem}");
            }
            let template = match loaded.template.as_deref() {
                Some(name) => {
                    let named = templates.iter().find(|template| template.name == name);
                    if named.is_none() {
                        // Issuing is the legal act; a layout file that moved must
                        // not block it. The artefact records the digest of the
                        // layout that really drew the PDF.
                        eprintln!("warning: template {name} is not in this archive; the default draws instead");
                    }
                    named.or_else(|| app::default_template(&templates))
                }
                None => app::default_template(&templates),
            };

            match app::invoice_pdf(
                &issued.number,
                &loaded,
                &seller,
                &customer,
                &issued.totals,
                language,
                template.as_ref().map(|template| &template.template),
            ) {
                Ok(pdf) => {
                    let pdf_path = out.join(format!("{}.pdf", issued.number));
                    std::fs::write(&pdf_path, &pdf)?;
                    let now = store::now_rfc3339();
                    match store::attach_artefact(
                        session.database().connection(),
                        issued.id,
                        "pdf",
                        &pdf,
                        render::RENDERER_VERSION,
                        template.map(|template| template.sha256.as_str()),
                        &now,
                    ) {
                        Ok(hash) => {
                            println!("pdf:    {} ({hash})", pdf_path.display());
                            if let Some(template) = template {
                                println!("layout: {} ({})", template.template.name, template.sha256);
                            }
                        }
                        Err(error) => eprintln!(
                            "warning: PDF written to {} but not attached: {error}",
                            pdf_path.display()
                        ),
                    }
                }
                Err(error) => {
                    eprintln!("warning: PDF not rendered: {error}");
                }
            }
            Ok(())
        }
        Some(Command::Ledger { command }) => ledger(&access, &database, &keystore, command),
    }
}

/// Runs one ledger subcommand against the archive.
///
/// Every command opens the archive, resolves the only seller these books hold,
/// and then reads or writes the ledger. Nothing here formats money itself beyond
/// two decimals: the arithmetic is in `store`, the rules are in `store::posting`.
fn ledger(access: &Access, database: &Path, keystore: &Path, command: LedgerCommand) -> Result<(), CliError> {
    let session = access.session(database, keystore)?;
    let connection = session.database().connection();
    let owner = owner_of(session.database(), None)?;
    let now = store::now_rfc3339();

    match command {
        LedgerCommand::Setup => {
            let added = store::seed_starter_chart(connection, &now)?;
            if added == 0 {
                println!("the chart of accounts is already in place");
            } else {
                println!("accounts added: {added}");
            }
            println!("accounts:       {}", store::list_accounts(connection)?.len());
            Ok(())
        }
        LedgerCommand::Chart { all } => {
            let accounts = store::list_accounts(connection)?;
            if accounts.is_empty() {
                println!("no chart of accounts yet; run `bonk ledger setup`");
                return Ok(());
            }
            for account in accounts.iter().filter(|account| all || !account.archived) {
                println!(
                    "{:<6} {:<34} {:<10} {}{}",
                    account.code,
                    account.name,
                    account.kind.as_str(),
                    if account.is_control { "control" } else { "" },
                    if account.archived { " archived" } else { "" }
                );
            }
            Ok(())
        }
        LedgerCommand::Post { documents, actor } => {
            if store::list_accounts(connection)?.is_empty() {
                return Err(CliError::Input(
                    "no chart of accounts yet; run `bonk ledger setup` first".to_owned(),
                ));
            }

            let targets = if documents.is_empty() {
                store::pending(connection, owner)?
            } else {
                let mut resolved = Vec::with_capacity(documents.len());
                for number in &documents {
                    let reference = store::find_issued_reference(connection, owner, number)?
                        .ok_or_else(|| CliError::Input(format!("no issued document {number:?}")))?;
                    resolved.push(store::PendingPosting {
                        id: reference.id,
                        number: reference.number,
                        issue_date: reference.issue_date,
                        direction: "outbound".to_owned(),
                    });
                }
                resolved
            };

            if targets.is_empty() {
                println!("nothing to post");
                return Ok(());
            }
            for target in &targets {
                let posted = store::post_document(connection, target.id, &actor, &now)?;
                if posted.created {
                    println!("posted {} {} {}", target.number, posted.journal.code(), posted.number);
                } else {
                    println!(
                        "already posted {} as {} {}",
                        target.number,
                        posted.journal.code(),
                        posted.number
                    );
                }
            }
            Ok(())
        }
        LedgerCommand::TrialBalance { from, to } => {
            let (from, to) = window(from, to)?;
            let balance = store::trial_balance(connection, owner, from, to)?;
            println!("proefbalans {from} to {to} ({})", balance.currency);
            println!("{:<6} {:<34} {:>14} {:>14}", "code", "name", "debit", "credit");
            for row in &balance.rows {
                println!(
                    "{:<6} {:<34} {:>14} {:>14}",
                    row.code,
                    row.name,
                    column_text(row.debit_micros, balance.currency)?,
                    column_text(row.credit_micros, balance.currency)?
                );
            }
            println!(
                "{:<6} {:<34} {:>14} {:>14}",
                "",
                "total",
                amount_text(balance.debit_total_micros, balance.currency)?,
                amount_text(balance.credit_total_micros, balance.currency)?
            );
            if balance.balances() {
                println!("balanced");
            } else {
                println!(
                    "DOES NOT BALANCE: off by {}",
                    amount_text(balance.difference_micros(), balance.currency)?
                );
            }
            Ok(())
        }
        LedgerCommand::ProfitAndLoss { from, to } => {
            let (from, to) = window(from, to)?;
            let report = store::profit_and_loss(connection, owner, from, to)?;
            println!("winst- en verliesrekening {from} to {to} ({})", report.currency);
            println!("omzet");
            for row in &report.revenue {
                println!(
                    "  {:<6} {:<34} {:>14}",
                    row.code,
                    row.name,
                    amount_text(row.balance_micros(), report.currency)?
                );
            }
            println!(
                "  {:<41} {:>14}",
                "omzet totaal",
                amount_text(report.revenue_micros, report.currency)?
            );
            println!("kosten");
            for row in &report.expenses {
                println!(
                    "  {:<6} {:<34} {:>14}",
                    row.code,
                    row.name,
                    amount_text(row.balance_micros(), report.currency)?
                );
            }
            println!(
                "  {:<41} {:>14}",
                "kosten totaal",
                amount_text(report.expense_micros, report.currency)?
            );
            println!(
                "{:<41} {:>14}",
                "resultaat",
                amount_text(report.result_micros(), report.currency)?
            );
            Ok(())
        }
        LedgerCommand::BalanceSheet { as_of } => {
            let as_of = parse_date_or(as_of, LocalDate::today_utc())?;
            let sheet = store::balance_sheet(connection, owner, as_of)?;
            println!("balans {as_of} ({})", sheet.currency);
            println!("activa");
            for row in &sheet.assets {
                println!(
                    "  {:<6} {:<34} {:>14}",
                    row.code,
                    row.name,
                    amount_text(row.balance_micros(), sheet.currency)?
                );
            }
            println!(
                "  {:<41} {:>14}",
                "totaal activa",
                amount_text(sheet.assets_micros, sheet.currency)?
            );
            println!("schulden");
            for row in &sheet.liabilities {
                println!(
                    "  {:<6} {:<34} {:>14}",
                    row.code,
                    row.name,
                    amount_text(row.balance_micros(), sheet.currency)?
                );
            }
            println!(
                "  {:<41} {:>14}",
                "totaal schulden",
                amount_text(sheet.liabilities_micros, sheet.currency)?
            );
            println!("eigen vermogen");
            for row in &sheet.equity {
                println!(
                    "  {:<6} {:<34} {:>14}",
                    row.code,
                    row.name,
                    amount_text(row.balance_micros(), sheet.currency)?
                );
            }
            println!(
                "  {:<41} {:>14}",
                "resultaat boekjaar",
                amount_text(sheet.result_micros, sheet.currency)?
            );
            println!(
                "  {:<41} {:>14}",
                "totaal vermogen",
                amount_text(sheet.liabilities_and_equity_micros(), sheet.currency)?
            );
            if sheet.balances() {
                println!("balanced: activa equals schulden plus vermogen");
            } else {
                println!(
                    "DOES NOT BALANCE: off by {}",
                    amount_text(sheet.difference_micros(), sheet.currency)?
                );
            }
            Ok(())
        }
        LedgerCommand::VatReturn {
            year,
            month,
            prepare,
            actor,
        } => {
            let (year, month) = match (year, month) {
                (Some(year), Some(month)) => (year, month),
                (year, None) => {
                    let today = LocalDate::today_utc();
                    let (default_year, default_month) = if u8::from(today.as_date().month()) == 1 {
                        (today.as_date().year() - 1, 12)
                    } else {
                        (today.as_date().year(), u8::from(today.as_date().month()) - 1)
                    };
                    (year.unwrap_or(default_year), default_month)
                }
                (None, Some(month)) => (LocalDate::today_utc().as_date().year(), month),
            };
            if !(1..=12).contains(&month) {
                return Err(CliError::Input(format!("month {month} is not 1-12")));
            }
            let (starts, ends) = store::vat::period_bounds(year, month)?;

            let report = if prepare {
                let prepared = store::vat::prepare(connection, owner, year, month, &actor, &now)?;
                if let Some(supersedes) = &prepared.supersedes {
                    println!("superseded: {}", supersedes);
                }
                println!("prepared:   {} ({})", prepared.id, prepared.prepared_at);
                println!(
                    "basis:      {} entries, {}",
                    prepared.computed.basis.len(),
                    prepared.basis_hash
                );
                prepared.computed
            } else {
                store::vat::compute(connection, owner, year, month)?
            };

            println!(
                "btw-aangifte {year}-{month:02} ({starts} to {ends}) {}",
                report.currency
            );
            println!("{:<4} {:<52} {:>13} {:>13}", "", "", "omzet", "btw");
            for line in report
                .lines
                .iter()
                .filter(|line| !line.rubriek.is_total() && line.rubriek != store::vat::Rubriek::VatDeductible)
            {
                println!(
                    "{:<4} {:<52} {:>13} {:>13}",
                    line.rubriek.code(),
                    line.rubriek.label(),
                    column_text(line.base_micros, report.currency)?,
                    column_text(line.vat_micros, report.currency)?
                );
            }
            println!(
                "{:<4} {:<52} {:>13} {:>13}",
                "5a",
                store::vat::Rubriek::VatDue.label(),
                "",
                amount_text(report.vat_due_micros(), report.currency)?
            );
            println!(
                "{:<4} {:<52} {:>13} {:>13}",
                "5b",
                store::vat::Rubriek::VatDeductible.label(),
                "",
                amount_text(report.vat_deductible_micros(), report.currency)?
            );
            println!(
                "{:<4} {:<52} {:>13} {:>13}",
                "5c",
                store::vat::Rubriek::VatPayable.label(),
                "",
                amount_text(report.payable_micros(), report.currency)?
            );
            let ready = store::readiness(connection, owner, year, month)?;
            println!();
            if ready.is_ready() {
                println!("fileable:    yes");
            } else {
                println!("fileable:    no");
                for blocker in ready.blockers() {
                    println!("  {blocker}");
                }
            }
            for reason in ready.unchecked() {
                println!("not checked: {reason}");
            }
            println!("mapping      {}", store::vat::MAPPING_VERSION);
            Ok(())
        }
        LedgerCommand::Bank { command } => bank(session.database(), owner, command, &now),
        LedgerCommand::RebuildOpenItems { actor } => {
            let report = store::rebuild_open_items(connection, owner, &actor, &now)?;
            if report.added == 0 && report.present == 0 {
                println!("nothing to rebuild: no document has been posted");
                return Ok(());
            }
            println!("open items: {} added, {} already present", report.added, report.present);
            if report.is_complete() {
                println!("every posted document has a subledger item");
            } else {
                println!(
                    "{} posted document(s) cannot appear in the subledger, having no counterparty:",
                    report.unmatched.len()
                );
                for number in &report.unmatched {
                    println!("  {number}");
                }
            }
            Ok(())
        }
        LedgerCommand::TaxCodes => {
            let codes = store::list_tax_codes(connection)?;
            if codes.is_empty() {
                println!("no tax codes");
                return Ok(());
            }
            println!("{:<16} {:<34} {:<4} {:<4} mapping", "code", "name", "cat", "box");
            for code in codes {
                println!(
                    "{:<16} {:<34} {:<4} {:<4} {}",
                    code.code.as_str(),
                    code.name,
                    code.category.code(),
                    code.rubriek.code(),
                    code.mapping_version
                );
            }
            Ok(())
        }
        LedgerCommand::TaxCode {
            document,
            code,
            clear,
            actor,
        } => {
            let id = resolve_document(connection, owner, &document)?
                .ok_or_else(|| CliError::Input(format!("no document matching {document:?}")))?;
            if clear {
                store::clear_tax_code(connection, id, &actor, &now)?;
                println!("cleared:   {document}");
                println!("the document falls back to its own category");
                return Ok(());
            }
            let Some(code) = code else {
                println!("document:  {document}");
                match store::assigned_tax_code(connection, id)? {
                    Some(assigned) => {
                        println!("tax code:  {} ({})", assigned.code, assigned.name);
                        println!("box:       {} {}", assigned.rubriek.code(), assigned.rubriek.label());
                    }
                    None => println!("tax code:  none; its category alone decides the box"),
                }
                return Ok(());
            };
            let assigned = store::assign_tax_code(connection, id, &code, &actor, &now)?;
            println!("assigned:  {} ({})", assigned.code, assigned.name);
            println!("box:       {} {}", assigned.rubriek.code(), assigned.rubriek.label());
            println!("the next `bonk ledger post` puts the VAT there");
            Ok(())
        }
        LedgerCommand::PostManual {
            date,
            description,
            line,
            actor,
        } => {
            if store::list_accounts(connection)?.is_empty() {
                return Err(CliError::Input(
                    "no chart of accounts yet; run `bonk ledger setup` first".to_owned(),
                ));
            }
            let entry_date = LocalDate::parse_iso(&date)
                .map_err(|_| CliError::Input(format!("{date:?} is not a date in YYYY-MM-DD form")))?;
            let currency = store::reporting_currency(connection, owner)?;
            let mut lines = Vec::with_capacity(line.len());
            for raw in &line {
                lines.push(parse_manual_line(raw, currency)?);
            }
            let count = lines.len();
            let posted = store::post_manual_entry(connection, owner, entry_date, &description, lines, &actor, &now)?;
            println!("posted {} {} ({count} lines)", posted.journal.code(), posted.number);
            Ok(())
        }
        LedgerCommand::Reclassify {
            entry,
            code,
            reason,
            date,
            actor,
        } => {
            let id = resolve_entry(session.database(), &entry)?;
            let date = parse_date_or(date, LocalDate::today_utc())?;
            let corrected = store::reclassify(connection, id, &code, &reason, date, &actor, &now)?;
            println!(
                "reversed {} as {} {}",
                corrected.original,
                corrected.reversal.journal.code(),
                corrected.reversal.number
            );
            println!(
                "refiled  {} {} under {code}",
                corrected.corrected.journal.code(),
                corrected.corrected.number
            );
            println!("dated {date}");
            Ok(())
        }
        LedgerCommand::OpenItems { kind, aging, as_of } => {
            let as_of = parse_date_or(as_of, LocalDate::today_utc())?;
            let kind = match kind.as_deref() {
                Some("receivable") => store::OpenItemKind::Receivable,
                Some("payable") => store::OpenItemKind::Payable,
                Some(other) => {
                    return Err(CliError::Input(format!(
                        "unknown kind {other:?}; use receivable or payable"
                    )));
                }
                None => store::OpenItemKind::Receivable,
            };
            let currency = store::reporting_currency(connection, owner)?;

            if aging {
                let report = store::aging(connection, owner, kind, as_of)?;
                println!("{} at {as_of} ({})", kind.as_str(), report.currency);
                println!(
                    "{:<20} {:>14}",
                    "not yet due",
                    amount_text(report.current.micros(), currency)?
                );
                println!(
                    "{:<20} {:>14}",
                    "1-30 days",
                    amount_text(report.days_1_30.micros(), currency)?
                );
                println!(
                    "{:<20} {:>14}",
                    "31-60 days",
                    amount_text(report.days_31_60.micros(), currency)?
                );
                println!(
                    "{:<20} {:>14}",
                    "61-90 days",
                    amount_text(report.days_61_90.micros(), currency)?
                );
                println!(
                    "{:<20} {:>14}",
                    "over 90 days",
                    amount_text(report.over_90.micros(), currency)?
                );
                println!(
                    "{:<20} {:>14}",
                    "total",
                    amount_text(report.total()?.micros(), currency)?
                );
                return Ok(());
            }

            let items: Vec<store::OpenItem> = store::open_items(connection, owner, kind)?
                .into_iter()
                .filter(store::OpenItem::is_open)
                .collect();
            if items.is_empty() {
                println!("no open {}", kind.as_str());
                return Ok(());
            }
            println!(
                "{:<14} {:<12} {:>14} {:<12} {:<36}",
                "document", "date", "amount", "due", "party"
            );
            for item in &items {
                println!(
                    "{:<14} {:<12} {:>14} {:<12} {:<36}",
                    item.number,
                    item.original_date.to_iso_string(),
                    amount_text(item.outstanding.micros(), currency)?,
                    item.due_date.map(|due| due.to_iso_string()).unwrap_or_default(),
                    item.party_id
                );
            }
            let total = store::open_item_total(connection, owner, kind)?;
            println!("{:<27} {:>14}", "total", amount_text(total.micros(), currency)?);
            Ok(())
        }
        LedgerCommand::Reverse {
            entry,
            reason,
            date,
            actor,
        } => {
            let id = resolve_entry(session.database(), &entry)?;
            let date = parse_date_or(date, LocalDate::today_utc())?;
            let posted = store::reverse_entry(connection, id, &reason, date, &actor, &now)?;
            println!("reversed {} as {} {}", entry, posted.journal.code(), posted.number);
            println!("dated {date}");
            Ok(())
        }
    }
}

/// Reads a statement file, whichever of the two formats it turns out to be.
///
/// Detection is on the bytes, not the extension: a bank hands out
/// `afschrift.xml`, `afschrift.940`, and `afschrift.txt` with equal confidence,
/// and the extension is never the one that says which format it wrote.
fn read_statement(bytes: &[u8], text: &str) -> Result<Vec<store::ParsedStatement>, import::bank_file::BankError> {
    match import::sniff(bytes) {
        import::SniffedKind::Mt940 => import::mt940::read_mt940(text),
        _ => import::camt::read_camt053(text),
    }
}

/// Runs one bank subcommand.
fn bank(database: &store::Database, owner: SellerId, command: BankCommand, now: &str) -> Result<(), CliError> {
    let connection = database.connection();
    match command {
        BankCommand::Add { name, iban, ledger } => {
            let account = store::BankAccount {
                id: DocumentId::new().to_string(),
                seller_id: owner,
                name,
                iban,
                currency: store::reporting_currency(connection, owner)?,
                ledger_account_code: crux::AccountCode::parse(&ledger)
                    .map_err(|_| CliError::Input(format!("{ledger:?} is not an account code")))?,
                import_profile: None,
            };
            store::insert_bank_account(connection, &account, now)?;
            println!("bank account: {}", account.id);
            println!("name:         {}", account.name);
            println!("ledger:       {}", account.ledger_account_code);
            Ok(())
        }
        BankCommand::Accounts => {
            let accounts = store::list_bank_accounts(connection, owner)?;
            if accounts.is_empty() {
                println!("no bank accounts; add one with `bonk ledger bank add --name <name>`");
                return Ok(());
            }
            for account in accounts {
                println!(
                    "{:<38} {:<24} {:<8} {}",
                    account.id,
                    account.name,
                    account.ledger_account_code,
                    account.iban.unwrap_or_default()
                );
            }
            Ok(())
        }
        BankCommand::Import { file, account } => {
            let accounts = store::list_bank_accounts(connection, owner)?;
            let account = match (account, accounts.as_slice()) {
                (Some(needle), _) => accounts
                    .iter()
                    .find(|account| account.id == needle || account.name == needle)
                    .ok_or_else(|| CliError::Input(format!("no bank account matching {needle:?}")))?,
                (None, [only]) => only,
                (None, []) => {
                    return Err(CliError::Input(
                        "add a bank account first: `bonk ledger bank add --name <name>`".to_owned(),
                    ));
                }
                (None, many) => {
                    let names: Vec<&str> = many.iter().map(|account| account.name.as_str()).collect();
                    return Err(CliError::Input(format!(
                        "more than one bank account ({}); choose one with --account",
                        names.join(", ")
                    )));
                }
            };

            let bytes = std::fs::read(&file)?;
            let text = String::from_utf8(bytes.clone()).map_err(|_| {
                CliError::Input(format!("{} is not UTF-8; a CAMT or MT940 statement is", file.display()))
            })?;
            let statements = read_statement(&bytes, &text)
                .map_err(|error| CliError::Input(format!("{}: {error}", file.display())))?;
            if statements.is_empty() {
                return Err(CliError::Input(format!("{} holds no statement", file.display())));
            }

            let file_hash = store::audit::sha256_hex(&bytes);
            let mut total = 0_usize;
            for (index, mut statement) in statements.into_iter().enumerate() {
                statement.source_name = Some(
                    file.file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("statement")
                        .to_owned(),
                );
                // One file can hold several statements; the hash names which one
                // so importing a file twice stays a no-op for all of them.
                let payload = store::audit::sha256_hex(format!("{file_hash}:{index}").as_bytes());
                let report = store::import_statement(connection, &account.id, &statement, &payload, now)?;
                total += report.imported;
                if report.created {
                    println!(
                        "statement {}: {} imported, {} already known",
                        report.statement_id, report.imported, report.duplicates
                    );
                } else {
                    println!("already imported: {}", report.statement_id);
                }
            }
            println!("imported: {total}");
            Ok(())
        }
        BankCommand::Propose { transaction } => {
            let candidates = store::proposals(connection, owner, &transaction)?;
            if candidates.is_empty() {
                println!("nothing to propose for {transaction}");
                return Ok(());
            }
            for candidate in &candidates {
                println!(
                    "{:<14} {:>14} {:<14} {}",
                    candidate.number,
                    amount_text(candidate.applies.micros(), candidate.applies.currency())?,
                    candidate.method.as_str(),
                    candidate.reason
                );
                println!("  item {}", candidate.open_item_id);
            }
            Ok(())
        }
        BankCommand::AutoMatch => {
            let matched = store::auto_match(connection, owner, "cli", now)?;
            if matched.is_empty() {
                println!("nothing matched automatically");
                return Ok(());
            }
            for report in &matched {
                println!(
                    "matched {} to transaction {}: {}",
                    report.number,
                    report.transaction_id,
                    amount_text(report.settled.micros(), report.settled.currency())?
                );
                println!("  allocation {}", report.allocation_id);
            }
            println!("matched: {}", matched.len());
            Ok(())
        }
        BankCommand::Match { transaction, item } => {
            let report = store::allocate(
                connection,
                owner,
                &transaction,
                &item,
                store::MatchMethod::Manual,
                "cli",
                now,
            )?;
            println!("matched {} to {}", report.number, report.transaction_id);
            println!(
                "settled:    {}",
                amount_text(report.settled.micros(), report.settled.currency())?
            );
            println!("entry:      BNK {}", report.entry_id);
            println!("allocation: {}", report.allocation_id);
            Ok(())
        }
        BankCommand::Unmatch { allocation } => {
            store::unmatch(connection, owner, &allocation, "cli", now)?;
            println!("unmatched allocation {allocation}");
            println!("the invoice is open again and the reversal is on the record");
            Ok(())
        }
        BankCommand::Transactions { status } => {
            let rows = store::list_transactions(connection, owner, status.as_deref())?;
            if rows.is_empty() {
                println!("no bank transactions");
                return Ok(());
            }
            println!(
                "{:<12} {:>14} {:<28} {:<10} {:<40}",
                "date", "amount", "counterparty", "status", "remittance"
            );
            for row in &rows {
                println!(
                    "{:<12} {:>14} {:<28} {:<10} {:<40}",
                    row.booking_date.to_iso_string(),
                    amount_text(row.amount.micros(), row.amount.currency())?,
                    row.counterparty_name.clone().unwrap_or_default(),
                    row.status,
                    row.remittance_info.clone().unwrap_or_default()
                );
            }
            Ok(())
        }
    }
}

/// Resolves `VK:2026-0001` or a bare entry identifier.
fn resolve_entry(database: &store::Database, needle: &str) -> Result<JournalEntryId, CliError> {
    match needle.split_once(':') {
        Some((journal, number)) => {
            let journal = Journal::parse(journal).map_err(|_| {
                CliError::Input(format!(
                    "unknown journal {journal:?}; use VK, IK, BNK, KAS, MEM, OPEN, or BTW"
                ))
            })?;
            store::find_entry(database.connection(), journal, number)?
                .ok_or_else(|| CliError::Input(format!("no entry {needle:?}")))
        }
        None => JournalEntryId::parse_str(needle)
            .map_err(|_| CliError::Input(format!("{needle:?} is neither VK:2026-0001 nor an entry identifier"))),
    }
}

/// The window a report covers: this calendar year up to today unless asked otherwise.
fn window(from: Option<String>, to: Option<String>) -> Result<(LocalDate, LocalDate), CliError> {
    let today = LocalDate::today_utc();
    let default_from = LocalDate::from_ymd(today.as_date().year(), 1, 1)?;
    Ok((parse_date_or(from, default_from)?, parse_date_or(to, today)?))
}

/// Parses an optional `YYYY-MM-DD`, falling back to a default.
fn parse_date_or(raw: Option<String>, default: LocalDate) -> Result<LocalDate, CliError> {
    match raw {
        Some(value) => Ok(LocalDate::parse_iso(&value)
            .map_err(|_| CliError::Input(format!("{value:?} is not a date, expected YYYY-MM-DD")))?),
        None => Ok(default),
    }
}

/// An amount in micro-units, printed with two decimals and no currency symbol.
fn amount_text(micros: i64, currency: Currency) -> Result<String, CliError> {
    Ok(Money::from_micros(micros, currency).to_fixed_string(2)?)
}

/// An amount in one column of a printed statement: nothing rather than `0.00`.
///
/// Column-oriented output is what a bookkeeper reads, and a column of zeros
/// hides the two numbers that matter.
fn column_text(micros: i64, currency: Currency) -> Result<String, CliError> {
    if micros == 0 {
        return Ok(String::new());
    }
    amount_text(micros, currency)
}

/// How a command wants to open the archive.
struct Access {
    /// Given on the command line, in the environment, or prompted for later.
    passphrase: Option<String>,
    /// Use the key this device remembered instead.
    device: bool,
}

impl Access {
    /// Opens the archive.
    ///
    /// The passphrase is resolved here rather than in `run`, so a command that
    /// keeps its own prompt (`init`, `remember`, the credential changes) does not
    /// ask for one it will not use.
    fn session(&self, database: &Path, keystore: &Path) -> Result<Session, CliError> {
        if self.device {
            return Ok(Session::unlock_with_device(
                database,
                keystore,
                &store::device::Keychain,
            )?);
        }
        let passphrase = resolve_passphrase(self.passphrase.clone())?;
        Ok(Session::unlock(database, keystore, &passphrase)?)
    }
}

fn seed_demo(
    database: &Path,
    keystore: &Path,
    passphrase: Option<String>,
    options: demo::Options,
    force: bool,
) -> Result<(), CliError> {
    let existed = database.exists() || keystore.exists();
    if existed && !force {
        return Err(CliError::Input(format!(
            "an archive already exists at {}; pass --force to replace it",
            database.display()
        )));
    }
    if existed {
        remove_archive(database, keystore)?;
    }

    let passphrase = passphrase.unwrap_or_else(|| demo::DEFAULT_PASSPHRASE.to_owned());
    let mut session = Session::create(database, keystore, &passphrase)?;
    let seeded = demo::seed(&mut session, &options)?;

    println!("database:   {}", database.display());
    println!("keystore:   {}", keystore.display());
    if passphrase == demo::DEFAULT_PASSPHRASE {
        println!("passphrase: {}", demo::DEFAULT_PASSPHRASE);
    } else {
        println!("passphrase: (the one you gave)");
    }
    println!("seller:     Studio Noord B.V.");
    println!("customers:  {}", seeded.customers);
    println!(
        "invoices:   {} issued, {} of them credit notes",
        seeded.invoices, seeded.credit_notes
    );
    println!(
        "received:   {} ({} matched, {} parked)",
        seeded.received, seeded.matched, seeded.parked
    );
    println!("drafts:     {}", seeded.drafts);
    println!("views:      {} saved searches", seeded.views);
    println!();
    let (default_database, _) = app::default_paths();
    if default_database.as_path() == database {
        println!("open the window with: bonk gui");
    } else {
        println!(
            "note: the window opens {}; this archive is at {}",
            default_database.display(),
            database.display()
        );
    }
    Ok(())
}

/// Removes the files an archive owns, including the write-ahead log.
fn remove_archive(database: &Path, keystore: &Path) -> Result<(), CliError> {
    let mut paths = vec![database.to_path_buf(), keystore.to_path_buf()];
    if let Some(name) = database.file_name().and_then(|name| name.to_str()) {
        for suffix in ["-wal", "-shm"] {
            paths.push(database.with_file_name(format!("{name}{suffix}")));
        }
    }
    for path in paths {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(CliError::Io(error)),
        }
    }
    Ok(())
}

/// Reports a long audit's progress on stderr.
///
/// `stdout` carries the report, so progress must never go there: a caller piping
/// `verify` into a file should get only the report. One line is printed per 5 %
/// step, which keeps a minutes-long pass to a couple of dozen lines instead of
/// thousands. Lines rather than a rewritten status line, because the output is
/// just as often copied out of a terminal or collected by a log as it is watched.
fn progress_reporter(label: &'static str) -> impl FnMut(usize, usize) {
    let mut reported: Option<usize> = None;
    move |done: usize, total: usize| {
        if total == 0 {
            return;
        }
        let percent = done.saturating_mul(100) / total;
        let finished = done >= total;
        let due = match reported {
            Some(last) => percent >= last.saturating_add(5) || finished,
            None => true,
        };
        if !due {
            return;
        }
        reported = Some(percent);
        eprintln!("{label}: {done}/{total} ({percent}%)");
    }
}

fn paths(cli: &Cli) -> (PathBuf, PathBuf) {
    let (default_database, default_keystore) = app::default_paths();
    let database = cli.db.clone().unwrap_or(default_database);
    let keystore = cli.keystore.clone().unwrap_or_else(|| {
        database
            .parent()
            .map(|parent| parent.join("keystore.bin"))
            .unwrap_or(default_keystore)
    });
    (database, keystore)
}

#[allow(clippy::too_many_arguments)]
fn party(
    name: String,
    city: String,
    country: String,
    street: Option<String>,
    postal: Option<String>,
    vat: Option<String>,
    kvk: Option<String>,
    peppol: Option<String>,
    email: Option<String>,
    contact: Option<String>,
) -> Result<Party, CliError> {
    let mut party = Party::new(
        name,
        Address {
            street,
            city,
            postal_code: postal,
            country: CountryCode::parse(&country)?,
        },
    );
    party.vat_id = vat.map(|value| value.trim().to_ascii_uppercase());
    party.kvk_number = kvk;
    party.peppol_id = peppol.map(|value| value.trim().to_owned());
    party.email = email;
    party.contact_name = contact;
    Ok(party)
}

fn parse_line(spec: &str, currency: Currency, position: u32) -> Result<DraftLine, CliError> {
    let parts: Vec<&str> = spec.split('|').map(str::trim).collect();
    if parts.len() < 4 {
        return Err(CliError::Input(format!(
            "line {spec:?} must be name|quantity|unit_price|rate"
        )));
    }
    if parts[0].is_empty() {
        return Err(CliError::Input("line name must not be empty".to_owned()));
    }

    let quantity = Quantity::parse(parts[1])?;
    let unit_price = Money::parse_fixed(parts[2], currency)?;
    let rate = VatRate::parse_percent(parts[3])?;
    let category = match parts.get(4).filter(|value| !value.is_empty()) {
        Some(code) => VatCategory::parse(code)?,
        None if rate.is_zero() => VatCategory::ZeroRated,
        None => VatCategory::Standard,
    };
    let unit = match parts.get(5).filter(|value| !value.is_empty()) {
        Some(code) => UnitCode::parse(code)?,
        None => UnitCode::PIECE,
    };
    let exemption_reason_code = parts
        .get(6)
        .filter(|value| !value.is_empty())
        .map(|value| (*value).to_owned());
    let exemption_reason = parts
        .get(7)
        .filter(|value| !value.is_empty())
        .map(|value| (*value).to_owned());

    Ok(DraftLine {
        id: LineId::new(),
        position,
        name: parts[0].to_owned(),
        description: None,
        quantity,
        unit,
        unit_price,
        price_base_quantity: Quantity::ONE,
        treatment: TaxTreatment {
            category,
            rate,
            exemption_reason,
            exemption_reason_code,
        },
    })
}

/// Imports what a watcher handed over, one line per file.
///
/// Returns how many files failed, so `--once` can fail the run while the poll
/// loop carries on: a folder keeps its purpose even when one drop is not an
/// invoice.
fn import_watched(session: &Session, owner: SellerId, files: &[PathBuf]) -> Result<usize, CliError> {
    let mut failed = 0_usize;
    for file in files {
        let now = store::now_rfc3339();
        match import::import_file(session.database(), owner, file, &now) {
            Ok(outcome) if outcome.duplicate => {
                println!("duplicate: {} {}", outcome.number, outcome.path);
            }
            Ok(outcome) => {
                let supplier = outcome
                    .supplier
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "unmatched".to_owned());
                println!("imported:  {} (supplier {supplier}) {}", outcome.number, outcome.path);
            }
            Err(error) => {
                eprintln!("error:     {}: {error}", file.display());
                failed += 1;
            }
        }
    }
    Ok(failed)
}

fn print_findings(findings: &[Finding]) {
    for finding in findings {
        let severity = match finding.severity {
            Severity::Blocking => "blocking",
            Severity::Warning => "warning",
            Severity::Info => "info",
        };
        match finding.bt {
            Some(bt) => eprintln!(
                "{severity} {} [{bt}] {}: {}",
                finding.code, finding.field, finding.message
            ),
            None => eprintln!("{severity} {} {}: {}", finding.code, finding.field, finding.message),
        }
    }
}

fn owner_of(database: &store::Database, explicit: Option<String>) -> Result<SellerId, CliError> {
    match explicit {
        Some(value) => Ok(SellerId::parse_str(&value)?),
        None => first_seller(database.connection())?
            .ok_or_else(|| CliError::Input("this database has no seller".to_owned()))
            .map(|seller| seller.id),
    }
}

/// The filters `search` and `export` share.
struct Filters {
    direction: Option<store::Direction>,
    from: Option<LocalDate>,
    to: Option<LocalDate>,
    status: Option<DocumentStatus>,
}

fn filters(
    direction: Option<String>,
    from: Option<String>,
    to: Option<String>,
    status: Option<String>,
) -> Result<Filters, CliError> {
    let direction = match direction.as_deref() {
        Some(value) => Some(
            store::Direction::parse(value)
                .ok_or_else(|| CliError::Input(format!("unknown direction {value:?}; use outbound or inbound")))?,
        ),
        None => None,
    };
    let date = |flag: &str, value: Option<String>| -> Result<Option<LocalDate>, CliError> {
        value
            .map(|value| LocalDate::parse_iso(&value).map_err(|error| CliError::Input(format!("--{flag}: {error}"))))
            .transpose()
    };
    let status = match status {
        Some(value) => Some(DocumentStatus::parse(&value).ok_or_else(|| {
            CliError::Input(format!(
                "unknown status {value:?}; use draft, issued, received, approved, paid, or archived"
            ))
        })?),
        None => None,
    };
    Ok(Filters {
        direction,
        from: date("from", from)?,
        to: date("to", to)?,
        status,
    })
}

/// Moves a received document forward in the payable register.
fn settle_received(
    access: &Access,
    database: &Path,
    keystore: &Path,
    document: &str,
    target: DocumentStatus,
) -> Result<(), CliError> {
    let session = access.session(database, keystore)?;
    let connection = session.database().connection();
    let owner = owner_of(session.database(), None)?;
    let id = resolve_received(connection, owner, document)?
        .ok_or_else(|| CliError::Input(format!("no received document matching {document:?}")))?;
    let status = advance_status(connection, id, target, "cli", &store::now_rfc3339())?;
    println!("{status}: {document}");
    Ok(())
}

fn warn_if_unencrypted() {
    if !store::cipher_enabled() {
        eprintln!("warning: this build stores the database unencrypted (plain feature, test only)");
    }
}

fn resolve_passphrase(explicit: Option<String>) -> Result<Zeroizing<String>, CliError> {
    if let Some(value) = from_flag_or_environment(explicit) {
        return Ok(value);
    }
    prompt("Passphrase: ")
}

/// The passphrase for a new archive.
///
/// A flag or `BONK_PASSPHRASE` is a single source and is taken as it stands. The
/// interactive prompt asks twice, because the archive is sealed with whatever
/// was typed and a typo leaves it with no way in.
fn resolve_creation_passphrase(explicit: Option<String>) -> Result<Zeroizing<String>, CliError> {
    match from_flag_or_environment(explicit) {
        Some(value) => non_blank(value),
        None => non_blank(confirmed_prompt()?),
    }
}

/// The recovery key for `passphrase-reset`.
fn resolve_recovery_key(explicit: Option<String>) -> Result<Zeroizing<String>, CliError> {
    if let Some(value) = explicit.filter(|value| !value.trim().is_empty()) {
        return Ok(Zeroizing::new(value));
    }
    if let Ok(value) = std::env::var("BONK_RECOVERY_KEY")
        && !value.trim().is_empty()
    {
        return Ok(Zeroizing::new(value));
    }
    prompt("Recovery key: ")
}

/// The new passphrase for `passphrase-change`, asked for twice when interactive.
fn resolve_new_passphrase(explicit: Option<String>) -> Result<Zeroizing<String>, CliError> {
    let value = match explicit.filter(|value| !value.is_empty()) {
        Some(value) => Zeroizing::new(value),
        None => match std::env::var("BONK_NEW_PASSPHRASE") {
            Ok(value) if !value.is_empty() => Zeroizing::new(value),
            _ => confirmed_prompt()?,
        },
    };
    non_blank(value)
}

/// Reads a passphrase from the terminal without echoing it.
fn prompt(label: &str) -> Result<Zeroizing<String>, CliError> {
    rpassword::prompt_password(label)
        .map(Zeroizing::new)
        .map_err(|error| CliError::Input(error.to_string()))
}

/// Asks for a passphrase twice and refuses a mismatch.
fn confirmed_prompt() -> Result<Zeroizing<String>, CliError> {
    let first = prompt("New passphrase: ")?;
    let again = prompt("Repeat the new passphrase: ")?;
    if *first != *again {
        return Err(CliError::Input("the passphrases do not match".to_owned()));
    }
    Ok(first)
}

fn non_blank(value: Zeroizing<String>) -> Result<Zeroizing<String>, CliError> {
    if value.trim().is_empty() {
        return Err(CliError::Input("an empty passphrase is not accepted".to_owned()));
    }
    Ok(value)
}

/// Advises on stderr when a passphrase looks too weak to protect the archive.
///
/// This is advice, not a gate: the check is a heuristic, and refusing to create
/// the archive the user asked for would be worse than telling them the truth
/// once.
fn warn_if_weak(passphrase: &str) {
    let assessment = strength::assess(passphrase);
    if assessment.strength == strength::Strength::Weak {
        eprintln!(
            "warning: this passphrase is weak ({} bits): {}",
            assessment.bits,
            text::strength::reason(text::Language::default(), assessment.reason)
        );
    }
}

fn from_flag_or_environment(explicit: Option<String>) -> Option<Zeroizing<String>> {
    if let Some(value) = explicit.filter(|value| !value.is_empty()) {
        return Some(Zeroizing::new(value));
    }
    std::env::var("BONK_PASSPHRASE")
        .ok()
        .filter(|value| !value.is_empty())
        .map(Zeroizing::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_keystore_sits_beside_the_database() {
        let cli = Cli {
            db: Some(PathBuf::from("/tmp/work/bonk.db")),
            keystore: None,
            passphrase: None,
            device: false,
            command: None,
        };
        let (database, keystore) = paths(&cli);
        assert_eq!(database, PathBuf::from("/tmp/work/bonk.db"));
        assert_eq!(keystore, PathBuf::from("/tmp/work/keystore.bin"));
    }

    #[test]
    fn explicit_passphrase_wins() {
        let passphrase = resolve_passphrase(Some("secret".to_owned())).unwrap();
        assert_eq!(&*passphrase, "secret");
        assert!(
            !format!("{passphrase:?}").contains("secret"),
            "a resolved passphrase must not print itself"
        );
    }

    #[test]
    fn a_creation_passphrase_from_a_flag_is_taken_once_and_must_not_be_blank() {
        let passphrase = resolve_creation_passphrase(Some("secret-canary".to_owned())).unwrap();
        assert_eq!(&*passphrase, "secret-canary");
        assert!(!format!("{passphrase:?}").contains("secret-canary"));
        assert!(resolve_creation_passphrase(Some("   ".to_owned())).is_err());
    }

    #[test]
    fn a_new_passphrase_is_never_empty_or_printable() {
        let passphrase = resolve_new_passphrase(Some("secret-canary".to_owned())).unwrap();
        assert_eq!(&*passphrase, "secret-canary");
        assert!(!format!("{passphrase:?}").contains("secret-canary"));
        assert!(resolve_new_passphrase(Some("   ".to_owned())).is_err());

        let key = resolve_recovery_key(Some("recovery-canary".to_owned())).unwrap();
        assert_eq!(&*key, "recovery-canary");
        assert!(!format!("{key:?}").contains("recovery-canary"));
    }

    #[test]
    fn line_parsing_covers_the_common_case() {
        let line = parse_line("Consulting|2|50.00|21", Currency::EUR, 1).unwrap();
        assert_eq!(line.name, "Consulting");
        assert_eq!(line.quantity.micros(), 2_000_000);
        assert_eq!(line.unit_price.micros(), 50_000_000);
        assert_eq!(line.treatment.category, VatCategory::Standard);
        assert_eq!(line.treatment.rate, VatRate::STANDARD_NL);
    }

    #[test]
    fn line_parsing_defaults_zero_rate_to_zero_rated() {
        let line = parse_line("Export|1|100|0", Currency::EUR, 1).unwrap();
        assert_eq!(line.treatment.category, VatCategory::ZeroRated);
    }

    #[test]
    fn line_parsing_accepts_exemption_reasons() {
        let line = parse_line("Exempt|1|100|0|E|||Exempt supply", Currency::EUR, 1).unwrap();
        assert_eq!(line.treatment.category, VatCategory::Exempt);
        assert!(line.treatment.exemption_reason_code.is_none());
        assert_eq!(line.treatment.exemption_reason.as_deref(), Some("Exempt supply"));

        let line = parse_line(
            "Reverse charge|1|100|0|AE||VATEX-EU-AE|Reverse charge",
            Currency::EUR,
            1,
        )
        .unwrap();
        assert_eq!(line.treatment.exemption_reason_code.as_deref(), Some("VATEX-EU-AE"));
        assert_eq!(line.treatment.exemption_reason.as_deref(), Some("Reverse charge"));
    }

    #[test]
    fn line_parsing_rejects_garbage() {
        assert!(parse_line("Consulting|2|50", Currency::EUR, 1).is_err());
        assert!(parse_line("|2|50|21", Currency::EUR, 1).is_err());
        assert!(parse_line("Consulting|nope|50|21", Currency::EUR, 1).is_err());
    }
}
