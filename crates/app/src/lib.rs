#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! egui application as a library.
//!
//! The binary lives in the `cli` crate so one executable can host both the GUI
//! and headless subcommands. This crate owns the session (an unlocked database),
//! the workspace, and the shared UBL renderer used by both front ends.

#![forbid(unsafe_code)]

mod workspace;

use std::path::{Path, PathBuf};

use conformance::{Profile, SemanticInvoice};
use crux::{InvoiceDraft, InvoiceTotals, LocalDate, Party};
use render::PdfRenderer;
use store::{CustomerRecord, Database, IssueView, KdfParams, KeyStore, SellerRecord, StoreError};
use syntax::{UblInvoice, UblLine, write_invoice};

pub use workspace::{Workspace, WorkspaceAction};

/// What the dashboard shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Summary {
    /// Number of seller records.
    pub sellers: i64,
    /// Number of drafts.
    pub drafts: i64,
    /// Applied schema version.
    pub schema_version: i64,
}

/// An unlocked application session: one database, one keystore.
pub struct Session {
    database: Database,
    database_path: PathBuf,
    keystore_path: PathBuf,
}

impl Session {
    /// Creates a new keystore and database. Refuses to overwrite either.
    pub fn create(
        database_path: impl AsRef<Path>,
        keystore_path: impl AsRef<Path>,
        passphrase: &str,
    ) -> Result<Self, StoreError> {
        let database_path = database_path.as_ref().to_path_buf();
        let keystore_path = keystore_path.as_ref().to_path_buf();
        if database_path.exists() || keystore_path.exists() {
            return Err(StoreError::InvalidKeystore);
        }
        if let Some(parent) = database_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if let Some(parent) = keystore_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let keystore = KeyStore::create(&keystore_path, passphrase, KdfParams::DEFAULT)?;
        let database = Database::create(&database_path, &keystore)?;
        Ok(Self {
            database,
            database_path,
            keystore_path,
        })
    }

    /// Unlocks an existing database.
    pub fn unlock(
        database_path: impl AsRef<Path>,
        keystore_path: impl AsRef<Path>,
        passphrase: &str,
    ) -> Result<Self, StoreError> {
        let database_path = database_path.as_ref().to_path_buf();
        let keystore_path = keystore_path.as_ref().to_path_buf();
        let keystore = KeyStore::open(&keystore_path, passphrase)?;
        let database = Database::open(&database_path, &keystore)?;
        Ok(Self {
            database,
            database_path,
            keystore_path,
        })
    }

    /// Dashboard data.
    pub fn summary(&self) -> Result<Summary, StoreError> {
        Ok(Summary {
            sellers: self.database.seller_count()?,
            drafts: self.database.draft_count()?,
            schema_version: self.database.schema_version()?,
        })
    }

    /// Runs SQLite's integrity check on the open database.
    pub fn integrity_check(&self) -> Result<(), StoreError> {
        self.database.integrity_check()
    }

    /// The open database, for application-level commands.
    pub fn database(&self) -> &Database {
        &self.database
    }

    /// Mutable access, required for issuing.
    pub fn database_mut(&mut self) -> &mut Database {
        &mut self.database
    }

    /// Path of the open database.
    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    /// Path of the keystore.
    pub fn keystore_path(&self) -> &Path {
        &self.keystore_path
    }
}

/// Renders a document to UBL for the issue transaction.
///
/// Shared by the GUI and the CLI so both produce byte-identical artefacts.
pub fn render_issue(view: &IssueView<'_>) -> Result<Vec<u8>, StoreError> {
    let semantic = SemanticInvoice::from_draft(
        view.number,
        view.draft,
        &view.seller.party,
        &view.customer.party,
        view.totals.clone(),
        Profile::PeppolBis3,
    );
    let findings = conformance::validate(&semantic);
    if crux::has_blocking(&findings) {
        return Err(StoreError::Conformance(findings));
    }

    let mut lines = Vec::with_capacity(view.draft.lines.len());
    for line in &view.draft.lines {
        let net = line.net().map_err(|error| StoreError::Data(error.to_string()))?;
        lines.push(UblLine {
            id: line.position,
            name: &line.name,
            description: line.description.as_deref(),
            quantity: line.quantity,
            unit: line.unit,
            unit_price: line.unit_price,
            base_quantity: line.price_base_quantity,
            line_net: net,
            treatment: &line.treatment,
        });
    }

    let invoice = UblInvoice {
        number: view.number,
        issue_date: view.draft.issue_date,
        due_date: view.draft.due_date,
        delivery_date: view.draft.delivery_date,
        doc_type: view.draft.doc_type,
        currency: view.draft.currency,
        buyer_reference: view.draft.buyer_reference.as_deref(),
        order_reference: view.draft.order_reference.as_deref(),
        note: view.draft.note.as_deref(),
        seller: &view.seller.party,
        customer: &view.customer.party,
        lines: &lines,
        totals: view.totals,
    };

    write_invoice(&invoice)
        .map(String::into_bytes)
        .map_err(|error| StoreError::Render(error.to_string()))
}

