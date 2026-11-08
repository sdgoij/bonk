//! S2 spike: a minimal UniFFI surface for the mobile hello world.
//!
//! The real boundary (`Archive`, `AppError`, the application services) replaces
//! this in M11.1. For S2 the only goal is to prove that Rust, UniFFI-generated
//! Kotlin, Compose Multiplatform, JNA, and R8 work together on a device.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::sync::{Arc, Mutex};
use std::time::Instant;

use application::{
    Archive as AppArchive, BackupExport, CustomerDto, CustomerImportDto, DocumentArtefact, DocumentDetail,
    DocumentEvent, DocumentHit, DocumentLine, DocumentSummary, DraftDto, DraftLineDto, DraftTotalsDto, FindingDto,
    ImportDto, IssueOutcome, PrecedingInvoiceDto, SellerDto,
};
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
    /// The archive has been closed.
    #[error("the archive is closed")]
    Closed,
    /// The passphrase did not decrypt the archive key.
    #[error("wrong passphrase")]
    WrongPassphrase,
    /// An archive already exists at the requested paths.
    #[error("an archive already exists at these paths")]
    ArchiveExists,
    /// The keystore has no recovery wrap.
    #[error("this archive has no recovery key")]
    NoRecoveryKey,
    /// The recovery key is malformed.
    #[error("the recovery key is not valid")]
    InvalidRecoveryKey,
    /// The keystore file is corrupt or from a newer version.
    #[error("the keystore is corrupt or unsupported")]
    InvalidKeystore,
    /// A filesystem operation failed.
    #[error("I/O error: {detail}")]
    Io { detail: String },
    /// The file handed to the archive is not an invoice it can read.
    #[error("import: {detail}")]
    Import { detail: String },
}

impl From<store::StoreError> for AppError {
    fn from(error: store::StoreError) -> Self {
        match error {
            store::StoreError::WrongPassphrase => Self::WrongPassphrase,
            store::StoreError::NoRecoveryKey => Self::NoRecoveryKey,
            store::StoreError::InvalidRecoveryKey => Self::InvalidRecoveryKey,
            store::StoreError::InvalidKeystore => Self::InvalidKeystore,
            store::StoreError::Io(error) => Self::Io {
                detail: error.to_string(),
            },
            other => Self::Store {
                detail: other.to_string(),
            },
        }
    }
}

impl From<application::import::ImportError> for AppError {
    fn from(error: application::import::ImportError) -> Self {
        Self::Import {
            detail: error.to_string(),
        }
    }
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
    /// True when the account has an entry, without reading it.
    ///
    /// This must not prompt for authentication: the app calls it to decide
    /// whether to offer the device-unlock route.
    fn has(&self, account: String) -> Result<bool, AppError>;
    /// Removes an entry; false when there was nothing to remove.
    fn delete(&self, account: String) -> Result<bool, AppError>;
}

/// What the dashboard shows for an opened archive.
#[derive(uniffi::Record)]
pub struct ArchiveSummary {
    /// Number of seller records.
    pub sellers: i64,
    /// Number of drafts.
    pub drafts: i64,
    /// Applied schema version.
    pub schema_version: i64,
}

/// The invoice a credit note corrects.
#[derive(uniffi::Record)]
pub struct PrecedingInvoiceRecord {
    /// Preceding invoice number.
    pub number: String,
    /// Preceding invoice issue date, ISO `YYYY-MM-DD`.
    pub issue_date: String,
}

/// One line of a draft.
#[derive(uniffi::Record)]
pub struct DraftLineRecord {
    /// Stable line identity.
    pub id: String,
    /// 1-based position.
    pub position: u32,
    /// Item name.
    pub name: String,
    /// Optional longer description.
    pub description: Option<String>,
    /// Quantity in micro-units.
    pub quantity_micros: i64,
    /// Three-character unit code.
    pub unit: String,
    /// Unit price in micro-units.
    pub unit_price_micros: i64,
    /// Price base quantity in micro-units.
    pub price_base_quantity_micros: i64,
    /// EN 16931 VAT category code.
    pub tax_category: String,
    /// VAT rate in basis points.
    pub tax_rate_bp: u16,
    /// Line net in micro-units.
    pub line_net_micros: i64,
    /// Exemption reason, when the category needs one.
    pub exemption_reason: Option<String>,
    /// Exemption reason code, when the category needs one.
    pub exemption_reason_code: Option<String>,
}

/// A draft as the editor holds it.
#[derive(uniffi::Record)]
pub struct DraftRecord {
    /// Draft identity.
    pub id: String,
    /// Seller identity.
    pub seller_id: String,
    /// Customer identity.
    pub customer_id: String,
    /// `380` for an invoice, `381` for a credit note.
    pub doc_type: String,
    /// ISO currency code.
    pub currency: String,
    /// Issue date, ISO `YYYY-MM-DD`.
    pub issue_date: String,
    /// Due date, ISO `YYYY-MM-DD`.
    pub due_date: Option<String>,
    /// Delivery date, ISO `YYYY-MM-DD`.
    pub delivery_date: Option<String>,
    /// Buyer reference.
    pub buyer_reference: Option<String>,
    /// Order reference.
    pub order_reference: Option<String>,
    /// Free-text note.
    pub note: Option<String>,
    /// Template file name, or `None` for the archive default.
    pub template: Option<String>,
    /// The invoice a credit note corrects.
    pub preceding_invoice: Option<PrecedingInvoiceRecord>,
    /// Lines.
    pub lines: Vec<DraftLineRecord>,
}

/// One validation finding, as data.
#[derive(uniffi::Record)]
pub struct FindingRecord {
    /// Stable code, such as `BR-16`.
    pub code: String,
    /// `blocking`, `warning`, or `info`.
    pub severity: String,
    /// EN 16931 business term, when one exists.
    pub bt: Option<String>,
    /// Field path the UI focuses.
    pub field: String,
    /// Human-readable fallback text. Named `detail` because a record field
    /// called `message` breaks the Kotlin generator.
    pub detail: String,
}

/// A customer as the editor holds it.
#[derive(uniffi::Record)]
pub struct CustomerRecord {
    /// Customer identity. Empty when the customer has not been saved yet.
    pub id: String,
    /// Legal name.
    pub name: String,
    /// Street and number.
    pub street: Option<String>,
    /// Postal code.
    pub postal: Option<String>,
    /// City.
    pub city: String,
    /// ISO 3166-1 alpha-2 country.
    pub country: String,
    /// VAT identifier.
    pub vat: Option<String>,
    /// Chamber of commerce number.
    pub kvk: Option<String>,
    /// Peppol endpoint, `EAS:value`.
    pub peppol: Option<String>,
    /// Contact person.
    pub contact: Option<String>,
    /// Contact email.
    pub email: Option<String>,
    /// Contact phone.
    pub phone: Option<String>,
}

impl From<CustomerDto> for CustomerRecord {
    fn from(dto: CustomerDto) -> Self {
        Self {
            id: dto.id,
            name: dto.name,
            street: dto.street,
            postal: dto.postal,
            city: dto.city,
            country: dto.country,
            vat: dto.vat,
            kvk: dto.kvk,
            peppol: dto.peppol,
            contact: dto.contact,
            email: dto.email,
            phone: dto.phone,
        }
    }
}

