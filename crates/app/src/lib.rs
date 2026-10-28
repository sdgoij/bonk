#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! egui application as a library.
//!
//! The binary lives in the `cli` crate so one executable can host both the GUI
//! and headless subcommands. This crate owns the session (an unlocked database),
//! the workspace, and the shared UBL renderer used by both front ends.

#![forbid(unsafe_code)]

pub mod customers;
pub mod export;
pub mod health;
pub mod idle;
pub mod strength;
mod table;
pub mod text;
pub mod theme;
mod workspace;

/// Marks the context as styled, so installing is not repeated every frame.
const THEME_INSTALLED: &str = "bonk.theme-installed";

use std::path::{Path, PathBuf};

use conformance::{Profile, SemanticInvoice};
use crux::{InvoiceDraft, InvoiceTotals, LocalDate, Party};
use render::PdfRenderer;
use store::{CustomerRecord, Database, IssueView, KdfParams, KeyStore, SellerRecord, StoreError};
use syntax::{UblInvoice, UblLine, UblPrecedingInvoice, write_invoice};
use zeroize::{Zeroize, Zeroizing};

pub use workspace::{Workspace, WorkspaceAction};

/// The language the printed document speaks.
///
/// Re-exported so a front end that renders through `invoice_pdf` does not need a
/// direct dependency on the renderer.
pub use render::Language as DocumentLanguage;

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
    /// The data key, kept so the window can write a verified backup without
    /// asking for the passphrase again. It is the same key SQLCipher is using.
    dek: Zeroizing<[u8; 32]>,
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
            store::permissions::private_directory(parent)?;
        }
        if let Some(parent) = keystore_path.parent() {
            store::permissions::private_directory(parent)?;
        }

        let keystore = KeyStore::create(&keystore_path, passphrase, KdfParams::DEFAULT)?;
        let dek = Zeroizing::new(*keystore.dek());
        let database = Database::create(&database_path, &keystore)?;
        Ok(Self {
            database,
            database_path,
            keystore_path,
            dek,
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
        let dek = Zeroizing::new(*keystore.dek());
        let database = Database::open(&database_path, &keystore)?;
        Ok(Self {
            database,
            database_path,
            keystore_path,
            dek,
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

    /// Adds or replaces the recovery key, returning the secret to show once.
    ///
    /// The passphrase is required again rather than taken from the unlock: a
    /// credential change should not ride on a window that has been left open.
    pub fn add_recovery_key(&self, passphrase: &str) -> Result<String, StoreError> {
        let mut keystore = KeyStore::open(self.keystore_path(), passphrase)?;
        let key = keystore.enable_recovery(passphrase)?;
        if let Err(error) = store::audit::record_recovery_key(
            self.database.connection(),
            &store::now_rfc3339(),
            "gui",
            &keystore.path().display().to_string(),
        ) {
            // The key is written but unrecorded. Saying so is better than
            // hiding it, and adding a key again replaces it with one that is
            // recorded.
            return Err(StoreError::Rejected(format!(
                "the recovery key was written, but recording it in the audit chain failed: {error}. \
                 Add a recovery key again to replace it."
            )));
        }
        Ok(key)
    }

    /// Rewraps the data key under a new passphrase.
    pub fn change_passphrase(&self, current: &str, new: &str) -> Result<(), StoreError> {
        let mut keystore = KeyStore::open(self.keystore_path(), current)?;
        keystore.change_passphrase(new)?;
        if let Err(error) = store::audit::record_passphrase_change(
            self.database.connection(),
            &store::now_rfc3339(),
            "gui",
            &keystore.path().display().to_string(),
        ) {
            return Err(StoreError::Rejected(format!(
                "the passphrase was changed, but recording it in the audit chain failed: {error}"
            )));
        }
        Ok(())
    }

    /// Keeps the data key in this device's credential store, so the archive can
    /// be opened without the passphrase.
    ///
    /// The passphrase is required again: trusting a machine should not be
    /// something an unattended window can do. The decision goes into the audit
    /// chain, because a second way in is evidence about the archive.
    pub fn remember_device(&self, passphrase: &str, store: &dyn store::DeviceStore) -> Result<(), StoreError> {
        let keystore = KeyStore::open(self.keystore_path(), passphrase)?;
        store::device::remember(store, self.keystore_path(), keystore.dek())?;
        if let Err(error) = store::audit::record_device_trust(
            self.database.connection(),
            &store::now_rfc3339(),
            "gui",
            &self.keystore_path().display().to_string(),
        ) {
            return Err(StoreError::Rejected(format!(
                "the key was stored for this device, but recording it in the audit chain failed: {error}"
            )));
        }
        Ok(())
    }

    /// Stops this device from keeping the key. False when it did not have one.
    pub fn forget_device(&self, store: &dyn store::DeviceStore) -> Result<bool, StoreError> {
        let forgotten = store::device::forget(store, self.keystore_path())?;
        if forgotten
            && let Err(error) = store::audit::record_device_forget(
                self.database.connection(),
                &store::now_rfc3339(),
                "gui",
                &self.keystore_path().display().to_string(),
            )
        {
            return Err(StoreError::Rejected(format!(
                "the key was forgotten by this device, but recording it in the audit chain failed: {error}"
            )));
        }
        Ok(forgotten)
    }

    /// True when this device holds a key for the archive at these paths.
    pub fn device_ready(keystore_path: impl AsRef<Path>, store: &dyn store::DeviceStore) -> bool {
        matches!(store::device::recall(store, keystore_path.as_ref()), Ok(Some(_)))
    }

    /// Opens the archive with the key this device kept.
    ///
    /// The keystore is not consulted: the point of the feature is that the
    /// operating system's login is enough.
    pub fn unlock_with_device(
        database_path: impl AsRef<Path>,
        keystore_path: impl AsRef<Path>,
        store: &dyn store::DeviceStore,
    ) -> Result<Self, StoreError> {
        let database_path = database_path.as_ref().to_path_buf();
        let keystore_path = keystore_path.as_ref().to_path_buf();
        let Some(dek) = store::device::recall(store, &keystore_path)? else {
            return Err(StoreError::Rejected(
                "this device has no key for that archive; unlock with the passphrase once and remember it".to_owned(),
            ));
        };
        let database = Database::open_with_key(&database_path, &dek)?;
        Ok(Self {
            database,
            database_path,
            keystore_path,
            dek,
        })
    }

    /// Replaces a forgotten passphrase using the recovery key.
    ///
    /// Returns the session and the replacement recovery key. The key that was
    /// used is spent: it has now been typed somewhere, and leaving it valid would
    /// keep a second route open.
    pub fn reset_passphrase(
        database_path: impl AsRef<Path>,
        keystore_path: impl AsRef<Path>,
        recovery_key: &str,
        new_passphrase: &str,
    ) -> Result<(Self, String), StoreError> {
        let database_path = database_path.as_ref().to_path_buf();
        let keystore_path = keystore_path.as_ref().to_path_buf();
        // Open the database first: a wrong key or a mismatched archive must
        // leave the keystore exactly as it was.
        let mut keystore = KeyStore::open_with_recovery(&keystore_path, recovery_key).map_err(|error| match error {
            StoreError::WrongPassphrase => {
                StoreError::Rejected("that recovery key does not open this archive".to_owned())
            }
            other => other,
        })?;
        let database = Database::open(&database_path, &keystore)?;
        let dek = Zeroizing::new(*keystore.dek());

        keystore.change_passphrase(new_passphrase)?;
        let replacement = keystore.enable_recovery(new_passphrase)?;
        if let Err(error) = store::audit::record_passphrase_reset(
            database.connection(),
            &store::now_rfc3339(),
            "gui",
            &keystore.path().display().to_string(),
            true,
        ) {
            return Err(StoreError::Rejected(format!(
                "the passphrase was reset, but recording it in the audit chain failed: {error}. \
                 The new recovery key is {replacement}; write it down now."
            )));
        }

        Ok((
            Self {
                database,
                database_path,
                keystore_path,
                dek,
            },
            replacement,
        ))
    }

    /// Writes a verified, encrypted backup using the key this session opened with.
    pub fn backup(
        &self,
        directory: impl AsRef<Path>,
        stamp: &str,
        keep: usize,
    ) -> Result<store::BackupReport, StoreError> {
        store::backup::backup_with_key(&self.database, &self.dek, directory, stamp, keep)
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
        preceding_invoice: view
            .draft
            .preceding_invoice
            .as_ref()
            .map(|preceding| UblPrecedingInvoice {
                number: &preceding.number,
                issue_date: preceding.issue_date,
            }),
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
    language: render::Language,
) -> Result<Vec<u8>, StoreError> {
    let data = pdf_data(number, draft, seller, customer, totals, language);

    render::TypstCliRenderer::from_environment()
        .render(&data)
        .map_err(|error| StoreError::Render(error.to_string()))
}

/// The payload handed to the template.
///
/// Kept apart from the renderer so the shape can be checked without shelling out
/// to Typst, and so the words the document prints are decided in one place.
pub fn pdf_data(
    number: &str,
    draft: &InvoiceDraft,
    seller: &SellerRecord,
    customer: &CustomerRecord,
    totals: &InvoiceTotals,
    language: render::Language,
) -> render::InvoiceData {
    render::InvoiceData {
        title: language.document_title(draft.doc_type.is_credit_note()).to_owned(),
        labels: language.labels(),
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
    }
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
        form: LockForm,
        error: Option<String>,
    },
    Unlocked {
        session: Session,
        workspace: Box<Workspace>,
    },
}

/// What the lock screen is asking for.
enum LockForm {
    /// Open an existing archive.
    Unlock { passphrase: String },
    /// Create a new one, which takes the phrase twice.
    Create { passphrase: String, confirmation: String },
    /// Replace a forgotten passphrase with the recovery key.
    Reset {
        recovery_key: String,
        new_passphrase: String,
        repeat: String,
        /// The replacement key, once the reset has happened. Shown until the
        /// user says they have written it down.
        replacement: Option<String>,
    },
}

/// The egui application.
pub struct BonkApp {
    database_path: String,
    keystore_path: String,
    state: UiState,
    idle: idle::IdleLock,
    /// Interface language. English today, and set here rather than in each
    /// screen so a second table has one place to be chosen.
    language: text::Language,
    /// True when this device holds the key for the configured archive.
    device_ready: bool,
    /// The chosen appearance, kept so `save` can write it down.
    appearance: egui::ThemePreference,
}

impl Default for BonkApp {
    fn default() -> Self {
        let (database, keystore) = default_paths();
        Self {
            database_path: database.display().to_string(),
            keystore_path: keystore.display().to_string(),
            state: UiState::Locked {
                form: LockForm::Unlock {
                    passphrase: String::new(),
                },
                error: None,
            },
            idle: idle::IdleLock::new(idle::configured_timeout(), std::time::Instant::now()),
            language: text::Language::default(),
            device_ready: Self::device_ready_for(&keystore.display().to_string()),
            appearance: egui::ThemePreference::System,
        }
    }
}

/// A user intention, applied after the frame is built so that no widget closure
/// holds a borrow of the application state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
    Unlock,
    UnlockDevice,
    BeginCreate,
    Create,
    CancelCreate,
    BeginReset,
    Reset,
    CancelReset,
    FinishReset,
    Lock,
}

impl BonkApp {
    /// Creates the app with its default paths.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates the app, installs the theme, and restores the appearance choice.
    fn new_with_context(context: &eframe::CreationContext<'_>) -> Self {
        theme::install(&context.egui_ctx);
        let mut app = Self::default();
        if let Some(storage) = context.storage
            && let Some(name) = storage.get_string(theme::STORAGE_KEY)
        {
            app.appearance = theme::from_name(&name).unwrap_or_default();
        }
        theme::set(&context.egui_ctx, app.appearance);
        app
    }

    fn unlock(&mut self) {
        let UiState::Locked {
            form: LockForm::Unlock { passphrase },
            ..
        } = &self.state
        else {
            return;
        };
        // The copy is wiped when it drops; the form's own buffer is wiped by
        // `finish` once the credentials are accepted.
        let passphrase = Zeroizing::new(passphrase.clone());
        let database_path = PathBuf::from(self.database_path.trim());
        let keystore_path = PathBuf::from(self.keystore_path.trim());

        let opened = Session::unlock(&database_path, &keystore_path, &passphrase);
        self.finish(opened);
    }

    /// True when this device holds a key for the archive at `keystore_path`.
    fn device_ready_for(keystore_path: &str) -> bool {
        Session::device_ready(keystore_path.trim(), &store::device::Keychain)
    }

    /// Opens the archive with the key this device kept, without asking for
    /// anything.
    fn unlock_with_device(&mut self) {
        let database_path = PathBuf::from(self.database_path.trim());
        let keystore_path = PathBuf::from(self.keystore_path.trim());
        let opened = Session::unlock_with_device(&database_path, &keystore_path, &store::device::Keychain);
        self.finish(opened);
    }

    fn create(&mut self) {
        let UiState::Locked {
            form: LockForm::Create {
                passphrase,
                confirmation,
            },
            ..
        } = &self.state
        else {
            return;
        };
        if let Err(reason) = confirm(passphrase, confirmation) {
            self.set_error(reason.to_owned());
            return;
        }
        let passphrase = Zeroizing::new(passphrase.clone());
        let database_path = PathBuf::from(self.database_path.trim());
        let keystore_path = PathBuf::from(self.keystore_path.trim());

        let opened = Session::create(&database_path, &keystore_path, &passphrase);
        self.finish(opened);
    }

    /// Replaces a forgotten passphrase with the recovery key.
    ///
    /// The session that comes back is dropped on purpose: the user still has to
    /// acknowledge the replacement key, and unlocking afterwards re-derives
    /// everything from the new passphrase.
    fn reset(&mut self) {
        let UiState::Locked {
            form:
                LockForm::Reset {
                    recovery_key,
                    new_passphrase,
                    repeat,
                    replacement,
                },
            ..
        } = &self.state
        else {
            return;
        };
        if replacement.is_some() {
            return;
        }
        if let Err(reason) = check_reset(recovery_key, new_passphrase, repeat) {
            self.set_error(reason.to_owned());
            return;
        }

        let database_path = PathBuf::from(self.database_path.trim());
        let keystore_path = PathBuf::from(self.keystore_path.trim());
        let key = Zeroizing::new(recovery_key.clone());
        let phrase = Zeroizing::new(new_passphrase.clone());

        match Session::reset_passphrase(&database_path, &keystore_path, &key, &phrase) {
            Ok((_, replacement_key)) => {
                if let UiState::Locked {
                    form:
                        LockForm::Reset {
                            recovery_key,
                            replacement,
                            ..
                        },
                    error,
                } = &mut self.state
                {
                    // The used key is spent; the new one is all the user has to
                    // write down, so it stays on screen until acknowledged.
                    recovery_key.zeroize();
                    *replacement = Some(replacement_key);
                    *error = None;
                }
            }
            Err(cause) => self.set_error(cause.to_string()),
        }
    }

    /// Leaves the reset screen with the new phrase, and unlocks with it.
    fn finish_reset(&mut self) {
        let UiState::Locked {
            form:
                LockForm::Reset {
                    new_passphrase,
                    replacement,
                    ..
                },
            ..
        } = &mut self.state
        else {
            return;
        };
        if let Some(mut key) = replacement.take() {
            key.zeroize();
        }
        let passphrase = std::mem::take(new_passphrase);
        self.state = UiState::Locked {
            form: LockForm::Unlock { passphrase },
            error: None,
        };
        self.unlock();
    }

    fn finish(&mut self, opened: Result<Session, StoreError>) {
        match opened {
            Ok(session) => {
                // The credentials were accepted, so the form no longer needs
                // them. A wrong passphrase is left in place instead, so a typo
                // can be corrected without retyping.
                self.wipe_passphrase_buffer();
                match Workspace::new(&session) {
                    Ok(workspace) => {
                        // A fresh unlock starts the idle clock, whatever
                        // happened before it.
                        self.idle.touch(std::time::Instant::now());
                        self.state = UiState::Unlocked {
                            session,
                            workspace: Box::new(workspace),
                        }
                    }
                    Err(cause) => self.set_error(cause.to_string()),
                }
            }
            Err(cause) => self.set_error(cause.to_string()),
        }
    }

    /// Wipes the lock screen's editor buffers.
    ///
    /// egui's `TextEdit` writes into a plain `String`, so the buffers are wiped
    /// here rather than on drop; dropping them would leave the bytes in the
    /// allocator.
    fn wipe_passphrase_buffer(&mut self) {
        if let UiState::Locked { form, .. } = &mut self.state {
            match form {
                LockForm::Unlock { passphrase } => passphrase.zeroize(),
                LockForm::Create {
                    passphrase,
                    confirmation,
                } => {
                    passphrase.zeroize();
                    confirmation.zeroize();
                }
                LockForm::Reset {
                    recovery_key,
                    new_passphrase,
                    repeat,
                    replacement,
                } => {
                    recovery_key.zeroize();
                    new_passphrase.zeroize();
                    repeat.zeroize();
                    if let Some(replacement) = replacement {
                        replacement.zeroize();
                    }
                }
            }
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
            Action::UnlockDevice => self.unlock_with_device(),
            Action::BeginCreate => {
                let UiState::Locked { form, error } = &mut self.state else {
                    return;
                };
                let LockForm::Unlock { passphrase } = form else {
                    return;
                };
                if passphrase.trim().is_empty() {
                    *error = Some("enter a passphrase first".to_owned());
                } else {
                    // The phrase typed for unlocking is the one being
                    // confirmed, so it moves into the new form rather than
                    // being copied and left behind.
                    let passphrase = std::mem::take(passphrase);
                    *form = LockForm::Create {
                        passphrase,
                        confirmation: String::new(),
                    };
                    *error = None;
                }
            }
            Action::Create => self.create(),
            Action::CancelCreate => {
                let UiState::Locked { form, error } = &mut self.state else {
                    return;
                };
                let LockForm::Create {
                    passphrase,
                    confirmation,
                } = form
                else {
                    return;
                };
                confirmation.zeroize();
                *form = LockForm::Unlock {
                    passphrase: std::mem::take(passphrase),
                };
                *error = None;
            }
            Action::BeginReset => {
                let UiState::Locked { form, error } = &mut self.state else {
                    return;
                };
                let LockForm::Unlock { passphrase } = form else {
                    return;
                };
                // The phrase that was tried is not carried over: it did not
                // work, and the reset is about the key instead.
                passphrase.zeroize();
                *form = LockForm::Reset {
                    recovery_key: String::new(),
                    new_passphrase: String::new(),
                    repeat: String::new(),
                    replacement: None,
                };
                *error = None;
            }
            Action::Reset => self.reset(),
            Action::CancelReset => {
                let UiState::Locked { form, error } = &mut self.state else {
                    return;
                };
                let LockForm::Reset {
                    recovery_key,
                    new_passphrase,
                    repeat,
                    replacement,
                } = form
                else {
                    return;
                };
                recovery_key.zeroize();
                new_passphrase.zeroize();
                repeat.zeroize();
                if let Some(mut replacement) = replacement.take() {
                    replacement.zeroize();
                }
                *form = LockForm::Unlock {
                    passphrase: String::new(),
                };
                *error = None;
            }
            Action::FinishReset => self.finish_reset(),
            Action::Lock => {
                self.state = UiState::Locked {
                    form: LockForm::Unlock {
                        passphrase: String::new(),
                    },
                    error: None,
                };
                // The device entry can have been added or removed while the
                // workspace was open.
                self.device_ready = Self::device_ready_for(&self.keystore_path);
            }
        }
    }
}

/// Whether a new archive's passphrase was entered twice, with the reason when it
/// was not.
///
/// A typo here is unrecoverable: the archive is sealed with whatever was typed,
/// and nothing else can open it.
fn confirm(passphrase: &str, confirmation: &str) -> Result<(), &'static str> {
    if passphrase.trim().is_empty() {
        return Err("enter a passphrase first");
    }
    if passphrase != confirmation {
        return Err("the passphrases do not match");
    }
    Ok(())
}

/// Whether a reset is well formed, with the reason when it is not.
fn check_reset(recovery_key: &str, new_passphrase: &str, repeat: &str) -> Result<(), &'static str> {
    if recovery_key.trim().is_empty() {
        return Err("enter the recovery key");
    }
    if new_passphrase.trim().is_empty() {
        return Err("enter a new passphrase");
    }
    if new_passphrase != repeat {
        return Err("the new passphrases do not match");
    }
    Ok(())
}