/// Renders the visual invoice PDF for an issued document.
///
/// Uses the Typst CLI in M1; the template is shared data, so the backend can
/// become the embedded Typst library when PDF/A-3 work starts.
pub fn invoice_pdf(
    number: &str,
    draft: &InvoiceDraft,
    seller: &SellerRecord,
    customer: &CustomerRecord,
    totals: &InvoiceTotals,
) -> Result<Vec<u8>, StoreError> {
    let data = render::InvoiceData {
        number: number.to_owned(),
        issue_date: draft.issue_date.to_iso_string(),
        due_date: draft.due_date.map(LocalDate::to_iso_string),
        currency: draft.currency.to_string(),
        seller: party_data(&seller.party),
        customer: party_data(&customer.party),
        lines: draft
            .lines
            .iter()
            .map(|line| render::LineData {
                name: line.name.clone(),
                qty: line.quantity.to_trimmed_string(),
                price: line.unit_price.to_trimmed_string(),
                vat: line.treatment.rate.percent_string(),
                total: line
                    .net()
                    .ok()
                    .and_then(|net| net.to_fixed_string(2).ok())
                    .unwrap_or_default(),
            })
            .collect(),
        net: totals.net.to_fixed_string(2).unwrap_or_default(),
        tax: totals.tax.to_fixed_string(2).unwrap_or_default(),
        gross: totals.gross.to_fixed_string(2).unwrap_or_default(),
        notes: draft.note.clone(),
    };

    render::TypstCliRenderer::from_environment()
        .render(&data)
        .map_err(|error| StoreError::Render(error.to_string()))
}

fn party_data(party: &Party) -> render::PartyData {
    let mut parts = Vec::new();
    if let Some(street) = &party.address.street {
        parts.push(street.clone());
    }
    parts.push(party.address.city.clone());
    parts.push(party.address.country.to_string());
    render::PartyData {
        name: party.legal_name.clone(),
        address: parts.join(", "),
        vat: party.vat_id.clone(),
    }
}

/// Default locations under the platform data directory.
pub fn default_paths() -> (PathBuf, PathBuf) {
    let base = data_dir().join("Bonk");
    (base.join("bonk.db"), base.join("keystore.bin"))
}

fn data_dir() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        return PathBuf::from(appdata);
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(xdg);
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".local").join("share");
    }
    PathBuf::from(".")
}

/// The application's UI state.
enum UiState {
    Locked {
        passphrase: String,
        error: Option<String>,
    },
    Unlocked {
        session: Session,
        workspace: Box<Workspace>,
    },
}

/// The egui application.
pub struct BonkApp {
    database_path: String,
    keystore_path: String,
    state: UiState,
}

impl Default for BonkApp {
    fn default() -> Self {
        let (database, keystore) = default_paths();
        Self {
            database_path: database.display().to_string(),
            keystore_path: keystore.display().to_string(),
            state: UiState::Locked {
                passphrase: String::new(),
                error: None,
            },
        }
    }
}

/// A user intention, applied after the frame is built so that no widget closure
/// holds a borrow of the application state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
    Unlock,
    Create,
    Lock,
}

impl BonkApp {
    /// Creates the app with its default paths.
    pub fn new() -> Self {
        Self::default()
    }