impl From<CustomerRecord> for CustomerDto {
    fn from(record: CustomerRecord) -> Self {
        Self {
            id: record.id,
            name: record.name,
            street: record.street,
            postal: record.postal,
            city: record.city,
            country: record.country,
            vat: record.vat,
            kvk: record.kvk,
            peppol: record.peppol,
            contact: record.contact,
            email: record.email,
            phone: record.phone,
        }
    }
}

/// The result of a customer CSV import.
#[derive(uniffi::Record)]
pub struct CustomerImportRecord {
    /// Customers written to the archive.
    pub imported: i64,
    /// Rows that were already on file.
    pub skipped: i64,
    /// Rows that could not be read, with the line and the reason.
    pub errors: Vec<String>,
}

impl From<CustomerImportDto> for CustomerImportRecord {
    fn from(dto: CustomerImportDto) -> Self {
        Self {
            imported: dto.imported,
            skipped: dto.skipped,
            errors: dto.errors,
        }
    }
}

/// What became of one imported invoice.
#[derive(uniffi::Record)]
pub struct ImportRecord {
    /// Number as the sender wrote it.
    pub number: String,
    /// Document identity in the archive.
    pub document_id: String,
    /// Supplier name, when the party table matched one.
    pub supplier: Option<String>,
    /// True when the archive already held this document, so nothing was added.
    pub duplicate: bool,
}

impl From<ImportDto> for ImportRecord {
    fn from(dto: ImportDto) -> Self {
        Self {
            number: dto.number,
            document_id: dto.document_id,
            supplier: dto.supplier,
            duplicate: dto.duplicate,
        }
    }
}

/// A verified copy written for export.
#[derive(uniffi::Record)]
pub struct BackupRecord {
    /// File the copy was written to.
    pub path: String,
    /// Size of the copy in bytes.
    pub bytes: i64,
}

impl From<BackupExport> for BackupRecord {
    fn from(dto: BackupExport) -> Self {
        Self {
            path: dto.path,
            bytes: dto.bytes,
        }
    }
}

/// A seller as the settings screen edits it.
#[derive(uniffi::Record)]
pub struct SellerRecord {
    /// Seller identity.
    pub id: String,
    /// Registered name.
    pub name: String,
    /// Street and number.
    pub street: Option<String>,
    /// Postal code.
    pub postal: Option<String>,
    /// City.
    pub city: String,
    /// ISO 3166-1 alpha-2 country.
    pub country: String,
    /// VAT identifier.
    pub vat: Option<String>,
    /// Chamber of commerce number.
    pub kvk: Option<String>,
    /// Peppol endpoint, `EAS:value`.
    pub peppol: Option<String>,
    /// Contact person.
    pub contact: Option<String>,
    /// Contact email.
    pub email: Option<String>,
    /// Contact phone.
    pub phone: Option<String>,
    /// ISO currency this seller issues in.
    pub default_currency: String,
    /// Locale this seller prefers.
    pub default_locale: String,
}

impl From<SellerDto> for SellerRecord {
    fn from(dto: SellerDto) -> Self {
        Self {
            id: dto.id,
            name: dto.name,
            street: dto.street,
            postal: dto.postal,
            city: dto.city,
            country: dto.country,
            vat: dto.vat,
            kvk: dto.kvk,
            peppol: dto.peppol,
            contact: dto.contact,
            email: dto.email,
            phone: dto.phone,
            default_currency: dto.default_currency,
            default_locale: dto.default_locale,
        }
    }
}

impl From<SellerRecord> for SellerDto {
    fn from(record: SellerRecord) -> Self {
        Self {
            id: record.id,
            name: record.name,
            street: record.street,
            postal: record.postal,
            city: record.city,
            country: record.country,
            vat: record.vat,
            kvk: record.kvk,
            peppol: record.peppol,
            contact: record.contact,
            email: record.email,
            phone: record.phone,
            default_currency: record.default_currency,
            default_locale: record.default_locale,
        }
    }
}

/// An issued document, as a list or a header shows it.
#[derive(uniffi::Record)]
pub struct DocumentSummaryRecord {
    /// Document identity.
    pub id: String,
    /// Human number.
    pub number: String,
    /// Issue date, ISO `YYYY-MM-DD`.
    pub issue_date: String,
    /// ISO currency code.
    pub currency: String,
    /// Net in micro-units.
    pub net_micros: i64,
    /// Tax in micro-units.
    pub tax_micros: i64,
    /// Gross in micro-units.
    pub gross_micros: i64,
    /// Stable status code.
    pub status: String,
}

impl From<DocumentSummary> for DocumentSummaryRecord {
    fn from(dto: DocumentSummary) -> Self {
        Self {
            id: dto.id,
            number: dto.number,
            issue_date: dto.issue_date,
            currency: dto.currency,
            net_micros: dto.net_micros,
            tax_micros: dto.tax_micros,
            gross_micros: dto.gross_micros,
            status: dto.status,
        }
    }
}

/// The result of an issue attempt: a document, or the findings that stopped it.
#[derive(uniffi::Record)]
pub struct IssueRecord {
    /// The issued document, when nothing blocked it.
    pub issued: Option<DocumentSummaryRecord>,
    /// Every finding, blocking or not.
    pub findings: Vec<FindingRecord>,
    /// SHA-256 of the stored UBL artefact, when a document was issued.
    pub ubl_sha256: Option<String>,
    /// SHA-256 of the stored PDF artefact, when one was rendered and filed.
    pub pdf_sha256: Option<String>,
    /// Why the PDF was not filed, when the document was issued anyway.
    pub pdf_error: Option<String>,
}

impl From<IssueOutcome> for IssueRecord {
    fn from(outcome: IssueOutcome) -> Self {
        Self {
            issued: outcome.issued.map(DocumentSummaryRecord::from),
            findings: outcome.findings.into_iter().map(FindingRecord::from).collect(),
            ubl_sha256: outcome.ubl_sha256,
            pdf_sha256: outcome.pdf_sha256,
            pdf_error: outcome.pdf_error,
        }
    }
}

/// One row in the document register.
#[derive(uniffi::Record)]
pub struct DocumentHitRecord {
    /// Document identity.
    pub id: String,
    /// Human number; empty for a draft that has none yet.
    pub number: String,
    /// `outbound` or `inbound`.
    pub direction: String,
    /// The party on the other side.
    pub party: String,
    /// Issue date, ISO `YYYY-MM-DD`.
    pub issue_date: String,
    /// ISO currency code.
    pub currency: String,
    /// Gross in micro-units.
    pub gross_micros: i64,
    /// Stable status code.
    pub status: String,
    /// True when the party on the other side is on file.
    pub matched: bool,
}

impl From<DocumentHit> for DocumentHitRecord {
    fn from(dto: DocumentHit) -> Self {
        Self {
            id: dto.id,
            number: dto.number,
            direction: dto.direction,
            party: dto.party,
            issue_date: dto.issue_date,
            currency: dto.currency,
            gross_micros: dto.gross_micros,
            status: dto.status,
            matched: dto.matched,
        }
    }
}

