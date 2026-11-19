//! The accountant handoff package.
//!
//! A package is a directory, like an evidence bundle, rather than one archive
//! file: every part stays readable with ordinary tools, and nothing depends on a
//! zip implementation staying byte-stable. The manifest lists every file with its
//! SHA-256, so the accountant can check that what arrived is what was written.
//!
//! What it carries: the three reports the books are read through, the journal
//! listing with each entry's content hash, the open items at the end of the
//! window, every prepared VAT return in it, the ICP listing for every month that
//! had intra-EU supplies, and the document bytes the archive filed. What it does
//! not carry: the audit chain itself. The manifest names its head, which is enough
//! to tie the package to the archive; proving the chain needs the bundle the
//! evidence export writes.
//!
//! The ledger reports are as of the window. The open-items file is the position
//! when the package was written, because the subledger does not store an as-of
//! settlement date; the README says so rather than letting a reader assume
//! otherwise.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crux::{Currency, LocalDate, Money, SellerId};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::audit;
use crate::error::StoreError;
use crate::evidence::{extension, extension_for_source, safe_name};
use crate::vat;

/// Marker in the manifest.
pub const FORMAT: &str = "bonk-accountant-package";
/// Package layout version.
pub const VERSION: u32 = 1;

const MANIFEST: &str = "manifest.json";
const README: &str = "README.txt";

/// What an export wrote.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PackageReport {
    /// First day covered, inclusive.
    pub from: LocalDate,
    /// Last day covered, inclusive.
    pub to: LocalDate,
    /// Files covered by the manifest, the README included.
    pub files: usize,
    /// Total bytes of those files.
    pub bytes: u64,
    /// Documents dated in the window.
    pub documents: usize,
    /// Journal entries dated in the window.
    pub journal_entries: usize,
    /// Prepared VAT returns included.
    pub vat_returns: usize,
    /// Months with an ICP listing.
    pub icp_months: usize,
    /// Hash of the newest audit event, when the chain has one.
    pub head: Option<String>,
}

#[derive(Serialize)]
struct Manifest {
    format: String,
    version: u32,
    seller_id: String,
    from: String,
    to: String,
    currency: String,
    exported_at: String,
    exported_by: String,
    audit: ManifestAudit,
    counts: ManifestCounts,
    files: Vec<ManifestFile>,
}

#[derive(Serialize)]
struct ManifestAudit {
    head: Option<ManifestHead>,
}

#[derive(Serialize)]
struct ManifestHead {
    seq: i64,
    hash: String,
    at: String,
}

#[derive(Serialize)]
struct ManifestCounts {
    documents: usize,
    journal_entries: usize,
    vat_returns: usize,
    icp_months: usize,
}

#[derive(Clone, Serialize)]
struct ManifestFile {
    path: String,
    sha256: String,
    size: u64,
}

/// A document that belongs in the package.
struct DocumentRow {
    id: String,
    number: String,
    direction: String,
}

