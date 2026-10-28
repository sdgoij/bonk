#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! The `bonk` executable: GUI by default, headless subcommands for support and CI.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use app::Session;
use clap::{Parser, Subcommand};
use crux::{
    Address, CountryCode, Currency, CustomerId, DocumentId, DraftLine, Finding, InvoiceDraft, LineId, LocalDate, Money,
    Party, Quantity, SellerId, Severity, TaxTreatment, UnitCode, VatCategory, VatRate, calculate_totals, has_blocking,
    validate_draft,
};
use store::{
    CustomerRecord, DocumentStatus, IssueRequest, KeyStore, SearchRequest, SellerRecord, StoreError, advance_status,
    credit_note_for, first_seller, insert_customer, insert_draft, insert_seller, issue_draft, list_received,
    load_customer, load_draft, load_seller, resolve_customer, resolve_received, set_supplier_match,
};

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

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Create an encrypted database and its keystore.
    Init,
    /// Check integrity, the audit chain, and every stored artefact.
    Verify,
    /// Write a verified, encrypted backup and prune older ones.
    Backup {
        /// Directory for backups. Defaults to `backups` beside the database.
        #[arg(long)]
        to: Option<PathBuf>,
        /// How many backups to keep.
        #[arg(long, default_value_t = store::backup::RETENTION)]
        keep: usize,
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
        #[arg(long, default_value = "cli")]
        device: String,
    },
    /// Open the desktop UI (also the default with no subcommand).
    Gui,
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
}