/// One line of a stored document.
#[derive(uniffi::Record)]
pub struct DocumentLineRecord {
    /// Position on the document.
    pub position: i64,
    /// Item name.
    pub name: String,
    /// Optional longer description.
    pub description: Option<String>,
    /// Quantity, trimmed (for example `3` or `1.5`).
    pub quantity: String,
    /// Unit of measure code.
    pub unit: String,
    /// Unit price in micro-units.
    pub unit_price_micros: i64,
    /// VAT category code.
    pub vat_category: String,
    /// VAT rate in basis points.
    pub vat_rate_bp: u16,
    /// Line net in micro-units, as recorded.
    pub line_net_micros: i64,
    /// Tax on the line in micro-units, as recorded.
    pub tax_micros: i64,
}

impl From<DocumentLine> for DocumentLineRecord {
    fn from(dto: DocumentLine) -> Self {
        Self {
            position: dto.position,
            name: dto.name,
            description: dto.description,
            quantity: dto.quantity,
            unit: dto.unit,
            unit_price_micros: dto.unit_price_micros,
            vat_category: dto.vat_category,
            vat_rate_bp: dto.vat_rate_bp,
            line_net_micros: dto.line_net_micros,
            tax_micros: dto.tax_micros,
        }
    }
}

/// One artefact filed under a document.
#[derive(uniffi::Record)]
pub struct DocumentArtefactRecord {
    /// `ubl`, `pdf`, and so on.
    pub kind: String,
    /// Syntax profile, when the kind has one.
    pub syntax_profile: Option<String>,
    /// SHA-256 of the bytes.
    pub bytes_hash: String,
    /// Size in bytes.
    pub size_bytes: i64,
    /// When it was rendered, RFC 3339.
    pub rendered_at: String,
    /// What rendered it.
    pub renderer_version: String,
}

impl From<DocumentArtefact> for DocumentArtefactRecord {
    fn from(dto: DocumentArtefact) -> Self {
        Self {
            kind: dto.kind,
            syntax_profile: dto.syntax_profile,
            bytes_hash: dto.bytes_hash,
            size_bytes: dto.size_bytes,
            rendered_at: dto.rendered_at,
            renderer_version: dto.renderer_version,
        }
    }
}

/// One status change on a document.
#[derive(uniffi::Record)]
pub struct DocumentEventRecord {
    /// Status code.
    pub kind: String,
    /// When it happened, RFC 3339.
    pub at: String,
    /// Who decided.
    pub actor: String,
    /// Optional detail, such as a payment reference.
    pub detail: Option<String>,
}

impl From<DocumentEvent> for DocumentEventRecord {
    fn from(dto: DocumentEvent) -> Self {
        Self {
            kind: dto.kind,
            at: dto.at,
            actor: dto.actor,
            detail: dto.detail,
        }
    }
}

/// A stored document, as the detail screen shows it.
#[derive(uniffi::Record)]
pub struct DocumentDetailRecord {
    /// Document identity.
    pub id: String,
    /// Human number.
    pub number: String,
    /// `outbound` or `inbound`.
    pub direction: String,
    /// UN/CEFACT type, `380` or `381`.
    pub doc_type: String,
    /// ISO currency code.
    pub currency: String,
    /// Issue date, ISO `YYYY-MM-DD`.
    pub issue_date: String,
    /// Due date, when the document has one.
    pub due_date: Option<String>,
    /// Delivery date, when the document has one.
    pub delivery_date: Option<String>,
    /// Buyer reference (BT-10).
    pub buyer_reference: Option<String>,
    /// Order reference (BT-13).
    pub order_reference: Option<String>,
    /// Free-text note (BT-22).
    pub note: Option<String>,
    /// Issuer as it was on the day.
    pub seller: String,
    /// Buyer as it was on the day.
    pub buyer: String,
    /// The customer a received document's supplier resolves to.
    pub supplier_id: Option<String>,
    /// Net in micro-units.
    pub net_micros: i64,
    /// Tax in micro-units.
    pub tax_micros: i64,
    /// Gross in micro-units.
    pub gross_micros: i64,
    /// Stable status code.
    pub status: String,
    /// When the archive recorded it, RFC 3339.
    pub recorded_at: String,
    /// Lines, in position order.
    pub lines: Vec<DocumentLineRecord>,
    /// Artefacts with their hashes, oldest first.
    pub artefacts: Vec<DocumentArtefactRecord>,
    /// Status events, oldest first.
    pub events: Vec<DocumentEventRecord>,
}

impl From<DocumentDetail> for DocumentDetailRecord {
    fn from(dto: DocumentDetail) -> Self {
        Self {
            id: dto.id,
            number: dto.number,
            direction: dto.direction,
            doc_type: dto.doc_type,
            currency: dto.currency,
            issue_date: dto.issue_date,
            due_date: dto.due_date,
            delivery_date: dto.delivery_date,
            buyer_reference: dto.buyer_reference,
            order_reference: dto.order_reference,
            note: dto.note,
            seller: dto.seller,
            buyer: dto.buyer,
            supplier_id: dto.supplier_id,
            net_micros: dto.net_micros,
            tax_micros: dto.tax_micros,
            gross_micros: dto.gross_micros,
            status: dto.status,
            recorded_at: dto.recorded_at,
            lines: dto.lines.into_iter().map(DocumentLineRecord::from).collect(),
            artefacts: dto.artefacts.into_iter().map(DocumentArtefactRecord::from).collect(),
            events: dto.events.into_iter().map(DocumentEventRecord::from).collect(),
        }
    }
}

impl From<PrecedingInvoiceDto> for PrecedingInvoiceRecord {
    fn from(dto: PrecedingInvoiceDto) -> Self {
        Self {
            number: dto.number,
            issue_date: dto.issue_date,
        }
    }
}

impl From<PrecedingInvoiceRecord> for PrecedingInvoiceDto {
    fn from(record: PrecedingInvoiceRecord) -> Self {
        Self {
            number: record.number,
            issue_date: record.issue_date,
        }
    }
}

impl From<DraftLineDto> for DraftLineRecord {
    fn from(dto: DraftLineDto) -> Self {
        Self {
            id: dto.id,
            position: dto.position,
            name: dto.name,
            description: dto.description,
            quantity_micros: dto.quantity_micros,
            unit: dto.unit,
            unit_price_micros: dto.unit_price_micros,
            price_base_quantity_micros: dto.price_base_quantity_micros,
            tax_category: dto.tax_category,
            tax_rate_bp: dto.tax_rate_bp,
            line_net_micros: dto.line_net_micros,
            exemption_reason: dto.exemption_reason,
            exemption_reason_code: dto.exemption_reason_code,
        }
    }
}