/// Writes the accountant package for a window.
///
/// The directory must not already contain a package; a handoff is not silently
/// overwritten. Everything that can refuse — an ICP listing that cannot name a
/// buyer, a window that runs backwards — is checked before the directory is
/// created, so a refusal leaves nothing half-written behind.
pub fn export(
    conn: &Connection,
    seller_id: SellerId,
    from: LocalDate,
    to: LocalDate,
    directory: impl AsRef<Path>,
    actor: &str,
    now: &str,
) -> Result<PackageReport, StoreError> {
    if from > to {
        return Err(StoreError::Rejected(format!(
            "the window ends before it starts: {from} to {to}"
        )));
    }
    let directory = directory.as_ref();
    let currency = crate::report::reporting_currency(conn, seller_id)?;

    // Compute everything that can refuse before touching the filesystem.
    let trial = crate::report::trial_balance(conn, seller_id, from, to)?;
    let profit = crate::report::profit_and_loss(conn, seller_id, from, to)?;
    let sheet = crate::report::balance_sheet(conn, seller_id, to)?;
    let mut returns = Vec::new();
    let mut listings = Vec::new();
    for (year, month) in months_between(from, to) {
        if let Some(prepared) = vat::prepared(conn, seller_id, year, month)? {
            returns.push((year, month, prepared));
        }
        let listing = vat::icp(conn, seller_id, year, month)?;
        if !listing.is_ready() {
            let reasons: Vec<String> = listing
                .problems
                .iter()
                .map(|problem| format!("{}: {}", problem.number, problem.reason))
                .collect();
            return Err(StoreError::Rejected(format!(
                "the ICP listing for {year}-{month:02} cannot be filed: {}",
                reasons.join("; ")
            )));
        }
        listings.push((year, month, listing));
    }
    let documents = documents_in(conn, seller_id, from, to)?;
    let journal_entries = journal_entry_count(conn, seller_id, from, to)?;

    if directory.join(MANIFEST).exists() {
        return Err(StoreError::Rejected(format!(
            "{} already contains a package",
            directory.display()
        )));
    }
    fs::create_dir_all(directory)?;
    fs::create_dir_all(directory.join("vat"))?;
    fs::create_dir_all(directory.join("icp"))?;
    fs::create_dir_all(directory.join("documents"))?;

    let mut files = Vec::new();
    write_file(
        directory,
        "trial-balance.csv",
        trial_csv(&trial)?.as_bytes(),
        &mut files,
    )?;
    write_file(
        directory,
        "profit-and-loss.csv",
        profit_csv(&profit)?.as_bytes(),
        &mut files,
    )?;
    write_file(
        directory,
        "balance-sheet.csv",
        sheet_csv(&sheet)?.as_bytes(),
        &mut files,
    )?;
    write_file(
        directory,
        "journal.csv",
        journal_csv(conn, seller_id, from, to, currency)?.as_bytes(),
        &mut files,
    )?;
    write_file(
        directory,
        "open-items.csv",
        open_items_csv(conn, seller_id, to)?.as_bytes(),
        &mut files,
    )?;

    let mut vat_returns = 0_usize;
    for (year, month, prepared) in &returns {
        let path = format!("vat/vat-return-{year}-{month:02}.json");
        write_file(directory, &path, return_json(prepared)?.as_bytes(), &mut files)?;
        vat_returns += 1;
    }

    let mut icp_months = 0_usize;
    for (year, month, listing) in &listings {
        if listing.rows.is_empty() {
            continue;
        }
        let path = format!("icp/icp-{year}-{month:02}.csv");
        write_file(directory, &path, vat::icp_csv(listing)?.as_bytes(), &mut files)?;
        icp_months += 1;
    }

    write_documents(conn, &documents, directory, &mut files)?;

    let readme = readme(conn, seller_id, from, to, currency, files.len() + 1)?;
    write_file(directory, README, readme.as_bytes(), &mut files)?;

    files.sort_by(|left, right| left.path.cmp(&right.path));
    let head = audit::head(conn)?;
    let manifest = Manifest {
        format: FORMAT.to_owned(),
        version: VERSION,
        seller_id: seller_id.to_string(),
        from: from.to_iso_string(),
        to: to.to_iso_string(),
        currency: currency.as_str().to_owned(),
        exported_at: now.to_owned(),
        exported_by: actor.to_owned(),
        audit: ManifestAudit {
            head: head.map(|head| ManifestHead {
                seq: head.seq,
                hash: head.hash,
                at: head.at,
            }),
        },
        counts: ManifestCounts {
            documents: documents.len(),
            journal_entries,
            vat_returns,
            icp_months,
        },
        files: files.clone(),
    };
    fs::write(directory.join(MANIFEST), serde_json::to_string_pretty(&manifest)?)?;

    let bytes = verify(directory, &files)?;
    Ok(PackageReport {
        from,
        to,
        files: files.len(),
        bytes,
        documents: documents.len(),
        journal_entries,
        vat_returns,
        icp_months,
        head: manifest.audit.head.map(|head| head.hash),
    })
}