    fn unlock(&mut self) {
        let UiState::Locked { passphrase, .. } = &self.state else {
            return;
        };
        let passphrase = passphrase.clone();
        let database_path = PathBuf::from(self.database_path.trim());
        let keystore_path = PathBuf::from(self.keystore_path.trim());

        let opened = Session::unlock(&database_path, &keystore_path, &passphrase);
        self.finish(opened);
    }

    fn create(&mut self) {
        let UiState::Locked { passphrase, .. } = &self.state else {
            return;
        };
        let passphrase = passphrase.clone();
        let database_path = PathBuf::from(self.database_path.trim());
        let keystore_path = PathBuf::from(self.keystore_path.trim());

        let opened = Session::create(&database_path, &keystore_path, &passphrase);
        self.finish(opened);
    }

    fn finish(&mut self, opened: Result<Session, StoreError>) {
        match opened {
            Ok(session) => match Workspace::new(&session) {
                Ok(workspace) => {
                    self.state = UiState::Unlocked {
                        session,
                        workspace: Box::new(workspace),
                    }
                }
                Err(cause) => self.set_error(cause.to_string()),
            },
            Err(cause) => self.set_error(cause.to_string()),
        }
    }

    fn set_error(&mut self, message: String) {
        if let UiState::Locked { error, .. } = &mut self.state {
            *error = Some(message);
        }
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::Unlock => self.unlock(),
            Action::Create => self.create(),
            Action::Lock => {
                self.state = UiState::Locked {
                    passphrase: String::new(),
                    error: None,
                };
            }
        }
    }
}

impl eframe::App for BonkApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            let mut action = None;
            match &mut self.state {
                UiState::Locked { passphrase, error } => {
                    ui.heading("Bonk");
                    ui.label("Local-first invoicing. Nothing leaves this machine unless you send it.");
                    ui.add_space(12.0);

                    ui.label("Database");
                    ui.text_edit_singleline(&mut self.database_path);
                    ui.label("Keystore");
                    ui.text_edit_singleline(&mut self.keystore_path);
                    ui.add_space(8.0);

                    ui.label("Passphrase");
                    ui.add(egui::TextEdit::singleline(passphrase).password(true));

                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Unlock").clicked() {
                            action = Some(Action::Unlock);
                        }
                        if ui.button("Create new database").clicked() {
                            action = Some(Action::Create);
                        }
                    });

                    if let Some(message) = error {
                        ui.add_space(8.0);
                        ui.colored_label(egui::Color32::from_rgb(200, 60, 60), message);
                    }
                }
                UiState::Unlocked { session, workspace } => {
                    if workspace.ui(ui, session) == WorkspaceAction::Lock {
                        action = Some(Action::Lock);
                    }
                }
            }

            if let Some(action) = action {
                self.apply(action);
            }
        });
    }
}

/// Launches the desktop window.
pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions::default();
    eframe::run_native("Bonk", options, Box::new(|_cc| Ok(Box::new(BonkApp::new()))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn create_then_unlock_round_trips() {
        let dir = TempDir::new().unwrap();
        let database = dir.path().join("bonk.db");
        let keystore = dir.path().join("keystore.bin");

        {
            let session = Session::create(&database, &keystore, "passphrase").unwrap();
            assert_eq!(session.summary().unwrap().sellers, 0);
            assert_eq!(session.summary().unwrap().schema_version, 2);
            assert!(session.database_path().exists());
        }

        let session = Session::unlock(&database, &keystore, "passphrase").unwrap();
        assert_eq!(session.summary().unwrap().sellers, 0);
        assert!(Session::unlock(&database, &keystore, "wrong").is_err());
    }

    #[test]
    fn create_refuses_to_overwrite() {
        let dir = TempDir::new().unwrap();
        let database = dir.path().join("bonk.db");
        let keystore = dir.path().join("keystore.bin");
        Session::create(&database, &keystore, "passphrase").unwrap();
        assert!(Session::create(&database, &keystore, "passphrase").is_err());
    }

    #[test]
    fn default_paths_are_under_one_directory() {
        let (database, keystore) = default_paths();
        assert_eq!(database.parent(), keystore.parent());
        assert_eq!(database.file_name().unwrap(), "bonk.db");
        assert_eq!(keystore.file_name().unwrap(), "keystore.bin");
    }
}