impl From<DraftLineRecord> for DraftLineDto {
    fn from(record: DraftLineRecord) -> Self {
        Self {
            id: record.id,
            position: record.position,
            name: record.name,
            description: record.description,
            quantity_micros: record.quantity_micros,
            unit: record.unit,
            unit_price_micros: record.unit_price_micros,
            price_base_quantity_micros: record.price_base_quantity_micros,
            tax_category: record.tax_category,
            tax_rate_bp: record.tax_rate_bp,
            line_net_micros: record.line_net_micros,
            exemption_reason: record.exemption_reason,
            exemption_reason_code: record.exemption_reason_code,
        }
    }
}

impl From<DraftDto> for DraftRecord {
    fn from(dto: DraftDto) -> Self {
        Self {
            id: dto.id,
            seller_id: dto.seller_id,
            customer_id: dto.customer_id,
            doc_type: dto.doc_type,
            currency: dto.currency,
            issue_date: dto.issue_date,
            due_date: dto.due_date,
            delivery_date: dto.delivery_date,
            buyer_reference: dto.buyer_reference,
            order_reference: dto.order_reference,
            note: dto.note,
            template: dto.template,
            preceding_invoice: dto.preceding_invoice.map(PrecedingInvoiceRecord::from),
            lines: dto.lines.into_iter().map(DraftLineRecord::from).collect(),
        }
    }
}

impl From<DraftRecord> for DraftDto {
    fn from(record: DraftRecord) -> Self {
        Self {
            id: record.id,
            seller_id: record.seller_id,
            customer_id: record.customer_id,
            doc_type: record.doc_type,
            currency: record.currency,
            issue_date: record.issue_date,
            due_date: record.due_date,
            delivery_date: record.delivery_date,
            buyer_reference: record.buyer_reference,
            order_reference: record.order_reference,
            note: record.note,
            template: record.template,
            preceding_invoice: record.preceding_invoice.map(PrecedingInvoiceDto::from),
            lines: record.lines.into_iter().map(DraftLineDto::from).collect(),
        }
    }
}

impl From<FindingDto> for FindingRecord {
    fn from(dto: FindingDto) -> Self {
        Self {
            code: dto.code,
            severity: dto.severity,
            bt: dto.bt,
            field: dto.field,
            detail: dto.message,
        }
    }
}

/// Totals for a draft preview, in micro-units.
#[derive(uniffi::Record)]
pub struct DraftTotalsRecord {
    /// Sum of line nets (BT-106).
    pub net_micros: i64,
    /// Sum of tax per category (BT-110).
    pub tax_micros: i64,
    /// Net plus tax (BT-112).
    pub gross_micros: i64,
}

impl From<DraftTotalsDto> for DraftTotalsRecord {
    fn from(dto: DraftTotalsDto) -> Self {
        Self {
            net_micros: dto.net_micros,
            tax_micros: dto.tax_micros,
            gross_micros: dto.gross_micros,
        }
    }
}

/// An unlocked archive, owned by the caller and closed explicitly.
///
/// Every method takes `&self`; the mutex serialises access, matching SQLite's
/// single-writer model. Hosts must call these off the UI thread.
#[derive(uniffi::Object)]
pub struct Archive {
    inner: Mutex<Option<AppArchive>>,
    /// The replacement recovery key from a reset, handed to the caller once.
    replacement_key: Mutex<Option<String>>,
}

#[uniffi::export]
impl Archive {
    /// Creates a new encrypted archive. Refuses to overwrite either file.
    #[uniffi::constructor]
    pub fn create(database_path: String, keystore_path: String, passphrase: String) -> Result<Arc<Self>, AppError> {
        if std::path::Path::new(&database_path).exists() || std::path::Path::new(&keystore_path).exists() {
            return Err(AppError::ArchiveExists);
        }
        let archive = AppArchive::create(database_path, keystore_path, &passphrase)?;
        Ok(Arc::new(Self {
            inner: Mutex::new(Some(archive)),
            replacement_key: Mutex::new(None),
        }))
    }

    /// Opens an existing archive with a passphrase.
    #[uniffi::constructor]
    pub fn open(database_path: String, keystore_path: String, passphrase: String) -> Result<Arc<Self>, AppError> {
        let archive = AppArchive::unlock(database_path, keystore_path, &passphrase)?;
        Ok(Arc::new(Self {
            inner: Mutex::new(Some(archive)),
            replacement_key: Mutex::new(None),
        }))
    }

    /// Opens an existing archive with the key the device kept.
    #[uniffi::constructor]
    pub fn open_with_device(
        database_path: String,
        keystore_path: String,
        device: Box<dyn DeviceSecretStore>,
    ) -> Result<Arc<Self>, AppError> {
        let adapter = DeviceAdapter(device);
        let archive = AppArchive::unlock_with_device(database_path, keystore_path, &adapter)?;
        Ok(Arc::new(Self {
            inner: Mutex::new(Some(archive)),
            replacement_key: Mutex::new(None),
        }))
    }

    /// Replaces a forgotten passphrase with the recovery key.
    ///
    /// The new archive is returned and the replacement recovery key is kept for
    /// [`Self::take_replacement_key`], so the caller can show it once before the
    /// user continues.
    #[uniffi::constructor]
    pub fn reset_passphrase(
        database_path: String,
        keystore_path: String,
        recovery_key: String,
        new_passphrase: String,
    ) -> Result<Arc<Self>, AppError> {
        let (archive, replacement) =
            AppArchive::reset_passphrase(database_path, keystore_path, &recovery_key, &new_passphrase)?;
        Ok(Arc::new(Self {
            inner: Mutex::new(Some(archive)),
            replacement_key: Mutex::new(Some(replacement)),
        }))
    }

    /// Dashboard data for the open archive.
    pub fn summary(&self) -> Result<ArchiveSummary, AppError> {
        let guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_ref().ok_or(AppError::Closed)?;
        let summary = archive.summary()?;
        Ok(ArchiveSummary {
            sellers: summary.sellers,
            drafts: summary.drafts,
            schema_version: summary.schema_version,
        })
    }

    /// Keeps the archive key in the platform store for later device unlocks.
    pub fn remember_device(&self, passphrase: String, device: Box<dyn DeviceSecretStore>) -> Result<(), AppError> {
        let adapter = DeviceAdapter(device);
        let guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_ref().ok_or(AppError::Closed)?;
        archive.remember_device(&passphrase, &adapter)?;
        Ok(())
    }

    /// Removes the device key; false when there was none.
    pub fn forget_device(&self, device: Box<dyn DeviceSecretStore>) -> Result<bool, AppError> {
        let adapter = DeviceAdapter(device);
        let guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_ref().ok_or(AppError::Closed)?;
        Ok(archive.forget_device(&adapter)?)
    }

    /// Records that the kept device key was revoked by an enrolment change.
    ///
    /// Call this after the next successful unlock, because the audit chain needs
    /// the open archive.
    pub fn record_device_revocation(&self) -> Result<(), AppError> {
        let guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_ref().ok_or(AppError::Closed)?;
        archive.record_device_revocation()?;
        Ok(())
    }

    /// Lists drafts, most recently updated first.
    pub fn list_drafts(&self) -> Result<Vec<DraftRecord>, AppError> {
        let guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_ref().ok_or(AppError::Closed)?;
        Ok(archive.list_drafts()?.into_iter().map(DraftRecord::from).collect())
    }