/// Every month the window touches, in order.
fn months_between(from: LocalDate, to: LocalDate) -> Vec<(i32, u8)> {
    let mut months = Vec::new();
    let mut year = from.as_date().year();
    let mut month = u8::from(from.as_date().month());
    let end_year = to.as_date().year();
    let end_month = u8::from(to.as_date().month());
    loop {
        months.push((year, month));
        if year == end_year && month == end_month {
            break;
        }
        if month == 12 {
            year += 1;
            month = 1;
        } else {
            month += 1;
        }
    }
    months
}

fn trial_csv(report: &crate::report::TrialBalance) -> Result<String, StoreError> {
    let mut out = String::from("code,name,kind,debit,credit,balance\n");
    for row in &report.rows {
        let debit = money(row.debit_micros, report.currency)?;
        let credit = money(row.credit_micros, report.currency)?;
        let balance = money(row.balance_micros(), report.currency)?;
        push_row(
            &mut out,
            &[
                row.code.as_str(),
                &row.name,
                row.kind.as_str(),
                &debit,
                &credit,
                &balance,
            ],
        );
    }
    let debit = money(report.debit_total_micros, report.currency)?;
    let credit = money(report.credit_total_micros, report.currency)?;
    let balance = money(report.difference_micros(), report.currency)?;
    push_row(&mut out, &["TOTAL", "", "", &debit, &credit, &balance]);
    Ok(out)
}

fn profit_csv(report: &crate::report::ProfitAndLoss) -> Result<String, StoreError> {
    let mut out = String::from("section,code,name,amount\n");
    for row in &report.revenue {
        push_account_row(&mut out, "revenue", row, report.currency)?;
    }
    for row in &report.expenses {
        push_account_row(&mut out, "expense", row, report.currency)?;
    }
    let result = money(report.result_micros(), report.currency)?;
    push_row(&mut out, &["result", "", "", &result]);
    Ok(out)
}

fn sheet_csv(report: &crate::report::BalanceSheet) -> Result<String, StoreError> {
    let mut out = String::from("section,code,name,amount\n");
    for row in &report.assets {
        push_account_row(&mut out, "asset", row, report.currency)?;
    }
    for row in &report.liabilities {
        push_account_row(&mut out, "liability", row, report.currency)?;
    }
    for row in &report.equity {
        push_account_row(&mut out, "equity", row, report.currency)?;
    }
    let result = money(report.result_micros, report.currency)?;
    push_row(&mut out, &["result", "", "Current result", &result]);
    let assets = money(report.assets_micros, report.currency)?;
    push_row(&mut out, &["assets_total", "", "", &assets]);
    let other = money(report.liabilities_and_equity_micros(), report.currency)?;
    push_row(&mut out, &["liabilities_and_equity_total", "", "", &other]);
    Ok(out)
}

fn push_account_row(
    out: &mut String,
    section: &str,
    row: &crate::report::AccountBalance,
    currency: Currency,
) -> Result<(), StoreError> {
    let amount = money(row.balance_micros(), currency)?;
    push_row(out, &[section, row.code.as_str(), &row.name, &amount]);
    Ok(())
}