fn main() -> ExitCode {
    let cli = Cli::parse();
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
    let passphrase = cli.passphrase.clone();
    match cli.command {
        None | Some(Command::Gui) => app::run().map_err(|error| CliError::Gui(error.to_string())),
        Some(Command::Backup { to, keep }) => {
            let passphrase = resolve_passphrase(passphrase)?;
            let key_store = KeyStore::open(&keystore, &passphrase)?;
            let session = Session::unlock(&database, &keystore, &passphrase)?;
            let directory = to.unwrap_or_else(|| {
                database
                    .parent()
                    .map(|parent| parent.join("backups"))
                    .unwrap_or_else(|| PathBuf::from("backups"))
            });

            let report =
                store::backup::backup(session.database(), &key_store, &directory, &store::now_rfc3339(), keep)?;
            println!("backup: {} ({} bytes, verified)", report.path.display(), report.bytes);
            for removed in &report.removed {
                println!("pruned: {}", removed.display());
            }
            Ok(())
        }
        Some(Command::VerifyBackup { path }) => {
            let passphrase = resolve_passphrase(passphrase)?;
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
            let passphrase = resolve_passphrase(passphrase)?;
            let new_passphrase = resolve_new_passphrase(new_passphrase)?;

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
        Some(Command::RecoveryAdd) => {
            let passphrase = resolve_passphrase(passphrase)?;
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
            let passphrase = resolve_passphrase(passphrase.clone())?;
            let session = Session::create(&database, &keystore, &passphrase)?;
            let summary = session.summary()?;
            println!("database: {}", database.display());
            println!("keystore: {}", keystore.display());
            println!("schema:   {}", summary.schema_version);
            println!("encryption: {}", store::encryption_label());
            Ok(())
        }
        Some(Command::Verify) => {
            let session = open(&database, &keystore, passphrase.clone())?;
            session.integrity_check()?;
            let summary = session.summary()?;
            let audit_events = store::audit::verify_chain(session.database().connection())?;
            let report = store::integrity::audit(session.database().connection())?;

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
            Ok(())
        }
        Some(Command::MigrateDown { to, yes }) => {
            if !yes {
                return Err(CliError::Input(
                    "refusing to roll back migrations without --yes".to_owned(),
                ));
            }
            let mut session = open(&database, &keystore, passphrase.clone())?;
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
            let session = open(&database, &keystore, passphrase.clone())?;
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
            let session = open(&database, &keystore, passphrase.clone())?;
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
        }) => {
            let session = open(&database, &keystore, passphrase.clone())?;
            let seller_id = SellerId::parse_str(&seller)?;
            let customer_id = CustomerId::parse_str(&customer)?;
            let seller_record = load_seller(session.database().connection(), seller_id)?;
            let customer_record = load_customer(session.database().connection(), customer_id)?;

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
            let session = open(&database, &keystore, passphrase.clone())?;
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
        Some(Command::Import { files, seller }) => {
            let session = open(&database, &keystore, passphrase.clone())?;
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
            let session = open(&database, &keystore, passphrase.clone())?;
            let connection = session.database().connection();
            let owner = owner_of(session.database(), None)?;
            let filter = match status {
                Some(value) => Some(DocumentStatus::parse(&value).ok_or_else(|| {
                    CliError::Input(format!(
                        "unknown status {value:?}; use issued, received, approved, paid, or archived"
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
            let session = open(&database, &keystore, passphrase.clone())?;
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
        Some(Command::Export {
            query,
            format,
            direction,
            from,
            to,
            status,
            out,
        }) => {
            let session = open(&database, &keystore, passphrase.clone())?;
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
            let session = open(&database, &keystore, passphrase.clone())?;
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
            settle_received(&database, &keystore, passphrase, &document, DocumentStatus::Approved)
        }
        Some(Command::MarkPaid { document }) => {
            settle_received(&database, &keystore, passphrase, &document, DocumentStatus::Paid)
        }
        Some(Command::Issue {
            draft,
            out,
            actor,
            device,
        }) => {
            let mut session = open(&database, &keystore, passphrase.clone())?;
            let draft_id = DocumentId::parse_str(&draft)?;

            let loaded = load_draft(session.database().connection(), draft_id)?
                .ok_or_else(|| CliError::Input(format!("draft {draft_id} does not exist")))?;
            let seller = load_seller(session.database().connection(), loaded.seller_id)?;
            let customer = load_customer(session.database().connection(), loaded.customer_id)?;

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
                    device: &device,
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

            match app::invoice_pdf(&issued.number, &loaded, &seller, &customer, &issued.totals) {
                Ok(pdf) => {
                    let pdf_path = out.join(format!("{}.pdf", issued.number));
                    std::fs::write(&pdf_path, &pdf)?;
                    let now = store::now_rfc3339();
                    match store::attach_artefact(
                        session.database().connection(),
                        issued.id,
                        "pdf",
                        &pdf,
                        "typst-cli-0.14",
                        &now,
                    ) {
                        Ok(hash) => println!("pdf:    {} ({hash})", pdf_path.display()),
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
    }
}

fn open(database: &Path, keystore: &Path, passphrase: Option<String>) -> Result<Session, CliError> {
    let passphrase = resolve_passphrase(passphrase)?;
    Ok(Session::unlock(database, keystore, &passphrase)?)
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
                "unknown status {value:?}; use issued, received, approved, paid, or archived"
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
    database: &Path,
    keystore: &Path,
    passphrase: Option<String>,
    document: &str,
    target: DocumentStatus,
) -> Result<(), CliError> {
    let session = open(database, keystore, passphrase)?;
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

fn resolve_passphrase(explicit: Option<String>) -> Result<String, CliError> {
    if let Some(value) = explicit
        && !value.is_empty()
    {
        return Ok(value);
    }
    if let Ok(value) = std::env::var("BONK_PASSPHRASE")
        && !value.is_empty()
    {
        return Ok(value);
    }
    rpassword::prompt_password("Passphrase: ").map_err(|error| CliError::Input(error.to_string()))
}

/// The new passphrase for `passphrase-change`, asked for twice when interactive.
fn resolve_new_passphrase(explicit: Option<String>) -> Result<String, CliError> {
    let value = match explicit.filter(|value| !value.is_empty()) {
        Some(value) => value,
        None => match std::env::var("BONK_NEW_PASSPHRASE") {
            Ok(value) if !value.is_empty() => value,
            _ => {
                let first = rpassword::prompt_password("New passphrase: ")
                    .map_err(|error| CliError::Input(error.to_string()))?;
                let again = rpassword::prompt_password("Repeat the new passphrase: ")
                    .map_err(|error| CliError::Input(error.to_string()))?;
                if first != again {
                    return Err(CliError::Input("the passphrases do not match".to_owned()));
                }
                first
            }
        },
    };
    if value.trim().is_empty() {
        return Err(CliError::Input("an empty passphrase is not accepted".to_owned()));
    }
    Ok(value)
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
            command: None,
        };
        let (database, keystore) = paths(&cli);
        assert_eq!(database, PathBuf::from("/tmp/work/bonk.db"));
        assert_eq!(keystore, PathBuf::from("/tmp/work/keystore.bin"));
    }

    #[test]
    fn explicit_passphrase_wins() {
        assert_eq!(resolve_passphrase(Some("secret".to_owned())).unwrap(), "secret");
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