    /// Loads a draft by identity.
    pub fn load_draft(&self, id: String) -> Result<DraftRecord, AppError> {
        let guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_ref().ok_or(AppError::Closed)?;
        Ok(archive.load_draft(&id)?.into())
    }

    /// Creates or updates a draft and returns the stored shape.
    pub fn save_draft(&self, draft: DraftRecord) -> Result<DraftRecord, AppError> {
        let guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_ref().ok_or(AppError::Closed)?;
        Ok(archive.save_draft(&DraftDto::from(draft))?.into())
    }

    /// Validates a draft against its seller and customer.
    pub fn validate_draft(&self, draft: DraftRecord) -> Result<Vec<FindingRecord>, AppError> {
        let guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_ref().ok_or(AppError::Closed)?;
        Ok(archive
            .validate_draft(&DraftDto::from(draft))?
            .into_iter()
            .map(FindingRecord::from)
            .collect())
    }

    /// Records a reason for accepting a non-blocking finding.
    pub fn suppress_finding(
        &self,
        draft_id: String,
        code: String,
        field: String,
        reason: String,
    ) -> Result<(), AppError> {
        let guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_ref().ok_or(AppError::Closed)?;
        archive.suppress_finding(&draft_id, &code, &field, &reason)?;
        Ok(())
    }

    /// Totals for the editor preview.
    pub fn draft_totals(&self, draft: DraftRecord) -> Result<DraftTotalsRecord, AppError> {
        let guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_ref().ok_or(AppError::Closed)?;
        Ok(archive.draft_totals(&DraftDto::from(draft))?.into())
    }

    /// Issues a draft, or returns the findings that stopped it.
    ///
    /// The draft is saved first, so an edit still in the editor is what gets
    /// issued. The mobile shell is English and uses the built-in layout until
    /// the settings screen exists.
    pub fn issue_draft(&self, draft: DraftRecord) -> Result<IssueRecord, AppError> {
        let mut guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_mut().ok_or(AppError::Closed)?;
        let outcome = archive.issue(
            &DraftDto::from(draft),
            application::DocumentLanguage::English,
            None,
            "mobile",
        )?;
        Ok(outcome.into())
    }

    /// Documents on one side of the books, best match first; an empty query
    /// lists newest first.
    ///
    /// `direction` is `outbound` for invoices we issued or `inbound` for the ones
    /// we received. A status code narrows the list to the documents that stand
    /// there.
    pub fn documents(
        &self,
        query: Option<String>,
        status: Option<String>,
        direction: String,
    ) -> Result<Vec<DocumentHitRecord>, AppError> {
        let guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_ref().ok_or(AppError::Closed)?;
        let status = status
            .as_deref()
            .filter(|code| !code.trim().is_empty())
            .map(|code| {
                store::DocumentStatus::parse(code).ok_or_else(|| AppError::Store {
                    detail: format!("unknown status {code}"),
                })
            })
            .transpose()?;
        let direction = store::Direction::parse(&direction).ok_or_else(|| AppError::Store {
            detail: format!("unknown direction {direction}"),
        })?;
        Ok(archive
            .documents(query.as_deref(), status, direction)?
            .into_iter()
            .map(DocumentHitRecord::from)
            .collect())
    }

    /// Records the supplier of a received document against a customer on file.
    pub fn match_supplier(&self, document_id: String, customer_id: String) -> Result<(), AppError> {
        let guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_ref().ok_or(AppError::Closed)?;
        archive.match_supplier(&document_id, &customer_id, "mobile")?;
        Ok(())
    }

    /// Moves a received document to a status it can hold, and returns that status.
    pub fn advance_status(&self, document_id: String, target: String) -> Result<String, AppError> {
        let guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_ref().ok_or(AppError::Closed)?;
        Ok(archive.advance_status(&document_id, &target, "mobile")?)
    }

    /// One stored document for the detail screen.
    pub fn document(&self, id: String) -> Result<DocumentDetailRecord, AppError> {
        let guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_ref().ok_or(AppError::Closed)?;
        Ok(archive.document(&id)?.into())
    }

    /// The bytes of one artefact, verified against the hash recorded for it.
    pub fn document_artefact(&self, id: String, kind: String) -> Result<Vec<u8>, AppError> {
        let guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_ref().ok_or(AppError::Closed)?;
        Ok(archive.document_artefact(&id, &kind)?)
    }

    /// A new invoice draft for a customer, dated today and saved.
    pub fn new_draft(&self, customer_id: String) -> Result<DraftRecord, AppError> {
        let guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_ref().ok_or(AppError::Closed)?;
        Ok(archive.new_draft(&customer_id)?.into())
    }

    /// Lists customers, ordered by display name.
    pub fn list_customers(&self) -> Result<Vec<CustomerRecord>, AppError> {
        let guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_ref().ok_or(AppError::Closed)?;
        Ok(archive
            .list_customers()?
            .into_iter()
            .map(CustomerRecord::from)
            .collect())
    }

    /// Creates a customer when the id is empty, otherwise updates one.
    pub fn save_customer(&self, customer: CustomerRecord) -> Result<CustomerRecord, AppError> {
        let guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_ref().ok_or(AppError::Closed)?;
        Ok(archive.save_customer(&CustomerDto::from(customer))?.into())
    }

    /// The seller this archive issues as.
    pub fn seller(&self) -> Result<SellerRecord, AppError> {
        let guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_ref().ok_or(AppError::Closed)?;
        Ok(archive.seller()?.into())
    }

    /// Updates the seller's party data.
    pub fn save_seller(&self, seller: SellerRecord) -> Result<SellerRecord, AppError> {
        let guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_ref().ok_or(AppError::Closed)?;
        Ok(archive.save_seller(&SellerDto::from(seller))?.into())
    }

    /// Exports the customer table as CSV.
    pub fn customers_csv(&self) -> Result<String, AppError> {
        let guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_ref().ok_or(AppError::Closed)?;
        Ok(archive.customers_csv()?)
    }

    /// Imports customers from CSV.
    pub fn import_customers_csv(&self, text: String) -> Result<CustomerImportRecord, AppError> {
        let guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_ref().ok_or(AppError::Closed)?;
        Ok(archive.import_customers_csv(&text)?.into())
    }

    /// Imports a received invoice from bytes, as a file picker or a share
    /// hand-off delivers them.
    ///
    /// The name is provenance for the record, not a path. A document the archive
    /// already holds comes back with `duplicate` set rather than being written
    /// twice.
    pub fn import_document(&self, name: Option<String>, bytes: Vec<u8>) -> Result<ImportRecord, AppError> {
        let guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_ref().ok_or(AppError::Closed)?;
        Ok(archive.import_document(name.as_deref(), &bytes)?.into())
    }