fn journal_csv(
    conn: &Connection,
    seller_id: SellerId,
    from: LocalDate,
    to: LocalDate,
    currency: Currency,
) -> Result<String, StoreError> {
    let mut out = String::from(
        "journal,number,date,description,currency,base_currency,account,side,amount,base_amount,fx_rate,reverses,entry_hash\n",
    );
    let mut statement = conn.prepare(
        "SELECT e.journal_code, e.number, e.entry_date_local, e.description, e.currency,
                e.base_currency, l.account_code, l.side, l.amount_micros, l.base_amount_micros,
                l.fx_rate_micros, r.number, e.content_hash
         FROM journal_entry e
         JOIN fiscal_period p ON p.id = e.period_id
         JOIN journal_line l ON l.entry_id = e.id
         LEFT JOIN journal_entry r ON r.id = e.reverses_entry_id
         WHERE p.seller_id = ?1 AND e.entry_date_local >= ?2 AND e.entry_date_local <= ?3
         ORDER BY e.entry_date_local, e.journal_code, e.number, l.line_no",
    )?;
    let mut rows = statement.query(params![seller_id.to_string(), from.to_iso_string(), to.to_iso_string()])?;
    while let Some(row) = rows.next()? {
        let journal: String = row.get(0)?;
        let number: String = row.get(1)?;
        let date: String = row.get(2)?;
        let description: String = row.get(3)?;
        let entry_currency_raw: String = row.get(4)?;
        let entry_currency =
            Currency::parse(&entry_currency_raw).map_err(|error| StoreError::Data(error.to_string()))?;
        let base_currency = row.get::<_, Option<String>>(5)?;
        let account: String = row.get(6)?;
        let side: String = row.get(7)?;
        let amount = money(row.get(8)?, entry_currency)?;
        let base_amount = money(row.get(9)?, currency)?;
        let fx_rate = row.get::<_, Option<i64>>(10)?.map_or_else(String::new, |rate| {
            format!("{}.{:06}", rate / 1_000_000, rate % 1_000_000)
        });
        let reverses = row.get::<_, Option<String>>(11)?.unwrap_or_default();
        let hash: String = row.get(12)?;
        push_row(
            &mut out,
            &[
                &journal,
                &number,
                &date,
                &description,
                entry_currency.as_str(),
                base_currency.as_deref().unwrap_or(currency.as_str()),
                &account,
                &side,
                &amount,
                &base_amount,
                &fx_rate,
                &reverses,
                &hash,
            ],
        );
    }
    Ok(out)
}

fn open_items_csv(conn: &Connection, seller_id: SellerId, as_of: LocalDate) -> Result<String, StoreError> {
    let mut out = String::from("kind,number,party,currency,amount,outstanding,due,original_date,days_past_due\n");
    let mut statement = conn.prepare(
        "SELECT o.kind, d.number, COALESCE(c.legal_name, ''), o.currency, o.amount_micros,
                o.amount_micros + COALESCE(
                    (SELECT sum(a.amount_micros) FROM allocation a WHERE a.open_item_id = o.id), 0),
                o.due_date_local, o.original_date_local
         FROM open_item o
         JOIN document d ON d.id = o.source_id
         LEFT JOIN customer c ON c.id = o.party_id
         WHERE o.seller_id = ?1 AND o.original_date_local <= ?2
         ORDER BY o.kind, o.original_date_local, d.number",
    )?;
    let mut rows = statement.query(params![seller_id.to_string(), as_of.to_iso_string()])?;
    while let Some(row) = rows.next()? {
        let outstanding: i64 = row.get(5)?;
        if outstanding == 0 {
            continue;
        }
        let kind: String = row.get(0)?;
        let number: String = row.get(1)?;
        let party: String = row.get(2)?;
        let currency_raw: String = row.get(3)?;
        let item_currency = Currency::parse(&currency_raw).map_err(|error| StoreError::Data(error.to_string()))?;
        let amount = money(row.get(4)?, item_currency)?;
        let outstanding_text = money(outstanding, item_currency)?;
        let due = row.get::<_, Option<String>>(6)?;
        let original: String = row.get(7)?;
        let days = match due.as_deref() {
            Some(raw) => {
                let due = LocalDate::parse_iso(raw).map_err(|error| StoreError::Data(error.to_string()))?;
                (as_of.as_date().to_julian_day() - due.as_date().to_julian_day()).to_string()
            }
            None => String::new(),
        };
        push_row(
            &mut out,
            &[
                &kind,
                &number,
                &party,
                item_currency.as_str(),
                &amount,
                &outstanding_text,
                due.as_deref().unwrap_or(""),
                &original,
                &days,
            ],
        );
    }
    Ok(out)
}