/// Draws the strength estimate under a passphrase field.
pub(crate) fn strength_row(ui: &mut egui::Ui, language: text::Language, passphrase: &str) {
    if passphrase.is_empty() {
        return;
    }
    let tokens = theme::palette(ui.ctx());
    let assessment = strength::assess(passphrase);
    let colour = match assessment.strength {
        strength::Strength::Weak => tokens.error,
        strength::Strength::Fair => tokens.warning,
        strength::Strength::Strong => tokens.success,
    };
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.label(text::strength::label(language));
        ui.colored_label(colour, text::strength::level(language, assessment.strength));
        ui.label(text::strength::reason(language, assessment.reason))
            .on_hover_text(text::strength::detail(language, assessment.bits));
    });
}

impl eframe::App for BonkApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // A window created outside `run` has not been styled yet; do it once.
        let installed = ui
            .ctx()
            .data(|data| data.get_temp::<bool>(egui::Id::unique(THEME_INSTALLED)));
        if installed != Some(true) {
            theme::install(ui.ctx());
            ui.ctx()
                .data_mut(|data| data.insert_temp(egui::Id::unique(THEME_INSTALLED), true));
        }

        // Any event counts as activity: a pointer move, a key, a scroll, a focus
        // change. The clock is checked before drawing, so a due lock shows the
        // unlock screen on this frame rather than the next.
        let now = std::time::Instant::now();
        if ui.input(|input| !input.events.is_empty()) {
            self.idle.touch(now);
        }
        if matches!(self.state, UiState::Unlocked { .. }) && self.idle.due(now) {
            self.apply(Action::Lock);
        }
        // egui is reactive: without a scheduled repaint an idle window is never
        // asked to draw, and the timeout would only be noticed on the next
        // mouse move. Half the shortest useful check keeps the drift small.
        if matches!(self.state, UiState::Unlocked { .. }) {
            ui.ctx().request_repaint_after(std::time::Duration::from_secs(30));
        }

        egui::CentralPanel::default().show(ui, |ui| {
            let mut action = None;
            let language = self.language;
            let device_ready = self.device_ready;
            match &mut self.state {
                UiState::Locked { form, error } => {
                    ui.heading(text::shell::PRODUCT);
                    match form {
                        LockForm::Create {
                            passphrase,
                            confirmation,
                        } => {
                            ui.label(text::shell::create_note(language));
                            ui.add_space(12.0);

                            ui.label(text::shell::new_passphrase_label(language));
                            ui.add(egui::TextEdit::singleline(passphrase).password(true));
                            strength_row(ui, language, passphrase.as_str());

                            ui.add_space(8.0);
                            ui.label(text::shell::repeat_passphrase_label(language));
                            ui.add(egui::TextEdit::singleline(confirmation).password(true));

                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                if ui.button(text::shell::create_action(language)).clicked() {
                                    action = Some(Action::Create);
                                }
                                if ui.button(text::shell::back_action(language)).clicked() {
                                    action = Some(Action::CancelCreate);
                                }
                            });
                        }
                        LockForm::Unlock { passphrase } => {
                            ui.label(text::shell::tagline(language));
                            ui.add_space(12.0);

                            ui.label(text::shell::database_label(language));
                            ui.text_edit_singleline(&mut self.database_path);
                            ui.label(text::shell::keystore_label(language));
                            ui.text_edit_singleline(&mut self.keystore_path);
                            ui.add_space(8.0);

                            ui.label(text::shell::passphrase_label(language));
                            ui.add(egui::TextEdit::singleline(passphrase).password(true));
                            strength_row(ui, language, passphrase.as_str());

                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                if ui.button(text::shell::unlock_action(language)).clicked() {
                                    action = Some(Action::Unlock);
                                }
                                if ui.button(text::shell::create_database_action(language)).clicked() {
                                    action = Some(Action::BeginCreate);
                                }
                            });
                            if ui.button(text::shell::forgotten_action(language)).clicked() {
                                action = Some(Action::BeginReset);
                            }
                            if device_ready && ui.button(text::shell::unlock_with_device(language)).clicked() {
                                action = Some(Action::UnlockDevice);
                            }
                        }
                        LockForm::Reset {
                            recovery_key,
                            new_passphrase,
                            repeat,
                            replacement,
                        } => match replacement {
                            Some(key) => {
                                ui.label(text::shell::reset_done_note(language));
                                ui.add_space(8.0);
                                ui.label(text::shell::new_recovery_key_label(language));
                                ui.label(egui::RichText::new(key.as_str()).monospace());
                                ui.add_space(8.0);
                                ui.colored_label(
                                    theme::palette(ui.ctx()).warning,
                                    text::shell::keep_key_note(language),
                                );
                                ui.add_space(8.0);
                                if ui.button(text::shell::written_down_action(language)).clicked() {
                                    action = Some(Action::FinishReset);
                                }
                            }
                            None => {
                                ui.label(text::shell::reset_note(language));
                                ui.add_space(12.0);

                                ui.label(text::shell::recovery_key_label(language));
                                ui.add(egui::TextEdit::singleline(recovery_key).password(true));

                                ui.add_space(8.0);
                                ui.label(text::shell::new_passphrase_label(language));
                                ui.add(egui::TextEdit::singleline(new_passphrase).password(true));
                                strength_row(ui, language, new_passphrase.as_str());

                                ui.add_space(8.0);
                                ui.label(text::shell::repeat_new_passphrase_label(language));
                                ui.add(egui::TextEdit::singleline(repeat).password(true));

                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    if ui.button(text::shell::reset_action(language)).clicked() {
                                        action = Some(Action::Reset);
                                    }
                                    if ui.button(text::shell::back_action(language)).clicked() {
                                        action = Some(Action::CancelReset);
                                    }
                                });
                            }
                        },
                    }

                    if let Some(message) = error {
                        ui.add_space(8.0);
                        ui.colored_label(theme::palette(ui.ctx()).error, message);
                    }
                }
                UiState::Unlocked { session, workspace } => {
                    if workspace.ui(ui, session, &mut self.appearance) == WorkspaceAction::Lock {
                        action = Some(Action::Lock);
                    }
                }
            }

            if let Some(action) = action {
                self.apply(action);
            }
        });
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(theme::STORAGE_KEY, self.appearance.name().to_owned());
    }
}