    /// Drops the archive and zeroizes the data key. Later calls return `Closed`.
    pub fn lock(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            *guard = None;
        }
    }

    /// Adds or replaces the recovery key, returning the secret to show once.
    pub fn add_recovery_key(&self, passphrase: String) -> Result<String, AppError> {
        let guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_ref().ok_or(AppError::Closed)?;
        Ok(archive.add_recovery_key(&passphrase)?)
    }

    /// Returns and clears the replacement recovery key from a reset.
    pub fn take_replacement_key(&self) -> Option<String> {
        self.replacement_key.lock().ok()?.take()
    }

    /// The database file this archive was opened from.
    pub fn database_path(&self) -> Result<String, AppError> {
        let guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_ref().ok_or(AppError::Closed)?;
        Ok(archive.database_path().display().to_string())
    }

    /// Writes a verified copy of the archive into `directory` and names it.
    ///
    /// The copy is checked here, before the host streams it anywhere; the host
    /// deletes it once the user's destination holds it.
    pub fn write_backup(&self, directory: String) -> Result<BackupRecord, AppError> {
        let guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_ref().ok_or(AppError::Closed)?;
        Ok(archive.write_backup(directory)?.into())
    }

    /// The keystore bytes, for a recovery kit.
    pub fn recovery_key(&self) -> Result<Vec<u8>, AppError> {
        let guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_ref().ok_or(AppError::Closed)?;
        Ok(archive.recovery_key()?)
    }

    /// The note that must travel with a recovery key.
    pub fn recovery_note(&self) -> Result<String, AppError> {
        let guard = self.inner.lock().map_err(|_| AppError::Closed)?;
        let archive = guard.as_ref().ok_or(AppError::Closed)?;
        Ok(archive.recovery_note())
    }
}

/// True when this device holds a key for the archive at this keystore path.
#[uniffi::export]
pub fn device_ready(keystore_path: String, device: Box<dyn DeviceSecretStore>) -> bool {
    AppArchive::device_ready(keystore_path, &DeviceAdapter(device))
}

/// True when both files of an archive exist at these paths.
#[uniffi::export]
pub fn archive_exists(database_path: String, keystore_path: String) -> bool {
    std::path::Path::new(&database_path).exists() && std::path::Path::new(&keystore_path).exists()
}

/// Replaces the archive with a staged pair, after checking it opens.
///
/// The caller stages the two picked files in app-private storage and closes any
/// open handle first. The restored archive is not opened here; the host opens it
/// with the passphrase that was just proved against the staged copy.
#[uniffi::export]
pub fn restore_archive(
    database_path: String,
    keystore_path: String,
    staged_database: String,
    staged_keystore: String,
    passphrase: String,
) -> Result<ArchiveSummary, AppError> {
    let summary = application::restore(
        database_path,
        keystore_path,
        staged_database,
        staged_keystore,
        &passphrase,
    )?;
    Ok(ArchiveSummary {
        sellers: summary.sellers,
        drafts: summary.drafts,
        schema_version: summary.schema_version,
    })
}

/// Adapts the host callback to the store's `DeviceStore` port.
struct DeviceAdapter(Box<dyn DeviceSecretStore>);

impl store::DeviceStore for DeviceAdapter {
    fn put(&self, account: &str, secret: &[u8]) -> Result<(), store::StoreError> {
        self.0.put(account.to_owned(), secret.to_vec()).map_err(device_error)
    }

    fn get(&self, account: &str) -> Result<Option<Vec<u8>>, store::StoreError> {
        self.0.get(account.to_owned()).map_err(device_error)
    }

    fn has(&self, account: &str) -> Result<bool, store::StoreError> {
        self.0.has(account.to_owned()).map_err(device_error)
    }

    fn delete(&self, account: &str) -> Result<bool, store::StoreError> {
        self.0.delete(account.to_owned()).map_err(device_error)
    }
}

