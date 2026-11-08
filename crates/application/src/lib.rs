//! UI-agnostic application services.
//!
//! This crate owns the use cases a front end calls: opening and locking an
//! archive, issuing a document, exporting a list, and checking archive health.
//! It depends on no UI toolkit, no window, and no foreign-function layer, so the
//! desktop window, the CLI, and the mobile shell all run the same pipeline.
//!
//! `Archive` keeps the data key for the life of an unlocked session; callers get
//! the database and keystore paths they supplied rather than a platform default.
//! Paths and display words are the front end's business.

#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod customers;
pub mod dto;
pub mod export;
pub mod health;
pub mod import;
pub mod issue;
pub mod passphrase;
pub mod session;
pub mod templates;

pub use dto::{
    BackupExport, CustomerDto, CustomerImportDto, DocumentArtefact, DocumentDetail, DocumentEvent, DocumentHit,
    DocumentLine, DocumentSummary, DraftDto, DraftLineDto, DraftTotalsDto, FindingDto, ImportDto, IssueOutcome,
    PrecedingInvoiceDto, SellerDto, Summary,
};
pub use issue::{DocumentLanguage, invoice_pdf, pdf_data, render_issue};
pub use session::{Archive, restore};
pub use templates::{ArchiveTemplate, DEFAULT_TEMPLATE, default_template, load_templates, renderer_for};