fn return_json(prepared: &vat::PreparedReturn) -> Result<String, StoreError> {
    let computed = &prepared.computed;
    let mut lines = Vec::with_capacity(computed.lines.len());
    for line in &computed.lines {
        lines.push(serde_json::json!({
            "rubriek": line.rubriek.code(),
            "base": money(line.base_micros, computed.currency)?,
            "vat": money(line.vat_micros, computed.currency)?,
        }));
    }
    let basis: Vec<String> = computed.basis.iter().map(ToString::to_string).collect();
    let value = serde_json::json!({
        "id": &prepared.id,
        "prepared_at": &prepared.prepared_at,
        "basis_hash": &prepared.basis_hash,
        "currency": computed.currency.as_str(),
        "year": computed.year,
        "period": computed.period_no,
        "vat_due": money(computed.vat_due_micros(), computed.currency)?,
        "vat_deductible": money(computed.vat_deductible_micros(), computed.currency)?,
        "payable": money(computed.payable_micros(), computed.currency)?,
        "lines": lines,
        "basis": basis,
    });
    Ok(serde_json::to_string_pretty(&value)?)
}

fn documents_in(
    conn: &Connection,
    seller_id: SellerId,
    from: LocalDate,
    to: LocalDate,
) -> Result<Vec<DocumentRow>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT d.id, d.number, d.direction
         FROM document d
         WHERE d.owner_id = ?1 AND d.issue_date_local >= ?2 AND d.issue_date_local <= ?3
         ORDER BY d.issue_date_local, d.number, d.direction",
    )?;
    let mut rows = statement.query(params![seller_id.to_string(), from.to_iso_string(), to.to_iso_string()])?;
    let mut documents = Vec::new();
    while let Some(row) = rows.next()? {
        documents.push(DocumentRow {
            id: row.get(0)?,
            number: row.get(1)?,
            direction: row.get(2)?,
        });
    }
    Ok(documents)
}