fn device_error(error: AppError) -> store::StoreError {
    store::StoreError::Keychain(error.to_string())
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

        fn has(&self, account: String) -> Result<bool, AppError> {
            Ok(self
                .entries
                .lock()
                .expect("the test store is not poisoned")
                .contains_key(&account))
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
    fn device_ready_does_not_read_the_secret() {
        struct Unreadable;
        impl DeviceSecretStore for Unreadable {
            fn put(&self, _: String, _: Vec<u8>) -> Result<(), AppError> {
                Ok(())
            }
            fn get(&self, _: String) -> Result<Option<Vec<u8>>, AppError> {
                panic!("a readiness check must not read the secret")
            }
            fn has(&self, _: String) -> Result<bool, AppError> {
                Ok(true)
            }
            fn delete(&self, _: String) -> Result<bool, AppError> {
                Ok(false)
            }
        }

        assert!(device_ready("/tmp/bonk/keystore.bin".to_owned(), Box::new(Unreadable)));
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
    fn an_archive_opens_summarises_and_closes() {
        let directory = tempfile::TempDir::new().expect("a temporary directory");
        let database = directory.path().join("bonk.db").display().to_string();
        let keystore = directory.path().join("keystore.bin").display().to_string();

        assert!(!archive_exists(database.clone(), keystore.clone()));
        let archive =
            Archive::create(database.clone(), keystore.clone(), "passphrase".to_owned()).expect("a new archive");
        assert!(archive_exists(database.clone(), keystore.clone()));
        assert_eq!(archive.summary().expect("a summary").schema_version, 17);
        assert_eq!(archive.database_path().expect("a path"), database);

        archive.lock();
        assert!(matches!(archive.summary(), Err(AppError::Closed)));

        assert!(matches!(
            Archive::open(database.clone(), keystore.clone(), "wrong".to_owned()),
            Err(AppError::WrongPassphrase)
        ));
        assert!(matches!(
            Archive::create(database.clone(), keystore.clone(), "passphrase".to_owned()),
            Err(AppError::ArchiveExists)
        ));

        let reopened = Archive::open(database, keystore, "passphrase".to_owned()).expect("an existing archive");
        assert_eq!(reopened.summary().expect("a summary").sellers, 0);
    }

    #[test]
    fn a_backup_and_a_recovery_key_come_out_of_the_archive() {
        let directory = tempfile::TempDir::new().expect("a temporary directory");
        let database = directory.path().join("bonk.db").display().to_string();
        let keystore = directory.path().join("keystore.bin").display().to_string();
        let archive = Archive::create(database, keystore, "passphrase".to_owned()).expect("a new archive");

        let cache = directory.path().join("export").display().to_string();
        let backup = archive.write_backup(cache).expect("a backup");
        assert!(backup.bytes > 0);
        assert!(std::path::Path::new(&backup.path).exists());

        let key = archive.recovery_key().expect("the keystore bytes");
        assert!(!key.is_empty());
        let note = archive.recovery_note().expect("the note");
        assert!(note.contains("another medium"), "{note}");
    }

    #[test]
    fn a_staged_pair_replaces_the_archive() {
        let source = tempfile::TempDir::new().expect("a temporary directory");
        let source_database = source.path().join("bonk.db").display().to_string();
        let source_keystore = source.path().join("keystore.bin").display().to_string();
        let opened = Archive::create(
            source_database.clone(),
            source_keystore.clone(),
            "passphrase".to_owned(),
        )
        .expect("a source archive");
        opened.seller().expect("the placeholder seller");
        opened.lock();

        let target = tempfile::TempDir::new().expect("a temporary directory");
        let database = target.path().join("bonk.db").display().to_string();
        let keystore = target.path().join("keystore.bin").display().to_string();
        let empty =
            Archive::create(database.clone(), keystore.clone(), "passphrase".to_owned()).expect("a target archive");
        empty.lock();

        let staging = tempfile::TempDir::new().expect("a staging directory");
        let staged_database = staging.path().join("bonk.db");
        let staged_keystore = staging.path().join("keystore.bin");
        std::fs::copy(&source_database, &staged_database).expect("a copy of the database");
        std::fs::copy(&source_keystore, &staged_keystore).expect("a copy of the key");

        assert!(
            restore_archive(
                database.clone(),
                keystore.clone(),
                staged_database.display().to_string(),
                staged_keystore.display().to_string(),
                "wrong".to_owned(),
            )
            .is_err()
        );

        let summary = restore_archive(
            database.clone(),
            keystore.clone(),
            staged_database.display().to_string(),
            staged_keystore.display().to_string(),
            "passphrase".to_owned(),
        )
        .expect("a restore");
        assert_eq!(summary.sellers, 1);

        let reopened = Archive::open(database, keystore, "passphrase".to_owned()).expect("the restored archive");
        assert_eq!(reopened.summary().expect("a summary").sellers, 1);
    }

    #[test]
    fn a_draft_round_trips_through_the_ffi() {
        let directory = tempfile::TempDir::new().expect("a temporary directory");
        let database = directory.path().join("bonk.db").display().to_string();
        let keystore = directory.path().join("keystore.bin").display().to_string();
        let archive = Archive::create(database, keystore, "passphrase".to_owned()).expect("a new archive");

        let now = "2026-11-08T12:00:00Z";
        let seller = store::SellerRecord {
            id: crux::SellerId::new(),
            party: Party::new("Seller BV", Address::new("Delft", CountryCode::NL)),
            default_currency: Currency::EUR,
            default_locale: "en".to_owned(),
        };
        let customer = store::CustomerRecord {
            id: crux::CustomerId::new(),
            party: Party::new("Buyer BV", Address::new("Utrecht", CountryCode::NL)),
        };
        {
            let mut guard = archive.inner.lock().expect("the archive lock");
            let app = guard.as_mut().expect("an open archive");
            store::insert_seller(app.database().connection(), &seller, now).expect("a seller");
            store::insert_customer(app.database().connection(), &customer, now).expect("a customer");
        }

        let draft = DraftRecord {
            id: crux::DocumentId::new().to_string(),
            seller_id: seller.id.to_string(),
            customer_id: customer.id.to_string(),
            doc_type: "380".to_owned(),
            currency: "EUR".to_owned(),
            issue_date: "2026-11-08".to_owned(),
            due_date: None,
            delivery_date: None,
            buyer_reference: None,
            order_reference: None,
            note: None,
            template: None,
            preceding_invoice: None,
            lines: vec![DraftLineRecord {
                id: crux::LineId::new().to_string(),
                position: 1,
                name: "Consultancy".to_owned(),
                description: None,
                quantity_micros: 1_000_000,
                unit: "HUR".to_owned(),
                unit_price_micros: 100_000_000,
                price_base_quantity_micros: 1_000_000,
                tax_category: "S".to_owned(),
                tax_rate_bp: 2100,
                line_net_micros: 100_000_000,
                exemption_reason: None,
                exemption_reason_code: None,
            }],
        };
        let draft_id = draft.id.clone();

        let saved = archive.save_draft(draft).expect("a saved draft");
        assert_eq!(saved.id, draft_id);
        assert_eq!(archive.list_drafts().expect("drafts").len(), 1);
        let loaded = archive.load_draft(draft_id).expect("a loaded draft");
        assert_eq!(loaded.id, saved.id);
        assert!(archive.validate_draft(loaded).is_ok());
    }

    #[test]
    fn a_customer_round_trips_through_the_ffi() {
        let directory = tempfile::TempDir::new().expect("a temporary directory");
        let database = directory.path().join("bonk.db").display().to_string();
        let keystore = directory.path().join("keystore.bin").display().to_string();
        let archive = Archive::create(database, keystore, "passphrase".to_owned()).expect("a new archive");

        let customer = CustomerRecord {
            id: String::new(),
            name: "Buyer BV".to_owned(),
            street: Some("Main 1".to_owned()),
            postal: Some("1000 AA".to_owned()),
            city: "Utrecht".to_owned(),
            country: "NL".to_owned(),
            vat: Some("nl123456788b01".to_owned()),
            kvk: None,
            peppol: None,
            contact: None,
            email: None,
            phone: None,
        };
        let saved = archive.save_customer(customer).expect("a saved customer");
        assert!(!saved.id.is_empty(), "a new customer gets an id");
        assert_eq!(saved.vat.as_deref(), Some("NL123456788B01"));
        assert_eq!(archive.list_customers().expect("customers").len(), 1);

        let saved = archive
            .save_customer(CustomerRecord {
                city: "Delft".to_owned(),
                ..saved
            })
            .expect("an updated customer");
        assert_eq!(saved.city, "Delft");
        assert_eq!(archive.list_customers().expect("customers").len(), 1);
    }

    #[test]
    fn the_seller_round_trips_through_the_ffi() {
        let directory = tempfile::TempDir::new().expect("a temporary directory");
        let database = directory.path().join("bonk.db").display().to_string();
        let keystore = directory.path().join("keystore.bin").display().to_string();
        let archive = Archive::create(database, keystore, "passphrase".to_owned()).expect("a new archive");

        // A fresh archive answers with the placeholder, so the screen opens on
        // something rather than on an empty form.
        let seller = archive.seller().expect("the placeholder seller");
        assert_eq!(seller.name, "My company");
        assert!(!seller.id.is_empty());

        let saved = archive
            .save_seller(SellerRecord {
                street: Some("Main 1".to_owned()),
                postal: Some("2611 AA".to_owned()),
                city: "Delft".to_owned(),
                vat: Some("nl123456782b01".to_owned()),
                peppol: Some("0088:1234567890123".to_owned()),
                ..seller
            })
            .expect("a saved seller");
        assert_eq!(saved.name, "My company");
        assert_eq!(saved.city, "Delft");
        assert_eq!(saved.vat.as_deref(), Some("NL123456782B01"));
        assert_eq!(archive.seller().expect("the seller").city, "Delft");
    }

    #[test]
    fn an_issued_document_crosses_the_ffi() {
        let directory = tempfile::TempDir::new().expect("a temporary directory");
        let database = directory.path().join("bonk.db").display().to_string();
        let keystore = directory.path().join("keystore.bin").display().to_string();
        let archive = Archive::create(database, keystore, "passphrase".to_owned()).expect("a new archive");

        // Parties the Dutch and Peppol profiles accept, so issuing gets past them.
        let now = "2026-11-08T12:00:00Z";
        let mut seller_party = Party::new("Seller BV", Address::new("Delft", CountryCode::NL));
        seller_party.address.street = Some("Main 1".to_owned());
        seller_party.address.postal_code = Some("2611 AA".to_owned());
        seller_party.vat_id = Some("NL123456782B01".to_owned());
        seller_party.peppol_id = Some("0088:1234567890123".to_owned());
        let seller = store::SellerRecord {
            id: crux::SellerId::new(),
            party: seller_party,
            default_currency: Currency::EUR,
            default_locale: "en".to_owned(),
        };
        let mut buyer_party = Party::new("Buyer BV", Address::new("Utrecht", CountryCode::NL));
        buyer_party.peppol_id = Some("0088:9876543210987".to_owned());
        let customer = store::CustomerRecord {
            id: crux::CustomerId::new(),
            party: buyer_party,
        };
        {
            let mut guard = archive.inner.lock().expect("the archive lock");
            let app = guard.as_mut().expect("an open archive");
            store::insert_seller(app.database().connection(), &seller, now).expect("a seller");
            store::insert_customer(app.database().connection(), &customer, now).expect("a customer");
        }

        let draft = |item: &str| DraftRecord {
            id: crux::DocumentId::new().to_string(),
            seller_id: seller.id.to_string(),
            customer_id: customer.id.to_string(),
            doc_type: "380".to_owned(),
            currency: "EUR".to_owned(),
            issue_date: "2026-11-08".to_owned(),
            due_date: Some("2026-12-08".to_owned()),
            delivery_date: None,
            buyer_reference: Some("PO-1".to_owned()),
            order_reference: None,
            note: None,
            template: None,
            preceding_invoice: None,
            lines: vec![DraftLineRecord {
                id: crux::LineId::new().to_string(),
                position: 1,
                name: item.to_owned(),
                description: None,
                quantity_micros: 1_000_000,
                unit: "HUR".to_owned(),
                unit_price_micros: 100_000_000,
                price_base_quantity_micros: 1_000_000,
                tax_category: "S".to_owned(),
                tax_rate_bp: 2100,
                line_net_micros: 100_000_000,
                exemption_reason: None,
                exemption_reason_code: None,
            }],
        };

        // A blocking finding comes back as data, and nothing is filed.
        let blocked = archive.issue_draft(draft("")).expect("an issue attempt");
        assert!(blocked.issued.is_none());
        assert!(blocked.findings.iter().any(|finding| finding.severity == "blocking"));
        assert!(blocked.ubl_sha256.is_none());
        assert!(
            archive
                .documents(None, None, "outbound".to_owned())
                .expect("the register")
                .is_empty()
        );

        let issued = archive.issue_draft(draft("Consultancy")).expect("an issue");
        let summary = issued.issued.as_ref().expect("an issued document");
        assert_eq!(summary.number, "2026-0001");
        assert_eq!(summary.gross_micros, 121_000_000);
        assert!(issued.findings.is_empty(), "unexpected findings");
        assert_eq!(issued.ubl_sha256.as_deref().map(str::len), Some(64));
        assert!(issued.pdf_error.is_none());

        let documents = archive
            .documents(None, None, "outbound".to_owned())
            .expect("the register");
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].number, "2026-0001");
        assert!(
            archive
                .documents(None, Some("paid".to_owned()), "outbound".to_owned())
                .expect("a filtered register")
                .is_empty()
        );
        assert!(
            archive
                .documents(None, Some("not-a-status".to_owned()), "outbound".to_owned())
                .is_err()
        );

        let detail = archive.document(summary.id.clone()).expect("a detail");
        assert_eq!(detail.buyer, "Buyer BV");
        assert_eq!(detail.lines.len(), 1);
        assert_eq!(detail.artefacts.len(), 2);

        let pdf = archive
            .document_artefact(summary.id.clone(), "pdf".to_owned())
            .expect("the pdf");
        assert_eq!(sha256_hex(&pdf), issued.pdf_sha256.clone().expect("a hash"));

        // The same bytes, handed back as a received document, are stored once.
        let ubl = archive
            .document_artefact(summary.id.clone(), "ubl".to_owned())
            .expect("the ubl");
        let imported = archive
            .import_document(Some("2026-0001.xml".to_owned()), ubl.clone())
            .expect("an import");
        assert_eq!(imported.number, "2026-0001");
        assert!(!imported.duplicate);
        let repeat = archive
            .import_document(Some("2026-0001.xml".to_owned()), ubl)
            .expect("the same bytes again");
        assert!(repeat.duplicate);
        assert_eq!(repeat.document_id, imported.document_id);

        assert!(matches!(
            archive.import_document(Some("notes.txt".to_owned()), b"not an invoice".to_vec()),
            Err(AppError::Import { .. })
        ));

        // The received side lists the import, unmatched, and a person can match it.
        let customer_id = customer.id.to_string();
        let received = archive
            .documents(None, None, "inbound".to_owned())
            .expect("the received list");
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].number, "2026-0001");
        assert!(!received[0].matched, "the sender is not on file");

        archive
            .match_supplier(imported.document_id.clone(), customer_id.clone())
            .expect("a supplier match");
        let matched = archive
            .document(imported.document_id.clone())
            .expect("the received document");
        assert_eq!(matched.supplier_id.as_deref(), Some(customer_id.as_str()));
        assert!(
            archive
                .documents(None, None, "inbound".to_owned())
                .expect("the received list")
                .first()
                .expect("a row")
                .matched
        );

        // Approval moves through the payable register and is visible there.
        assert_eq!(
            archive
                .advance_status(imported.document_id.clone(), "approved".to_owned())
                .expect("an approval"),
            "approved"
        );
        assert_eq!(
            archive
                .documents(None, None, "inbound".to_owned())
                .expect("the received list")[0]
                .status,
            "approved"
        );

        assert!(matches!(
            archive.document("not-a-document".to_owned()),
            Err(AppError::Store { .. })
        ));
    }

    #[test]
    fn a_recovery_key_resets_the_passphrase() {
        let directory = tempfile::TempDir::new().expect("a temporary directory");
        let database = directory.path().join("bonk.db").display().to_string();
        let keystore = directory.path().join("keystore.bin").display().to_string();

        let archive = Archive::create(database.clone(), keystore.clone(), "old".to_owned()).expect("a new archive");
        let recovery = archive.add_recovery_key("old".to_owned()).expect("a recovery key");

        let reset = Archive::reset_passphrase(database.clone(), keystore.clone(), recovery, "new".to_owned())
            .expect("a reset archive");
        let replacement = reset.take_replacement_key().expect("the replacement key");
        assert!(!replacement.is_empty());
        assert!(reset.take_replacement_key().is_none(), "the key is shown once");

        assert!(matches!(
            Archive::open(database.clone(), keystore.clone(), "old".to_owned()),
            Err(AppError::WrongPassphrase)
        ));
        assert!(Archive::open(database, keystore, "new".to_owned()).is_ok());
    }

    #[test]
    fn a_short_secret_is_refused() {
        let result = write_device_secret(Box::new(MemoryStore::default()), "s3-account".to_owned(), vec![1; 31]);
        assert!(matches!(result, Err(AppError::Store { .. })));
    }
}