/// Launches the desktop window.
pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([900.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Bonk",
        options,
        Box::new(|context| Ok(Box::new(BonkApp::new_with_context(context)))),
    )
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
            assert_eq!(session.summary().unwrap().schema_version, 3);
            assert!(session.database_path().exists());
        }

        let session = Session::unlock(&database, &keystore, "passphrase").unwrap();
        assert_eq!(session.summary().unwrap().sellers, 0);
        assert!(Session::unlock(&database, &keystore, "wrong").is_err());
    }

    #[test]
    fn a_session_writes_a_verified_backup() {
        let dir = TempDir::new().unwrap();
        let session = Session::create(
            dir.path().join("bonk.db"),
            dir.path().join("keystore.bin"),
            "passphrase",
        )
        .unwrap();
        let report = session
            .backup(dir.path().join("backups"), "2026-10-28T12:00:00Z", 5)
            .unwrap();
        assert!(report.path.exists());
        assert!(report.bytes > 0);
        assert!(report.removed.is_empty());
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
    fn a_wrong_passphrase_stays_for_a_retry_and_a_used_buffer_is_wiped() {
        let dir = TempDir::new().unwrap();
        let database = dir.path().join("bonk.db");
        let keystore = dir.path().join("keystore.bin");
        Session::create(&database, &keystore, "buffer-canary").unwrap();

        let mut app = BonkApp {
            database_path: database.display().to_string(),
            keystore_path: keystore.display().to_string(),
            state: UiState::Locked {
                form: LockForm::Unlock {
                    passphrase: "wrong-canary".to_owned(),
                },
                error: None,
            },
            ..Default::default()
        };

        app.apply(Action::Unlock);

        {
            let UiState::Locked {
                form: LockForm::Unlock { passphrase },
                error,
            } = &app.state
            else {
                panic!("a wrong passphrase must not unlock the archive");
            };
            assert_eq!(passphrase, "wrong-canary", "a retry needs the text still there");
            assert!(error.is_some());
        }

        app.wipe_passphrase_buffer();

        let UiState::Locked {
            form: LockForm::Unlock { passphrase },
            ..
        } = &app.state
        else {
            panic!("wiping the buffer must not change the state");
        };
        assert!(passphrase.is_empty(), "a used buffer must hold no passphrase");
    }

    #[test]
    fn creating_needs_the_same_passphrase_twice() {
        assert_eq!(confirm("", ""), Err("enter a passphrase first"));
        assert_eq!(confirm("   ", "   "), Err("enter a passphrase first"));
        assert_eq!(confirm("one", "two"), Err("the passphrases do not match"));
        assert_eq!(confirm("same", "same"), Ok(()));
    }

    #[test]
    fn a_creation_without_a_passphrase_does_not_start() {
        let mut app = BonkApp {
            state: UiState::Locked {
                form: LockForm::Unlock {
                    passphrase: "   ".to_owned(),
                },
                error: None,
            },
            ..Default::default()
        };

        app.apply(Action::BeginCreate);

        assert!(matches!(
            &app.state,
            UiState::Locked {
                form: LockForm::Unlock { .. },
                error: Some(message),
            } if message.contains("enter a passphrase")
        ));
    }

    #[test]
    fn a_new_archive_is_confirmed_before_it_is_written() {
        let dir = TempDir::new().unwrap();
        let database = dir.path().join("bonk.db");
        let keystore = dir.path().join("keystore.bin");

        let mut app = BonkApp {
            database_path: database.display().to_string(),
            keystore_path: keystore.display().to_string(),
            state: UiState::Locked {
                form: LockForm::Unlock {
                    passphrase: "first-canary".to_owned(),
                },
                error: None,
            },
            ..Default::default()
        };

        app.apply(Action::BeginCreate);
        assert!(
            matches!(
                &app.state,
                UiState::Locked {
                    form: LockForm::Create { passphrase, confirmation },
                    ..
                } if passphrase == "first-canary" && confirmation.is_empty()
            ),
            "beginning a creation keeps the phrase and asks for it again"
        );

        // A different entry creates nothing and says why.
        if let UiState::Locked {
            form: LockForm::Create { confirmation, .. },
            ..
        } = &mut app.state
        {
            *confirmation = "second-canary".to_owned();
        }
        app.apply(Action::Create);
        assert!(!database.exists(), "a mismatched confirmation must not write");
        assert!(matches!(
            &app.state,
            UiState::Locked {
                error: Some(message),
                ..
            } if message.contains("do not match")
        ));

        // The same entry twice writes the archive and unlocks it.
        if let UiState::Locked {
            form: LockForm::Create { confirmation, .. },
            ..
        } = &mut app.state
        {
            *confirmation = "first-canary".to_owned();
        }
        app.apply(Action::Create);
        assert!(database.exists());
        assert!(matches!(app.state, UiState::Unlocked { .. }));
    }

    #[test]
    fn backing_out_of_a_creation_wipes_the_confirmation() {
        let mut app = BonkApp {
            state: UiState::Locked {
                form: LockForm::Create {
                    passphrase: "first-canary".to_owned(),
                    confirmation: "first-canary".to_owned(),
                },
                error: Some("the passphrases do not match".to_owned()),
            },
            ..Default::default()
        };

        app.apply(Action::CancelCreate);

        assert!(matches!(
            &app.state,
            UiState::Locked {
                form: LockForm::Unlock { passphrase },
                error: None,
            } if passphrase == "first-canary"
        ));
    }

    #[test]
    fn a_recovery_key_from_the_app_opens_the_archive() {
        let dir = TempDir::new().unwrap();
        let database = dir.path().join("bonk.db");
        let keystore = dir.path().join("keystore.bin");
        let passphrase = "correct horse battery staple";
        let session = Session::create(&database, &keystore, passphrase).unwrap();

        let key = session.add_recovery_key(passphrase).unwrap();
        assert_eq!(key.split('-').count(), 8, "eight groups");
        assert_eq!(key.chars().filter(|symbol| *symbol != '-').count(), 32);
        assert!(KeyStore::open(&keystore, passphrase).unwrap().recovery_enabled());

        // The key is a real route in: it unwraps the data key the database needs.
        let recovered = KeyStore::open_with_recovery(&keystore, &key).unwrap();
        let reopened = Database::open(&database, &recovered).unwrap();
        assert!(reopened.seller_count().unwrap() == 0);

        let recorded: i64 = session
            .database()
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM audit_event WHERE kind = 'recovery_key_added'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(recorded, 1, "the chain must know a route was added");
    }

    #[test]
    fn changing_the_passphrase_from_the_app_keeps_the_archive_openable() {
        let dir = TempDir::new().unwrap();
        let database = dir.path().join("bonk.db");
        let keystore = dir.path().join("keystore.bin");
        let session = Session::create(&database, &keystore, "old passphrase").unwrap();

        // A wrong current passphrase changes nothing.
        assert!(session.change_passphrase("not it", "new passphrase").is_err());
        assert!(Session::unlock(&database, &keystore, "old passphrase").is_ok());

        session.change_passphrase("old passphrase", "new passphrase").unwrap();
        assert!(Session::unlock(&database, &keystore, "old passphrase").is_err());
        assert!(Session::unlock(&database, &keystore, "new passphrase").is_ok());
    }

    #[test]
    fn a_forgotten_passphrase_is_reset_from_the_lock_screen() {
        let dir = TempDir::new().unwrap();
        let database = dir.path().join("bonk.db");
        let keystore = dir.path().join("keystore.bin");
        let forgotten = "correct horse battery staple";
        let replacement = "a fresh and rather long phrase";

        let recovery_key = {
            let session = Session::create(&database, &keystore, forgotten).unwrap();
            session.add_recovery_key(forgotten).unwrap()
        };

        let mut app = BonkApp {
            database_path: database.display().to_string(),
            keystore_path: keystore.display().to_string(),
            state: UiState::Locked {
                form: LockForm::Reset {
                    recovery_key,
                    new_passphrase: replacement.to_owned(),
                    repeat: replacement.to_owned(),
                    replacement: None,
                },
                error: None,
            },
            ..Default::default()
        };

        app.apply(Action::Reset);

        let UiState::Locked {
            form: LockForm::Reset {
                replacement: new_key, ..
            },
            error,
        } = &app.state
        else {
            panic!("a successful reset stays on the screen until the key is acknowledged");
        };
        assert!(error.is_none());
        assert!(new_key.is_some(), "the replacement key must be shown");

        app.apply(Action::FinishReset);
        assert!(matches!(app.state, UiState::Unlocked { .. }));

        // The forgotten phrase is gone and the new one opens the archive.
        assert!(Session::unlock(&database, &keystore, forgotten).is_err());
        assert!(Session::unlock(&database, &keystore, replacement).is_ok());
    }

    #[test]
    fn a_wrong_recovery_key_leaves_the_archive_alone() {
        let dir = TempDir::new().unwrap();
        let database = dir.path().join("bonk.db");
        let keystore = dir.path().join("keystore.bin");
        let passphrase = "correct horse battery staple";

        {
            let session = Session::create(&database, &keystore, passphrase).unwrap();
            session.add_recovery_key(passphrase).unwrap();
        }

        let mut app = BonkApp {
            database_path: database.display().to_string(),
            keystore_path: keystore.display().to_string(),
            state: UiState::Locked {
                form: LockForm::Reset {
                    recovery_key: "AAAA-BBBB-CCCC-DDDD-EEEE-FFFF-GGGG-HHHH".to_owned(),
                    new_passphrase: "a fresh and rather long phrase".to_owned(),
                    repeat: "a fresh and rather long phrase".to_owned(),
                    replacement: None,
                },
                error: None,
            },
            ..Default::default()
        };

        app.apply(Action::Reset);

        assert!(matches!(
            &app.state,
            UiState::Locked {
                form: LockForm::Reset { replacement: None, .. },
                error: Some(message),
            } if message.contains("does not open")
        ));
        assert!(Session::unlock(&database, &keystore, passphrase).is_ok());
    }

    #[test]
    fn a_reset_must_be_well_formed() {
        assert_eq!(check_reset("", "new", "new"), Err("enter the recovery key"));
        assert_eq!(check_reset("key", " ", " "), Err("enter a new passphrase"));
        assert_eq!(
            check_reset("key", "new", "other"),
            Err("the new passphrases do not match")
        );
        assert_eq!(check_reset("key", "new", "new"), Ok(()));
    }

    #[test]
    fn backing_out_of_a_reset_returns_to_the_unlock_form() {
        let mut app = BonkApp {
            state: UiState::Locked {
                form: LockForm::Reset {
                    recovery_key: "key-canary".to_owned(),
                    new_passphrase: "phrase-canary".to_owned(),
                    repeat: "phrase-canary".to_owned(),
                    replacement: None,
                },
                error: Some("that recovery key does not open this archive".to_owned()),
            },
            ..Default::default()
        };

        app.apply(Action::CancelReset);

        assert!(matches!(
            &app.state,
            UiState::Locked {
                form: LockForm::Unlock { passphrase },
                error: None,
            } if passphrase.is_empty()
        ));
    }

    #[test]
    fn a_remembered_device_opens_the_archive_without_the_passphrase() {
        let dir = TempDir::new().unwrap();
        let database = dir.path().join("bonk.db");
        let keystore = dir.path().join("keystore.bin");
        let passphrase = "correct horse battery staple";
        let session = Session::create(&database, &keystore, passphrase).unwrap();
        let store = store::device::MemoryStore::default();

        assert!(!Session::device_ready(&keystore, &store));
        // A wrong passphrase cannot trust the device.
        assert!(session.remember_device("not the passphrase", &store).is_err());
        assert!(!Session::device_ready(&keystore, &store));

        session.remember_device(passphrase, &store).unwrap();
        assert!(Session::device_ready(&keystore, &store));

        // The keystore is not consulted: the device store is enough.
        let reopened = Session::unlock_with_device(&database, &keystore, &store).unwrap();
        assert_eq!(reopened.summary().unwrap().schema_version, 3);

        let trusted: i64 = reopened
            .database()
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM audit_event WHERE kind = 'device_trusted'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(trusted, 1, "a second way in belongs in the chain");
    }

    #[test]
    fn forgetting_the_device_removes_the_route() {
        let dir = TempDir::new().unwrap();
        let database = dir.path().join("bonk.db");
        let keystore = dir.path().join("keystore.bin");
        let passphrase = "correct horse battery staple";
        let session = Session::create(&database, &keystore, passphrase).unwrap();
        let store = store::device::MemoryStore::default();
        session.remember_device(passphrase, &store).unwrap();

        assert!(session.forget_device(&store).unwrap());
        assert!(!Session::device_ready(&keystore, &store));
        assert!(Session::unlock_with_device(&database, &keystore, &store).is_err());
        assert!(!session.forget_device(&store).unwrap(), "a second forget is a no-op");

        let forgotten: i64 = session
            .database()
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM audit_event WHERE kind = 'device_forgotten'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(forgotten, 1);

        // The passphrase still opens it, which is the point of turning this off.
        assert!(Session::unlock(&database, &keystore, passphrase).is_ok());
    }

    #[test]
    fn default_paths_are_under_one_directory() {
        let (database, keystore) = default_paths();
        assert_eq!(database.parent(), keystore.parent());
        assert_eq!(database.file_name().unwrap(), "bonk.db");
        assert_eq!(keystore.file_name().unwrap(), "keystore.bin");
    }
}