fn write_documents(
    conn: &Connection,
    documents: &[DocumentRow],
    directory: &Path,
    files: &mut Vec<ManifestFile>,
) -> Result<(), StoreError> {
    for document in documents {
        // The latest bytes of each kind: re-rendering a document supersedes an
        // earlier artefact rather than adding a second file under the same name.
        let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        {
            let mut statement = conn.prepare(
                "SELECT a.kind, b.payload
                 FROM artefact a JOIN blob b ON b.id = a.blob_id
                 WHERE a.document_id = ?1
                 ORDER BY a.kind, a.rendered_at, a.id",
            )?;
            let mut rows = statement.query([&document.id])?;
            while let Some(row) = rows.next()? {
                artefacts.insert(row.get(0)?, row.get(1)?);
            }
        }
        for (kind, payload) in &artefacts {
            let path = format!(
                "documents/{}-{}-{}.{}",
                document.direction,
                safe_name(&document.number),
                safe_name(kind),
                extension(kind)
            );
            write_file(directory, &path, payload, files)?;
        }

        let payload: Option<(String, Option<Vec<u8>>)> = conn
            .query_row(
                "SELECT i.source_kind, b.payload
                 FROM import_record i LEFT JOIN blob b ON b.sha256 = i.payload_hash
                 WHERE i.document_id = ?1 LIMIT 1",
                [&document.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((kind, Some(bytes))) = payload {
            let path = format!(
                "documents/{}-{}-received.{}",
                document.direction,
                safe_name(&document.number),
                extension_for_source(&kind)
            );
            write_file(directory, &path, &bytes, files)?;
        }
    }
    Ok(())
}

fn readme(
    conn: &Connection,
    seller_id: SellerId,
    from: LocalDate,
    to: LocalDate,
    currency: Currency,
    files: usize,
) -> Result<String, StoreError> {
    let seller = crate::repo::load_seller(conn, seller_id)?;
    Ok(format!(
        "Bonk accountant package\n\
         =======================\n\
         \n\
         Books:    {} ({})\n\
         Window:   {} to {}\n\
         Currency: {}\n\
         Files:    {} (each listed with its SHA-256 in manifest.json)\n\
         \n\
         trial-balance.csv     accounts with movement in the window\n\
         profit-and-loss.csv   revenue and expenses in the window\n\
         balance-sheet.csv     assets, liabilities, equity and the current result at the end\n\
         journal.csv           one row per journal line, with the entry's content hash\n\
         open-items.csv        receivables and payables open when the package was written\n\
         vat/                  one JSON file per prepared VAT return in the window\n\
         icp/                  one CSV file per month with intra-EU supplies\n\
         documents/            the UBL, PDF and received bytes filed for each document\n\
         \n\
         The ledger reports are as of the window. The open-items file is the position\n\
         when the package was written, because the subledger does not store an as-of\n\
         settlement date. Check every file against manifest.json before relying on it.\n",
        seller.party.legal_name, seller.id, from, to, currency, files
    ))
}

fn journal_entry_count(
    conn: &Connection,
    seller_id: SellerId,
    from: LocalDate,
    to: LocalDate,
) -> Result<usize, StoreError> {
    let count: i64 = conn.query_row(
        "SELECT count(*)
         FROM journal_entry e
         JOIN fiscal_period p ON p.id = e.period_id
         WHERE p.seller_id = ?1 AND e.entry_date_local >= ?2 AND e.entry_date_local <= ?3",
        params![seller_id.to_string(), from.to_iso_string(), to.to_iso_string()],
        |row| row.get(0),
    )?;
    Ok(usize::try_from(count).unwrap_or(0))
}

fn money(micros: i64, currency: Currency) -> Result<String, StoreError> {
    Money::from_micros(micros, currency)
        .to_fixed_string(2)
        .map_err(|error| StoreError::Data(error.to_string()))
}

fn push_row(out: &mut String, fields: &[&str]) {
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_field(out, field);
    }
    out.push('\n');
}

/// Quotes a field only when it would otherwise break the columns.
fn push_field(out: &mut String, field: &str) {
    if !field.contains([',', '"', '\n', '\r']) {
        out.push_str(field);
        return;
    }
    out.push('"');
    for character in field.chars() {
        if character == '"' {
            out.push('"');
        }
        out.push(character);
    }
    out.push('"');
}

fn write_file(directory: &Path, relative: &str, bytes: &[u8], files: &mut Vec<ManifestFile>) -> Result<(), StoreError> {
    fs::write(directory.join(relative), bytes)?;
    files.push(ManifestFile {
        path: relative.to_owned(),
        sha256: audit::sha256_hex(bytes),
        size: bytes.len() as u64,
    });
    Ok(())
}

/// Re-reads what was written and checks it against the manifest entries.
fn verify(directory: &Path, files: &[ManifestFile]) -> Result<u64, StoreError> {
    let mut bytes = 0_u64;
    for file in files {
        let content = fs::read(directory.join(&file.path))?;
        if content.len() as u64 != file.size || audit::sha256_hex(&content) != file.sha256 {
            return Err(StoreError::Integrity(format!(
                "{} does not match its manifest entry",
                file.path
            )));
        }
        bytes += file.size;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_crossing_a_year_lists_every_month_once() {
        let months = months_between(
            LocalDate::parse_iso("2026-11-15").unwrap(),
            LocalDate::parse_iso("2027-02-02").unwrap(),
        );
        assert_eq!(months, vec![(2026, 11), (2026, 12), (2027, 1), (2027, 2)]);
    }

    #[test]
    fn a_csv_field_is_quoted_only_when_it_has_to_be() {
        let mut out = String::new();
        push_row(&mut out, &["plain", "a,b", "say \"hi\"", "line\nbreak"]);
        assert_eq!(out, "plain,\"a,b\",\"say \"\"hi\"\"\",\"line\nbreak\"\n");
    }
}
