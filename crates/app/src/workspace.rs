//! The unlocked workspace: onboarding, customers, and the invoice editor.
//!
//! The UI never touches SQL directly; it builds a domain draft, asks the domain
//! for totals and findings, and calls the same store functions as the CLI.

use crux::{
    Address, CountryCode, Currency, CustomerId, DocType, DocumentId, DraftLine, Finding, InvoiceDraft, LineId,
    LocalDate, Money, Party, PrecedingInvoice, Quantity, SavedSearchId, SellerId, Severity, TaxTreatment, UnitCode,
    VatCategory, VatRate, calculate_totals, has_blocking, validate_draft,
};
use store::{
    CustomerRecord, Direction, DocumentRecord, DocumentStatus, IssueRequest, SavedSearch, SearchHit, SearchRequest,
    SellerRecord, StoreError, Suppression, archive::DocumentView, attach_artefact, credit_note_for, first_seller,
    insert_customer, insert_seller, issue_draft, list_customers, list_suppressions, save_draft, save_suppression,
    search,
};
use zeroize::Zeroize;

use crate::Session;
use crate::health::Health;
use crate::text::editor::Field;
use crate::text::find::Fact;
use crate::text::{self, Language};

const ERROR_COLOR: egui::Color32 = egui::Color32::from_rgb(200, 60, 60);
const WARN_COLOR: egui::Color32 = egui::Color32::from_rgb(190, 140, 40);
const OK_COLOR: egui::Color32 = egui::Color32::from_rgb(70, 150, 90);

const CATEGORIES: [VatCategory; 6] = [
    VatCategory::Standard,
    VatCategory::ZeroRated,
    VatCategory::Exempt,
    VatCategory::ReverseCharge,
    VatCategory::IntraCommunity,
    VatCategory::Export,
];

/// What the shell should do after drawing the workspace.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WorkspaceAction {
    /// Keep working.
    None,
    /// Drop the keys and return to the unlock screen.
    Lock,
}

/// Unlocked application state.
pub struct Workspace {
    seller: Option<SellerRecord>,
    customers: Vec<CustomerRecord>,
    seller_form: SellerForm,
    customer_form: CustomerForm,
    show_customer_form: bool,
    editor: Option<Editor>,
    issued: Option<DocumentRecord>,
    pdf: Option<PdfDocument>,
    pdf_path: String,
    conformance_findings: Vec<Finding>,
    suppressions: Vec<Suppression>,
    suppress_target: Option<SuppressionTarget>,
    suppression_reason: String,
    focus_request: Option<String>,
    message: Option<Message>,
    search: SearchView,
    security: SecurityForm,
    /// Interface language, chosen once per workspace.
    language: Language,
    health: Option<Health>,
}

/// The find panel's state, kept apart from the drawing code so it can be tested.
#[derive(Default)]
struct SearchView {
    query: String,
    direction: Option<Direction>,
    status: Option<DocumentStatus>,
    results: Vec<SearchHit>,
    /// True once a query has run, which is what separates "nothing found" from
    /// "nothing asked yet".
    ran: bool,
    /// The result the user opened, read back from the archive.
    opened: Option<DocumentView>,
    /// Why the last attempt to open a result failed, if it did.
    open_error: Option<String>,
    /// The saved filters for this seller.
    saved: Vec<SavedSearch>,
    /// The name typed for "save current".
    save_name: String,
    /// Name of the saved filter last applied, so Delete has a target.
    applied: Option<String>,
    /// Why the last save, apply, or delete failed.
    saved_error: Option<String>,
}

impl SearchView {
    /// Runs what the panel describes and keeps the result, empty or not.
    fn run(&mut self, database: &store::Database, owner: SellerId) -> Result<(), StoreError> {
        let query = self.query.trim();
        let request = SearchRequest {
            query: (!query.is_empty()).then_some(query),
            direction: self.direction,
            from: None,
            to: None,
            status: self.status,
        };
        self.results = search(database.connection(), owner, &request)?;
        self.ran = true;
        Ok(())
    }

    /// Reads a result back from the archive and shows it.
    fn open(&mut self, database: &store::Database, id: DocumentId) {
        match store::archive::load(database.connection(), id) {
            Ok(Some(view)) => {
                self.opened = Some(view);
                self.open_error = None;
            }
            Ok(None) => {
                self.opened = None;
                self.open_error = Some("that document is no longer in this archive; run the search again".to_owned());
            }
            Err(error) => {
                self.opened = None;
                self.open_error = Some(error.to_string());
            }
        }
    }

    /// Reloads the saved filters for this seller.
    fn reload_saved(&mut self, database: &store::Database, owner: SellerId) {
        match store::saved::list(database.connection(), owner) {
            Ok(saved) => {
                self.saved = saved;
                self.saved_error = None;
            }
            Err(error) => self.saved_error = Some(error.to_string()),
        }
    }

    /// Stores what the panel currently asks for under the typed name.
    fn save_current(&mut self, database: &store::Database, owner: SellerId) {
        let search = SavedSearch {
            id: SavedSearchId::new(),
            name: self.save_name.trim().to_owned(),
            query: self.query.trim().to_owned(),
            direction: self.direction,
            status: self.status,
        };
        match store::saved::save(database.connection(), owner, &search, &store::now_rfc3339()) {
            Ok(()) => {
                self.save_name.clear();
                self.applied = Some(search.name);
                self.reload_saved(database, owner);
            }
            Err(error) => self.saved_error = Some(error.to_string()),
        }
    }

    /// Fills the panel from a saved search and runs it, so a stored view shows
    /// its results rather than only its filters.
    fn apply_saved(&mut self, name: &str, database: &store::Database, owner: SellerId) {
        let Some(found) = self.saved.iter().find(|search| search.name == name).cloned() else {
            return;
        };
        self.query = found.query;
        self.direction = found.direction;
        self.status = found.status;
        self.applied = Some(found.name);
        match self.run(database, owner) {
            Ok(()) => self.saved_error = None,
            Err(error) => self.saved_error = Some(error.to_string()),
        }
    }

    /// Deletes a saved search by name.
    fn delete_saved(&mut self, name: &str, database: &store::Database, owner: SellerId) {
        match store::saved::delete(database.connection(), owner, name) {
            Ok(true) => {
                if self.applied.as_deref() == Some(name) {
                    self.applied = None;
                }
                self.reload_saved(database, owner);
            }
            Ok(false) => {
                self.applied = None;
                self.reload_saved(database, owner);
            }
            Err(error) => self.saved_error = Some(error.to_string()),
        }
    }
}

#[derive(Clone)]
struct SuppressionTarget {
    code: String,
    field: String,
}

/// The security panel's state.
///
/// These are passphrases in plain `String`s because egui's `TextEdit` writes into
/// one; [`Workspace`] wipes them when it is dropped, which happens on lock.
#[derive(Default)]
struct SecurityForm {
    current: String,
    new_passphrase: String,
    repeat: String,
    /// The recovery key, shown once and wiped when the panel is done or the
    /// workspace closes.
    recovery_key: Option<String>,
}

impl SecurityForm {
    /// Empties every secret and keeps whatever the user still has to read.
    fn forget_secrets(&mut self) {
        self.current.zeroize();
        self.new_passphrase.zeroize();
        self.repeat.zeroize();
    }
}

enum FindingAction {
    Focus(String),
    Suppress { code: String, field: String },
}

struct PdfDocument {
    bytes: Vec<u8>,
    hash: String,
}

struct Message {
    text: String,
    error: bool,
}

impl Message {
    fn ok(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            error: false,
        }
    }

    fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            error: true,
        }
    }
}

struct SellerForm {
    name: String,
    city: String,
    street: String,
    postal: String,
    country: String,
    vat: String,
    kvk: String,
    peppol: String,
}

impl Default for SellerForm {
    fn default() -> Self {
        Self {
            name: String::new(),
            city: String::new(),
            street: String::new(),
            postal: String::new(),
            country: "NL".to_owned(),
            vat: String::new(),
            kvk: String::new(),
            peppol: String::new(),
        }
    }
}

#[derive(Default)]
struct CustomerForm {
    name: String,
    city: String,
    street: String,
    postal: String,
    country: String,
    vat: String,
    peppol: String,
}

struct Editor {
    id: DocumentId,
    doc_type: DocType,
    preceding: Option<PrecedingInvoice>,
    customer: Option<CustomerId>,
    issue_date: String,
    due_date: String,
    delivery_date: String,
    note: String,
    buyer_reference: String,
    order_reference: String,
    /// The language the printed document speaks, chosen per document.
    document_language: render::Language,
    lines: Vec<LineForm>,
}

impl Clone for Editor {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            doc_type: self.doc_type,
            preceding: self.preceding.clone(),
            customer: self.customer,
            issue_date: self.issue_date.clone(),
            due_date: self.due_date.clone(),
            delivery_date: self.delivery_date.clone(),
            note: self.note.clone(),
            buyer_reference: self.buyer_reference.clone(),
            order_reference: self.order_reference.clone(),
            document_language: self.document_language,
            lines: self.lines.clone(),
        }
    }
}

impl Editor {
    fn new(customer: Option<CustomerId>, document_language: render::Language) -> Self {
        Self {
            id: DocumentId::new(),
            doc_type: DocType::Invoice,
            preceding: None,
            customer,
            issue_date: LocalDate::today_utc().to_iso_string(),
            due_date: String::new(),
            delivery_date: String::new(),
            note: String::new(),
            buyer_reference: String::new(),
            order_reference: String::new(),
            document_language,
            lines: vec![LineForm::default()],
        }
    }

    /// An editor pre-filled from a credit-note draft copied from an issued
    /// invoice: the reference is fixed, the lines and customer are editable.
    fn credit_note(draft: &InvoiceDraft, document_language: render::Language) -> Option<Self> {
        let preceding = draft.preceding_invoice.clone()?;
        let mut editor = Self::new(Some(draft.customer_id), document_language);
        editor.id = draft.id;
        editor.doc_type = DocType::CreditNote;
        editor.preceding = Some(preceding);
        editor.issue_date = draft.issue_date.to_iso_string();
        editor.due_date.clear();
        editor.buyer_reference = draft.buyer_reference.clone().unwrap_or_default();
        editor.order_reference = draft.order_reference.clone().unwrap_or_default();
        editor.note = draft.note.clone().unwrap_or_default();
        editor.lines = draft
            .lines
            .iter()
            .map(|line| LineForm {
                name: line.name.clone(),
                quantity: line.quantity.to_trimmed_string(),
                unit_price: line.unit_price.to_trimmed_string(),
                rate: line.treatment.rate.percent_string(),
                category: line.treatment.category,
            })
            .collect();
        Some(editor)
    }
}

#[derive(Clone)]
struct LineForm {
    name: String,
    quantity: String,
    unit_price: String,
    rate: String,
    category: VatCategory,
}

impl Default for LineForm {
    fn default() -> Self {
        Self {
            name: String::new(),
            quantity: "1".to_owned(),
            unit_price: "0.00".to_owned(),
            rate: "21".to_owned(),
            category: VatCategory::Standard,
        }
    }
}

impl Drop for Workspace {
    /// The workspace holds passphrase fields because egui's `TextEdit` needs
    /// plain `String`s. Locking drops it, so the wipe belongs here as well as
    /// after a successful change.
    fn drop(&mut self) {
        self.security.forget_secrets();
        self.forget_recovery_key();
    }
}

impl Workspace {
    /// Loads the seller and customer list for a freshly unlocked session.
    pub fn new(session: &Session) -> Result<Self, StoreError> {
        let connection = session.database().connection();
        let mut workspace = Self {
            seller: first_seller(connection)?,
            customers: list_customers(connection)?,
            seller_form: SellerForm::default(),
            customer_form: CustomerForm::default(),
            show_customer_form: false,
            editor: None,
            issued: None,
            pdf: None,
            pdf_path: String::new(),
            conformance_findings: Vec::new(),
            suppressions: Vec::new(),
            suppress_target: None,
            suppression_reason: String::new(),
            focus_request: None,
            message: None,
            search: SearchView::default(),
            security: SecurityForm::default(),
            language: Language::default(),
            health: Health::snapshot(session.database(), session.database_path()).ok(),
        };
        if let Some(seller) = workspace.seller.as_ref().map(|seller| seller.id) {
            workspace.search.reload_saved(session.database(), seller);
        }
        Ok(workspace)
    }

    /// Draws the workspace.
    pub fn ui(&mut self, ui: &mut egui::Ui, session: &mut Session) -> WorkspaceAction {
        let mut lock = false;
        let mut focus = self.focus_request.take();
        if ui.input(|input| input.modifiers.command && input.key_pressed(egui::Key::F)) {
            focus = Some("search.query".to_owned());
        }

        ui.horizontal(|ui| {
            ui.heading(text::shell::PRODUCT);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(text::workspace::lock_action(self.language)).clicked() {
                    lock = true;
                }
            });
        });
        ui.label(text::workspace::database_line(
            self.language,
            &session.database_path().display().to_string(),
        ));
        if !store::cipher_enabled() {
            ui.colored_label(ERROR_COLOR, text::workspace::unencrypted_warning(self.language));
        }
        ui.separator();

        if let Some(message) = &self.message {
            let color = if message.error { ERROR_COLOR } else { OK_COLOR };
            ui.colored_label(color, message.text.as_str());
            ui.separator();
        }

        if !self.conformance_findings.is_empty() {
            if let Some(action) = show_findings(ui, self.language, &self.conformance_findings) {
                self.apply_finding_action(action);
            }
            ui.separator();
        }

        if self.seller.is_some() {
            self.search_ui(ui, session, focus.as_deref());
            ui.separator();
        }

        if self.seller.is_none() {
            self.seller_onboarding(ui, session, focus.as_deref());
        } else {
            self.unlocked_ui(ui, session, focus.as_deref());
        }

        ui.add_space(8.0);
        self.health_ui(ui, session);
        self.security_ui(ui, session);

        self.suppression_dialog(ui, session);

        if lock {
            WorkspaceAction::Lock
        } else {
            WorkspaceAction::None
        }
    }

    /// Draws the find panel and runs the query when the user asks for it.
    ///
    /// Nothing runs while typing: a search reads the whole index, and a key
    /// press per character would be work the user did not ask for. Enter or the
    /// button runs it, and an empty query lists what the filters allow.
    fn search_ui(&mut self, ui: &mut egui::Ui, session: &Session, focus: Option<&str>) {
        let language = self.language;
        let mut run = false;
        let mut clear = false;
        let mut opened = None;
        let mut apply_saved = None;
        let mut save_current = false;
        let mut delete_saved = false;
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.strong(text::find::title(language));
                ui.weak(text::find::shortcut(language));
            });
            ui.horizontal(|ui| {
                let response = text_field(ui, &mut self.search.query, "search.query", 300.0, focus);
                if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                    run = true;
                }
                direction_combo(ui, language, &mut self.search.direction);
                status_combo(ui, language, &mut self.search.status);
                if ui.button(text::find::search_action(language)).clicked() {
                    run = true;
                }
                if ui.button(text::find::clear_action(language)).clicked() {
                    clear = true;
                }
            });

            ui.horizontal(|ui| {
                ui.label(text::find::saved_label(language));
                let selected = self
                    .search
                    .applied
                    .clone()
                    .unwrap_or_else(|| text::find::no_saved(language).to_owned());
                egui::ComboBox::from_id_salt("search.saved")
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for search in &self.search.saved {
                            let chosen = self.search.applied.as_deref() == Some(search.name.as_str());
                            if ui.selectable_label(chosen, search.name.as_str()).clicked() {
                                apply_saved = Some(search.name.clone());
                            }
                        }
                    });
                ui.add(
                    egui::TextEdit::singleline(&mut self.search.save_name)
                        .hint_text(text::find::save_name_hint(language))
                        .desired_width(140.0),
                );
                if ui.button(text::find::save_current_action(language)).clicked() {
                    save_current = true;
                }
                let deletable = self.search.applied.is_some();
                if ui
                    .add_enabled(deletable, egui::Button::new(text::find::delete_action(language)))
                    .clicked()
                {
                    delete_saved = true;
                }
            });

            if let Some(message) = &self.search.saved_error {
                ui.colored_label(ERROR_COLOR, message.as_str());
            }

            if self.search.ran {
                if self.search.results.is_empty() {
                    ui.label(if self.search.query.trim().is_empty() {
                        text::find::empty_archive(language)
                    } else {
                        text::find::no_matches(language)
                    });
                } else {
                    let selected = self.search.opened.as_ref().map(|view| view.id);
                    if let Some(id) = search_results_ui(ui, language, &self.search.results, selected) {
                        opened = Some(id);
                    }
                }
            } else {
                ui.label(text::find::prompt(language));
            }

            if let Some(message) = &self.search.open_error {
                ui.colored_label(ERROR_COLOR, message.as_str());
            }

            let mut close = false;
            if let Some(view) = &self.search.opened {
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .id_salt("opened-document")
                    .show(ui, |ui| {
                        ui.group(|ui| close = document_panel(ui, language, view));
                    });
            }
            if close {
                self.search.opened = None;
            }
        });

        let owner = self.seller.as_ref().map(|seller| seller.id);
        if let Some(id) = opened {
            self.search.open(session.database(), id);
        }
        if clear {
            self.search = SearchView::default();
        }
        if run {
            let Some(owner) = owner else {
                return;
            };
            if let Err(error) = self.search.run(session.database(), owner) {
                self.message = Some(Message::error(error.to_string()));
            }
        }
        if let (Some(name), Some(owner)) = (apply_saved, owner) {
            self.search.apply_saved(&name, session.database(), owner);
        }
        if save_current && let Some(owner) = owner {
            self.search.save_current(session.database(), owner);
        }
        if delete_saved && let (Some(owner), Some(name)) = (owner, self.search.applied.clone()) {
            self.search.delete_saved(&name, session.database(), owner);
        }
    }

    /// The seller's locale is the sensible default for what gets printed, and
    /// only a default: the editor can change it per document.
    fn default_document_language(&self) -> render::Language {
        self.seller.as_ref().map_or_else(render::Language::default, |seller| {
            render::Language::from_locale(&seller.default_locale)
        })
    }

    /// Draws the health panel: cheap facts always, the payload audit on request.
    fn health_ui(&mut self, ui: &mut egui::Ui, session: &Session) {
        let mut check = false;
        egui::CollapsingHeader::new(text::health::panel_title(self.language))
            .default_open(false)
            .show(ui, |ui| {
                check = self.health_details_ui(ui);
            });

        if check {
            self.run_health_check(session);
        }
    }

    /// Draws the security panel: credential changes, which ask for the current
    /// passphrase rather than trusting the open window.
    ///
    /// Passphrase changes and recovery keys are the two things that protect the
    /// archive from its own user, so they are here rather than only in the CLI.
    fn security_ui(&mut self, ui: &mut egui::Ui, session: &Session) {
        egui::CollapsingHeader::new(text::security::panel_title(self.language))
            .default_open(false)
            .show(ui, |ui| self.security_details_ui(ui, session));
    }

    /// The security panel's contents, drawn directly by the headless test.
    fn security_details_ui(&mut self, ui: &mut egui::Ui, session: &Session) {
        let language = self.language;
        let mut change = false;
        let mut add_key = false;
        let mut done = false;

        ui.label(text::security::note(language));
        egui::Grid::new("security-form").num_columns(2).show(ui, |ui| {
            ui.label(text::security::current_label(language));
            ui.add(egui::TextEdit::singleline(&mut self.security.current).password(true));
            ui.end_row();
            ui.label(text::shell::new_passphrase_label(language));
            ui.add(egui::TextEdit::singleline(&mut self.security.new_passphrase).password(true));
            ui.end_row();
            ui.label("");
            crate::strength_row(ui, self.language, &self.security.new_passphrase);
            ui.end_row();
            ui.label(text::shell::repeat_new_passphrase_label(language));
            ui.add(egui::TextEdit::singleline(&mut self.security.repeat).password(true));
            ui.end_row();
        });
        ui.horizontal(|ui| {
            if ui.button(text::security::change_action(language)).clicked() {
                change = true;
            }
            if ui.button(text::security::add_key_action(language)).clicked() {
                add_key = true;
            }
        });

        if let Some(key) = &self.security.recovery_key {
            ui.add_space(6.0);
            ui.colored_label(WARN_COLOR, text::shell::keep_key_note(language));
            ui.label(egui::RichText::new(key.as_str()).monospace());
            if ui.button(text::security::done_action(language)).clicked() {
                done = true;
            }
        }

        if change {
            let verdict = check_password_change(
                &self.security.current,
                &self.security.new_passphrase,
                &self.security.repeat,
            );
            if let Err(reason) = verdict {
                self.message = Some(Message::error(reason));
            } else {
                match session.change_passphrase(&self.security.current, &self.security.new_passphrase) {
                    Ok(()) => {
                        self.security.forget_secrets();
                        self.message = Some(Message::ok(text::security::passphrase_changed(language)));
                    }
                    Err(error) => self.message = Some(Message::error(error.to_string())),
                }
            }
        }

        if add_key {
            if self.security.current.trim().is_empty() {
                self.message = Some(Message::error(text::security::enter_current(language)));
            } else {
                match session.add_recovery_key(&self.security.current) {
                    Ok(key) => {
                        self.security.recovery_key = Some(key);
                        self.security.forget_secrets();
                        self.message = Some(Message::ok(text::security::key_added(language)));
                    }
                    Err(error) => self.message = Some(Message::error(error.to_string())),
                }
            }
        }

        if done {
            self.forget_recovery_key();
        }
    }

    /// Drops the shown recovery key, wiping the bytes first.
    fn forget_recovery_key(&mut self) {
        if let Some(mut key) = self.security.recovery_key.take() {
            key.zeroize();
        }
    }

    /// The panel's contents. Returns true when the user asked for a check.
    fn health_details_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let language = self.language;
        let Some(health) = &self.health else {
            ui.label(text::health::unreadable(language));
            return false;
        };

        ui.horizontal(|ui| {
            ui.label(text::health::schema(language, health.schema_version));
            ui.label("·");
            ui.label(health.encryption);
            ui.label("·");
            ui.label(text::health::size(language, health.database_bytes / 1024));
        });

        match &health.chain {
            Some(head) => {
                let short = health.short_head().unwrap_or_default();
                ui.horizontal(|ui| {
                    ui.label(text::health::chain(language, head.seq));
                    ui.label(egui::RichText::new(short).monospace())
                        .on_hover_text(head.hash.as_str());
                });
            }
            None => {
                ui.label(text::health::empty_chain(language));
            }
        }

        match (&health.report, &health.error) {
            (_, Some(error)) => {
                ui.colored_label(ERROR_COLOR, text::health::integrity_failed(language, error));
            }
            (Some(report), None) if report.orphaned_blobs > 0 => {
                ui.colored_label(
                    WARN_COLOR,
                    text::health::integrity_with_orphans(language, report.blobs, report.orphaned_blobs),
                );
            }
            (Some(report), None) => {
                ui.colored_label(OK_COLOR, text::health::integrity_ok(language, report.blobs));
            }
            (None, None) => {
                ui.label(text::health::not_checked(language));
            }
        }
        if let Some(checked) = &health.checked_at {
            ui.weak(text::health::checked(language, checked));
        }

        match &health.last_backup {
            Some(backup) => {
                ui.label(text::health::last_backup(language, &backup.name, backup.bytes / 1024));
            }
            None => {
                ui.label(text::health::no_backup(language));
            }
        }

        ui.button(text::health::check_action(language)).clicked()
    }

    fn run_health_check(&mut self, session: &Session) {
        let now = store::now_rfc3339();
        match &mut self.health {
            Some(health) => health.check(session.database(), &now),
            None => {
                if let Ok(health) = Health::snapshot(session.database(), session.database_path()) {
                    self.health = Some(health);
                    if let Some(health) = &mut self.health {
                        health.check(session.database(), &now);
                    }
                }
            }
        }
    }

    fn apply_finding_action(&mut self, action: FindingAction) {
        match action {
            FindingAction::Focus(field) => self.focus_request = Some(field),
            FindingAction::Suppress { code, field } => {
                self.suppress_target = Some(SuppressionTarget { code, field });
                self.suppression_reason.clear();
            }
        }
    }

    fn suppression_dialog(&mut self, ui: &mut egui::Ui, session: &Session) {
        let Some(target) = self.suppress_target.clone() else {
            return;
        };
        let mut open = true;
        let mut record = false;
        let mut cancel = false;
        let language = self.language;
        egui::Window::new(text::editor::suppress_title(language))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                ui.label(text::editor::suppress_target(language, &target.code, &target.field));
                ui.label(text::editor::reason_label(language));
                ui.add(
                    egui::TextEdit::multiline(&mut self.suppression_reason)
                        .desired_width(360.0)
                        .desired_rows(3),
                );
                ui.horizontal(|ui| {
                    if ui.button(text::editor::record_suppression_action(language)).clicked() {
                        record = true;
                    }
                    if ui.button(text::editor::cancel_action(language)).clicked() {
                        cancel = true;
                    }
                });
            });
        if record {
            self.record_suppression(session);
        }
        if cancel || !open {
            self.suppress_target = None;
            self.suppression_reason.clear();
        }
    }

    fn record_suppression(&mut self, session: &Session) {
        let language = self.language;
        let Some(seller) = self.seller.clone() else {
            self.message = Some(Message::error(text::editor::seller_required_to_suppress(language)));
            return;
        };
        let Some(editor) = self.editor.clone() else {
            self.message = Some(Message::error(text::editor::invoice_required_to_suppress(language)));
            return;
        };
        let Some(target) = self.suppress_target.clone() else {
            return;
        };
        let reason = self.suppression_reason.trim();
        if reason.is_empty() {
            self.message = Some(Message::error(text::editor::suppression_reason_required(language)));
            return;
        }
        let draft = match build_draft(&editor, &seller) {
            Ok(draft) => draft,
            Err(error) => {
                self.message = Some(Message::error(error));
                return;
            }
        };

        let now = store::now_rfc3339();
        let connection = session.database().connection();
        if let Err(error) = save_draft(connection, &draft, &now) {
            self.message = Some(Message::error(error.to_string()));
            return;
        }
        match save_suppression(
            connection,
            draft.id,
            &target.code,
            &target.field,
            reason,
            "desktop",
            &now,
        ) {
            Ok(()) => match list_suppressions(connection, draft.id) {
                Ok(suppressions) => {
                    self.suppressions = suppressions;
                    self.suppress_target = None;
                    self.suppression_reason.clear();
                    self.message = Some(Message::ok(text::editor::warning_suppressed(language)));
                }
                Err(error) => self.message = Some(Message::error(error.to_string())),
            },
            Err(error) => self.message = Some(Message::error(error.to_string())),
        }
    }

    fn seller_onboarding(&mut self, ui: &mut egui::Ui, session: &Session, focus: Option<&str>) {
        let language = self.language;
        ui.heading(text::editor::seller_heading(language));
        ui.label(text::editor::seller_note(language));
        ui.add_space(8.0);

        egui::Grid::new("seller-form").num_columns(2).show(ui, |ui| {
            ui.label(text::editor::field(language, Field::LegalName));
            text_field(ui, &mut self.seller_form.name, "seller.name", 200.0, focus);
            ui.end_row();
            ui.label(text::editor::field(language, Field::Street));
            text_field(ui, &mut self.seller_form.street, "seller.street", 200.0, focus);
            ui.end_row();
            ui.label(text::editor::field(language, Field::PostCode));
            text_field(ui, &mut self.seller_form.postal, "seller.postal_code", 200.0, focus);
            ui.end_row();
            ui.label(text::editor::field(language, Field::City));
            text_field(ui, &mut self.seller_form.city, "seller.city", 200.0, focus);
            ui.end_row();
            ui.label(text::editor::field(language, Field::Country));
            text_field(ui, &mut self.seller_form.country, "seller.country", 200.0, focus);
            ui.end_row();
            ui.label(text::editor::field(language, Field::VatId));
            text_field(ui, &mut self.seller_form.vat, "seller.vat_id", 200.0, focus);
            ui.end_row();
            ui.label(text::editor::field(language, Field::Kvk));
            text_field(ui, &mut self.seller_form.kvk, "seller.kvk_number", 200.0, focus);
            ui.end_row();
            ui.label(text::editor::field(language, Field::Peppol));
            text_field(ui, &mut self.seller_form.peppol, "seller.peppol", 200.0, focus);
            ui.end_row();
        });

        ui.add_space(8.0);
        if ui.button(text::editor::create_seller_action(language)).clicked() {
            self.create_seller(session);
        }
    }

    fn unlocked_ui(&mut self, ui: &mut egui::Ui, session: &mut Session, focus: Option<&str>) {
        let language = self.language;
        let mut start_credit = false;
        if let Some(issued) = &self.issued {
            let credit_note = issued.number.contains("-C");
            ui.heading(text::editor::issued_heading(credit_note));
            ui.label(text::editor::number_line(language, &issued.number));
            ui.label(text::editor::issue_date_line(language, &issued.issue_date.to_string()));
            ui.label(text::editor::payable_line(language, &money_text(issued.totals.gross)));
            ui.label(text::editor::ubl_hash_line(language, &issued.ubl_sha256));
        }

        if self.issued.is_some() {
            let is_credit_note = self.issued.as_ref().is_some_and(|issued| issued.number.contains("-C"));
            let mut save_pdf = false;
            if let Some(pdf) = &self.pdf {
                ui.label(text::editor::pdf_hash_line(language, &pdf.hash));
                ui.horizontal(|ui| {
                    ui.label(text::editor::save_pdf_to(language));
                    ui.add(egui::TextEdit::singleline(&mut self.pdf_path).desired_width(420.0));
                    save_pdf = ui.button(text::editor::save_pdf_action(language)).clicked();
                });
            }

            ui.add_space(8.0);
            let mut reset = false;
            ui.horizontal(|ui| {
                if !is_credit_note && ui.button(text::editor::credit_note_action(language)).clicked() {
                    start_credit = true;
                }
                if ui.button(text::editor::new_invoice_action(language)).clicked() {
                    reset = true;
                }
            });
            if save_pdf {
                self.save_pdf();
            }
            if reset {
                self.reset_document();
            } else if start_credit {
                self.start_credit_note(session);
            }
            return;
        }

        if let Some(seller) = &self.seller {
            ui.label(text::editor::seller_line(language, &seller.party.legal_name));
        }

        ui.horizontal(|ui| {
            if ui.button(text::editor::new_invoice_action(language)).clicked() {
                let customer = self.customers.first().map(|customer| customer.id);
                self.editor = Some(Editor::new(customer, self.default_document_language()));
                self.suppressions.clear();
                self.suppress_target = None;
                self.suppression_reason.clear();
            }
            if ui.button(text::editor::add_customer_action(language)).clicked() {
                self.show_customer_form = !self.show_customer_form;
            }
        });

        if self.show_customer_form {
            ui.group(|ui| {
                ui.label(text::editor::new_customer_heading(language));
                egui::Grid::new("customer-form").num_columns(2).show(ui, |ui| {
                    ui.label(text::editor::field(language, Field::Name));
                    text_field(ui, &mut self.customer_form.name, "customer.name", 200.0, focus);
                    ui.end_row();
                    ui.label(text::editor::field(language, Field::Street));
                    text_field(ui, &mut self.customer_form.street, "customer.street", 200.0, focus);
                    ui.end_row();
                    ui.label(text::editor::field(language, Field::PostCode));
                    text_field(ui, &mut self.customer_form.postal, "customer.postal_code", 200.0, focus);
                    ui.end_row();
                    ui.label(text::editor::field(language, Field::City));
                    text_field(ui, &mut self.customer_form.city, "customer.city", 200.0, focus);
                    ui.end_row();
                    ui.label(text::editor::field(language, Field::Country));
                    text_field(ui, &mut self.customer_form.country, "customer.country", 200.0, focus);
                    ui.end_row();
                    ui.label(text::editor::field(language, Field::VatId));
                    text_field(ui, &mut self.customer_form.vat, "customer.vat_id", 200.0, focus);
                    ui.end_row();
                    ui.label(text::editor::field(language, Field::Peppol));
                    text_field(ui, &mut self.customer_form.peppol, "customer.peppol", 200.0, focus);
                    ui.end_row();
                });
                if ui.button(text::editor::save_customer_action(language)).clicked() {
                    self.create_customer(session);
                }
            });
        }

        if self.editor.is_some() {
            self.editor_ui(ui, session, focus);
        } else {
            ui.add_space(8.0);
            ui.label(text::editor::empty_editor(language));
        }
    }

    fn editor_ui(&mut self, ui: &mut egui::Ui, session: &mut Session, focus: Option<&str>) {
        let language = self.language;
        let Some(seller) = self.seller.clone() else {
            return;
        };
        let customers = self.customers.clone();
        let save;
        let issue;
        let mut remove_line = None;

        {
            let Some(editor) = self.editor.as_mut() else {
                return;
            };

            ui.separator();
            if editor.doc_type.is_credit_note() {
                let number = editor
                    .preceding
                    .as_ref()
                    .map(|preceding| preceding.number.as_str())
                    .unwrap_or_default();
                ui.colored_label(OK_COLOR, text::editor::credit_note_banner(language, number));
            }
            ui.horizontal(|ui| {
                ui.label(text::editor::customer_label(language));
                let selected = editor
                    .customer
                    .and_then(|id| customers.iter().find(|customer| customer.id == id))
                    .map(|customer| customer.party.legal_name.clone())
                    .unwrap_or_else(|| text::editor::select_customer(language).to_owned());
                egui::ComboBox::from_id_salt("customer")
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for customer in &customers {
                            ui.selectable_value(
                                &mut editor.customer,
                                Some(customer.id),
                                customer.party.legal_name.as_str(),
                            );
                        }
                    });
            });

            egui::Grid::new("header-form").num_columns(4).show(ui, |ui| {
                ui.label(text::editor::issue_date_label(language));
                text_field(ui, &mut editor.issue_date, "issue_date", 110.0, focus);
                ui.label(text::editor::due_date_label(language));
                text_field(ui, &mut editor.due_date, "due_date", 110.0, focus);
                ui.end_row();
                ui.label(text::editor::buyer_reference_label(language));
                text_field(ui, &mut editor.buyer_reference, "buyer_reference", 110.0, focus);
                ui.label(text::editor::order_reference_label(language));
                text_field(ui, &mut editor.order_reference, "order_reference", 110.0, focus);
                ui.end_row();
                ui.label(text::editor::delivery_date_label(language));
                text_field(ui, &mut editor.delivery_date, "delivery_date", 110.0, focus);
                ui.label(text::editor::document_language_label(language));
                egui::ComboBox::from_id_salt("editor.language")
                    .selected_text(match editor.document_language {
                        render::Language::English => "English",
                        render::Language::Dutch => "Nederlands",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut editor.document_language, render::Language::English, "English");
                        ui.selectable_value(&mut editor.document_language, render::Language::Dutch, "Nederlands");
                    });
                ui.end_row();
            });
            ui.horizontal(|ui| {
                ui.label(text::editor::note_label(language));
                text_field(ui, &mut editor.note, "note", 420.0, focus);
            });

            ui.add_space(8.0);
            egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                egui::Grid::new("lines-form")
                    .num_columns(6)
                    .striped(true)
                    .show(ui, |ui| {
                        for heading in text::editor::line_columns(language) {
                            ui.label(heading);
                        }
                        ui.label("");
                        ui.end_row();

                        for (index, line) in editor.lines.iter_mut().enumerate() {
                            text_field(ui, &mut line.name, &format!("lines[{index}].name"), 180.0, focus);
                            text_field(ui, &mut line.quantity, &format!("lines[{index}].quantity"), 60.0, focus);
                            text_field(
                                ui,
                                &mut line.unit_price,
                                &format!("lines[{index}].unit_price"),
                                80.0,
                                focus,
                            );
                            text_field(ui, &mut line.rate, &format!("lines[{index}].rate"), 50.0, focus);
                            let vat_field = format!("lines[{index}].vat_category");
                            let combo = egui::ComboBox::from_id_salt(("vat", index))
                                .selected_text(category_label(line.category))
                                .show_ui(ui, |ui| {
                                    for category in CATEGORIES {
                                        ui.selectable_value(&mut line.category, category, category_label(category));
                                    }
                                });
                            if focus == Some(vat_field.as_str()) {
                                combo.response.request_focus();
                            }
                            if ui.button(text::editor::remove_line_action(language)).clicked() {
                                remove_line = Some(index);
                            }
                            ui.end_row();
                        }
                    });
            });
            if let Some(index) = remove_line
                && editor.lines.len() > 1
            {
                editor.lines.remove(index);
            }
            if ui.button(text::editor::add_line_action(language)).clicked() {
                editor.lines.push(LineForm::default());
            }

            ui.add_space(8.0);
            save = ui.button(text::editor::save_draft_action(language)).clicked();
            issue = ui.button(text::editor::issue_action(language)).clicked();
        }

        let mut finding_action = None;
        if let Some(editor) = &self.editor {
            match build_draft(editor, &seller) {
                Ok(draft) => {
                    match calculate_totals(&draft.lines) {
                        Ok(totals) => {
                            ui.label(text::editor::totals(
                                language,
                                &money_text(totals.net),
                                &money_text(totals.tax),
                                &money_text(totals.gross),
                            ));
                        }
                        Err(error) => {
                            ui.colored_label(ERROR_COLOR, error.to_string());
                        }
                    }

                    match self
                        .customers
                        .iter()
                        .find(|customer| Some(customer.id) == editor.customer)
                    {
                        Some(customer) => {
                            let findings = validate_draft(&draft, &seller.party, &customer.party);
                            let suppressed = findings.iter().filter(|finding| self.is_suppressed(finding)).count();
                            let visible: Vec<Finding> = findings
                                .into_iter()
                                .filter(|finding| !self.is_suppressed(finding))
                                .collect();
                            if let Some(action) = show_findings(ui, language, &visible) {
                                finding_action = Some(action);
                            }
                            if suppressed > 0 {
                                ui.label(format!("{suppressed} warning(s) suppressed with a recorded reason"));
                            }
                        }
                        None => {
                            ui.colored_label(ERROR_COLOR, text::editor::select_customer_to_validate(language));
                        }
                    }
                }
                Err(error) => {
                    ui.colored_label(ERROR_COLOR, error);
                }
            }
        }
        if let Some(action) = finding_action {
            self.apply_finding_action(action);
        }

        if save {
            self.save_draft(session);
        }
        if issue {
            self.issue(session);
        }
    }

    fn is_suppressed(&self, finding: &Finding) -> bool {
        suppressed_by(finding, &self.suppressions)
    }

    fn reset_document(&mut self) {
        self.issued = None;
        self.editor = None;
        self.pdf = None;
        self.pdf_path.clear();
        self.conformance_findings.clear();
        self.suppressions.clear();
        self.suppress_target = None;
        self.suppression_reason.clear();
    }

    /// Opens an editor for a credit note that copies the issued invoice.
    fn start_credit_note(&mut self, session: &Session) {
        let (Some(issued), Some(seller)) = (&self.issued, &self.seller) else {
            return;
        };
        let number = issued.number.clone();
        let seller_id = seller.id;
        // A correction speaks the same language as the document it corrects.
        let document_language = self
            .editor
            .as_ref()
            .map_or_else(render::Language::default, |editor| editor.document_language);
        let language = self.language;
        let result = credit_note_for(
            session.database().connection(),
            seller_id,
            &number,
            LocalDate::today_utc(),
        );
        match result {
            Ok(Some(draft)) => match Editor::credit_note(&draft, document_language) {
                Some(editor) => {
                    self.reset_document();
                    self.editor = Some(editor);
                    self.message = Some(Message::ok(text::editor::credit_note_started(language, &number)));
                }
                None => {
                    self.message = Some(Message::error(text::editor::no_preceding_invoice(language)));
                }
            },
            Ok(None) => {
                self.message = Some(Message::error(text::editor::invoice_not_found(language, &number)));
            }
            Err(error) => self.message = Some(Message::error(error.to_string())),
        }
    }

    fn create_seller(&mut self, session: &Session) {
        let form = &self.seller_form;
        let country = match CountryCode::parse(form.country.trim()) {
            Ok(country) => country,
            Err(error) => {
                self.message = Some(Message::error(error.to_string()));
                return;
            }
        };
        let mut party = Party::new(
            form.name.trim().to_owned(),
            Address {
                street: nonempty(&form.street),
                city: form.city.trim().to_owned(),
                postal_code: nonempty(&form.postal),
                country,
            },
        );
        party.vat_id = nonempty(&form.vat).map(|value| value.to_ascii_uppercase());
        party.kvk_number = nonempty(&form.kvk);
        party.peppol_id = nonempty(&form.peppol);

        let record = SellerRecord {
            id: SellerId::new(),
            party,
            default_currency: Currency::EUR,
            default_locale: "nl-NL".to_owned(),
        };

        match insert_seller(session.database().connection(), &record, &store::now_rfc3339()) {
            Ok(()) => {
                self.seller = Some(record);
                self.seller_form = SellerForm::default();
                self.message = Some(Message::ok(text::editor::seller_created(self.language)));
            }
            Err(error) => self.message = Some(Message::error(error.to_string())),
        }
    }

    fn create_customer(&mut self, session: &Session) {
        let form = &self.customer_form;
        let country = match CountryCode::parse(form.country.trim()) {
            Ok(country) => country,
            Err(error) => {
                self.message = Some(Message::error(error.to_string()));
                return;
            }
        };
        let mut party = Party::new(
            form.name.trim().to_owned(),
            Address {
                street: nonempty(&form.street),
                city: form.city.trim().to_owned(),
                postal_code: nonempty(&form.postal),
                country,
            },
        );
        party.vat_id = nonempty(&form.vat).map(|value| value.to_ascii_uppercase());
        party.peppol_id = nonempty(&form.peppol);

        let record = CustomerRecord {
            id: CustomerId::new(),
            party,
        };
        match insert_customer(session.database().connection(), &record, &store::now_rfc3339()) {
            Ok(()) => {
                self.customers.push(record);
                self.customer_form = CustomerForm::default();
                self.show_customer_form = false;
                if let Some(editor) = &mut self.editor
                    && editor.customer.is_none()
                {
                    editor.customer = self.customers.last().map(|customer| customer.id);
                }
                self.message = Some(Message::ok(text::editor::customer_created(self.language)));
            }
            Err(error) => self.message = Some(Message::error(error.to_string())),
        }
    }

    fn save_draft(&mut self, session: &Session) {
        let Some(seller) = self.seller.clone() else {
            return;
        };
        let Some(editor) = self.editor.clone() else {
            return;
        };
        let draft = match build_draft(&editor, &seller) {
            Ok(draft) => draft,
            Err(error) => {
                self.message = Some(Message::error(error));
                return;
            }
        };

        match save_draft(session.database().connection(), &draft, &store::now_rfc3339()) {
            Ok(()) => {
                self.message = Some(Message::ok(text::editor::draft_saved(self.language)));
            }
            Err(error) => self.message = Some(Message::error(error.to_string())),
        }
    }

    fn issue(&mut self, session: &mut Session) {
        self.conformance_findings.clear();
        let Some(seller) = self.seller.clone() else {
            return;
        };
        let Some(editor) = self.editor.clone() else {
            return;
        };
        let language = editor.document_language;
        let draft = match build_draft(&editor, &seller) {
            Ok(draft) => draft,
            Err(error) => {
                self.message = Some(Message::error(error));
                return;
            }
        };
        let customer = match self.customers.iter().find(|customer| customer.id == draft.customer_id) {
            Some(customer) => customer.clone(),
            None => {
                self.message = Some(Message::error(text::editor::select_customer_required(self.language)));
                return;
            }
        };

        let findings = validate_draft(&draft, &seller.party, &customer.party);
        if has_blocking(&findings) {
            let count = findings
                .iter()
                .filter(|finding| finding.severity == Severity::Blocking)
                .count();
            self.message = Some(Message::error(text::editor::blocking_findings(self.language, count)));
            return;
        }

        let now = store::now_rfc3339();
        if let Err(error) = save_draft(session.database().connection(), &draft, &now) {
            self.message = Some(Message::error(error.to_string()));
            return;
        }

        let result = issue_draft(
            session.database_mut().connection_mut(),
            IssueRequest {
                draft_id: draft.id,
                actor: "desktop",
                device: "desktop",
                renderer_version: "syntax-0.1",
                now: &now,
            },
            crate::render_issue,
        );

        match result {
            Ok(issued) => {
                let mut attach_error = None;
                match crate::invoice_pdf(&issued.number, &draft, &seller, &customer, &issued.totals, language) {
                    Ok(bytes) => {
                        let hash = match attach_artefact(
                            session.database().connection(),
                            issued.id,
                            "pdf",
                            &bytes,
                            "typst-cli-0.14",
                            &now,
                        ) {
                            Ok(hash) => hash,
                            Err(error) => {
                                attach_error = Some(error.to_string());
                                String::new()
                            }
                        };
                        self.pdf_path = default_pdf_path(session, &issued.number);
                        self.pdf = Some(PdfDocument { bytes, hash });
                    }
                    Err(error) => {
                        self.pdf = None;
                        self.pdf_path.clear();
                        self.message = Some(Message::ok(text::editor::issued_without_pdf(
                            self.language,
                            &error.to_string(),
                        )));
                    }
                }

                self.issued = Some(issued);
                self.editor = None;
                self.conformance_findings.clear();
                if let Some(error) = attach_error {
                    self.message = Some(Message::error(text::editor::issued_with_unattached_pdf(
                        self.language,
                        &error,
                    )));
                } else if self.pdf.is_some() {
                    self.message = None;
                }
            }
            Err(StoreError::Conformance(findings)) => {
                let count = findings
                    .iter()
                    .filter(|finding| finding.severity == Severity::Blocking)
                    .count();
                self.conformance_findings = findings;
                self.message = Some(Message::error(text::editor::conformance_findings(self.language, count)));
            }
            Err(error) => self.message = Some(Message::error(error.to_string())),
        }
    }

    fn save_pdf(&mut self) {
        let Some(pdf) = &self.pdf else {
            return;
        };
        let path = self.pdf_path.trim();
        if path.is_empty() {
            self.message = Some(Message::error(text::editor::choose_pdf_path(self.language)));
            return;
        }
        match std::fs::write(path, &pdf.bytes) {
            Ok(()) => self.message = Some(Message::ok(text::editor::pdf_saved(self.language, path))),
            Err(error) => {
                self.message = Some(Message::error(text::editor::pdf_save_failed(
                    self.language,
                    &error.to_string(),
                )))
            }
        }
    }
}

fn default_pdf_path(session: &Session, number: &str) -> String {
    let directory = session.database_path().parent().unwrap_or(std::path::Path::new("."));
    directory.join(format!("{number}.pdf")).display().to_string()
}

fn build_draft(editor: &Editor, seller: &SellerRecord) -> Result<InvoiceDraft, String> {
    let customer_id = editor.customer.ok_or_else(|| "Select a customer.".to_owned())?;
    let issue_date = if editor.issue_date.trim().is_empty() {
        LocalDate::today_utc()
    } else {
        LocalDate::parse_iso(editor.issue_date.trim()).map_err(|_| "Issue date must be YYYY-MM-DD.".to_owned())?
    };
    let due_date = if editor.due_date.trim().is_empty() {
        None
    } else {
        Some(LocalDate::parse_iso(editor.due_date.trim()).map_err(|_| "Due date must be YYYY-MM-DD.".to_owned())?)
    };
    let delivery_date = if editor.delivery_date.trim().is_empty() {
        None
    } else {
        Some(
            LocalDate::parse_iso(editor.delivery_date.trim())
                .map_err(|_| "Delivery date must be YYYY-MM-DD.".to_owned())?,
        )
    };

    let mut draft = InvoiceDraft::new(seller.id, customer_id, seller.default_currency, issue_date);
    draft.doc_type = editor.doc_type;
    draft.preceding_invoice = editor.preceding.clone();
    if !editor.doc_type.is_credit_note() {
        draft.due_date = due_date;
    }
    draft.delivery_date = delivery_date;
    draft.note = nonempty(&editor.note);
    draft.buyer_reference = nonempty(&editor.buyer_reference);
    draft.order_reference = nonempty(&editor.order_reference);

    for (index, line) in editor.lines.iter().enumerate() {
        let position = u32::try_from(index + 1).map_err(|_| "Too many lines.".to_owned())?;
        let name = line.name.trim();
        if name.is_empty() {
            return Err(format!("Line {position}: item name is required."));
        }
        let quantity = Quantity::parse(line.quantity.trim()).map_err(|error| format!("Line {position}: {error}"))?;
        let unit_price = Money::parse_fixed(line.unit_price.trim(), seller.default_currency)
            .map_err(|error| format!("Line {position}: {error}"))?;
        let rate = VatRate::parse_percent(line.rate.trim()).map_err(|error| format!("Line {position}: {error}"))?;

        draft.lines.push(DraftLine {
            id: LineId::new(),
            position,
            name: name.to_owned(),
            description: None,
            quantity,
            unit: UnitCode::PIECE,
            unit_price,
            price_base_quantity: Quantity::ONE,
            treatment: TaxTreatment {
                category: line.category,
                rate,
                exemption_reason: None,
                exemption_reason_code: None,
            },
        });
    }

    Ok(draft)
}

fn suppressed_by(finding: &Finding, suppressions: &[Suppression]) -> bool {
    finding.severity == Severity::Warning
        && suppressions
            .iter()
            .any(|suppression| suppression.code == finding.code && suppression.field == finding.field)
}

fn text_field(ui: &mut egui::Ui, value: &mut String, field: &str, width: f32, focus: Option<&str>) -> egui::Response {
    let response = ui.add(
        egui::TextEdit::singleline(value)
            .id(egui::Id::unique(field))
            .desired_width(width),
    );
    if focus == Some(field) {
        response.request_focus();
    }
    response
}

fn direction_combo(ui: &mut egui::Ui, language: Language, value: &mut Option<Direction>) {
    let label = value
        .map(|direction| text::find::direction(language, direction))
        .unwrap_or_else(|| text::find::any_side(language));
    egui::ComboBox::from_id_salt("search.direction")
        .selected_text(label)
        .show_ui(ui, |ui| {
            ui.selectable_value(value, None, text::find::any_side(language));
            ui.selectable_value(
                value,
                Some(Direction::Outbound),
                text::find::direction(language, Direction::Outbound),
            );
            ui.selectable_value(
                value,
                Some(Direction::Inbound),
                text::find::direction(language, Direction::Inbound),
            );
        });
}

fn status_combo(ui: &mut egui::Ui, language: Language, value: &mut Option<DocumentStatus>) {
    let label = value
        .map(|status| text::find::status(language, status))
        .unwrap_or_else(|| text::find::any_status(language));
    egui::ComboBox::from_id_salt("search.status")
        .selected_text(label)
        .show_ui(ui, |ui| {
            ui.selectable_value(value, None, text::find::any_status(language));
            for status in [
                DocumentStatus::Issued,
                DocumentStatus::Received,
                DocumentStatus::Approved,
                DocumentStatus::Paid,
                DocumentStatus::Archived,
            ] {
                ui.selectable_value(value, Some(status), text::find::status(language, status));
            }
        });
}

fn search_results_ui(
    ui: &mut egui::Ui,
    language: Language,
    results: &[SearchHit],
    selected: Option<DocumentId>,
) -> Option<DocumentId> {
    let mut open = None;
    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
        egui::Grid::new("search-results")
            .num_columns(6)
            .striped(true)
            .show(ui, |ui| {
                for heading in text::find::columns(language) {
                    ui.strong(heading);
                }
                ui.end_row();
                for hit in results {
                    // The number is the handle: a whole row of labels cannot be
                    // clicked, and a separate button would need a column.
                    if ui
                        .selectable_label(selected == Some(hit.id), hit.number.as_str())
                        .clicked()
                    {
                        open = Some(hit.id);
                    }
                    ui.label(text::find::direction(language, hit.direction));
                    ui.label(hit.party.as_str());
                    ui.label(hit.issue_date.to_iso_string());
                    ui.label(format!("{} {}", money_text(hit.gross), hit.currency.as_str()));
                    ui.label(text::find::status(language, hit.status));
                    ui.end_row();
                }
            });
    });
    ui.weak(text::find::footer(language, results.len(), store::SEARCH_LIMIT));
    open
}

/// Draws a stored document read-only. Returns true when the user closes it.
fn document_panel(ui: &mut egui::Ui, language: Language, view: &DocumentView) -> bool {
    let mut close = false;
    ui.horizontal(|ui| {
        ui.heading(text::find::document_title(
            language,
            &view.number,
            view.doc_type.is_credit_note(),
        ));
        if ui.button(text::find::close_action(language)).clicked() {
            close = true;
        }
    });
    ui.weak(text::find::stored_note(language));

    egui::Grid::new("document-facts")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            fact(
                ui,
                language,
                Fact::Side,
                text::find::direction(language, view.direction),
            );
            fact(ui, language, Fact::Status, text::find::status(language, view.status));
            fact(ui, language, Fact::Issued, &view.issue_date.to_iso_string());
            if let Some(due) = view.due_date {
                fact(ui, language, Fact::Due, &due.to_iso_string());
            }
            if let Some(delivery) = view.delivery_date {
                fact(ui, language, Fact::Delivered, &delivery.to_iso_string());
            }
            if let Some(reference) = &view.buyer_reference {
                fact(ui, language, Fact::BuyerReference, reference);
            }
            if let Some(reference) = &view.order_reference {
                fact(ui, language, Fact::OrderReference, reference);
            }
            fact(ui, language, Fact::Seller, &view.seller.legal_name);
            fact(ui, language, Fact::Buyer, &view.buyer.legal_name);
            if let Some(note) = &view.note {
                fact(ui, language, Fact::Note, note);
            }
            fact(ui, language, Fact::Recorded, &view.recorded_at);
            fact(ui, language, Fact::SemanticHash, &view.semantic_hash);
        });

    ui.add_space(6.0);
    egui::Grid::new("document-lines")
        .num_columns(6)
        .striped(true)
        .show(ui, |ui| {
            for heading in text::find::line_columns(language) {
                ui.strong(heading);
            }
            ui.end_row();
            for line in &view.lines {
                ui.label(line.position.to_string());
                ui.label(line.name.as_str());
                ui.label(format!("{} {}", line.quantity.to_trimmed_string(), line.unit.as_str()));
                ui.label(money_text(line.unit_price));
                ui.label(line.vat_rate.percent_string());
                ui.label(money_text(line.line_net));
                ui.end_row();
            }
        });

    ui.add_space(6.0);
    ui.label(text::find::totals(
        language,
        &money_text(view.totals.net),
        &money_text(view.totals.tax),
        &money_text(view.totals.gross),
    ));

    if !view.events.is_empty() {
        ui.add_space(6.0);
        ui.strong(text::find::history_label(language));
        for event in &view.events {
            ui.label(format!("{} · {} · {}", event.at, event.kind, event.actor));
        }
    }

    if !view.artefacts.is_empty() {
        ui.add_space(6.0);
        ui.strong(text::find::artefacts_label(language));
        for artefact in &view.artefacts {
            ui.label(text::find::artefact_line(
                language,
                &artefact.kind,
                artefact.size_bytes,
                &artefact.bytes_hash,
            ));
        }
    }

    close
}

fn fact(ui: &mut egui::Ui, language: Language, fact: Fact, value: &str) {
    ui.weak(text::find::fact(language, fact));
    ui.label(value);
    ui.end_row();
}

fn show_findings(ui: &mut egui::Ui, language: Language, findings: &[Finding]) -> Option<FindingAction> {
    let mut action = None;
    for finding in findings.iter().take(12) {
        let color = match finding.severity {
            Severity::Blocking => ERROR_COLOR,
            Severity::Warning => WARN_COLOR,
            Severity::Info => egui::Color32::GRAY,
        };
        let bt = finding.bt.unwrap_or("-");
        let text = format!("{} [{}] {}: {}", finding.code, bt, finding.field, finding.message);
        let can_focus = focusable(&finding.field);
        let can_suppress = finding.severity == Severity::Warning;
        if can_focus || can_suppress {
            ui.horizontal(|ui| {
                ui.colored_label(color, text);
                if can_focus && ui.small_button(text::editor::focus_action(language)).clicked() {
                    action = Some(FindingAction::Focus(finding.field.clone()));
                }
                if can_suppress && ui.small_button(text::editor::suppress_action(language)).clicked() {
                    action = Some(FindingAction::Suppress {
                        code: finding.code.to_owned(),
                        field: finding.field.clone(),
                    });
                }
            });
        } else {
            ui.colored_label(color, text);
        }
    }
    action
}

fn focusable(field: &str) -> bool {
    if matches!(
        field,
        "issue_date"
            | "due_date"
            | "delivery_date"
            | "buyer_reference"
            | "order_reference"
            | "note"
            | "seller.name"
            | "seller.city"
            | "seller.country"
            | "seller.vat_id"
            | "seller.kvk_number"
            | "customer.name"
            | "customer.city"
            | "customer.country"
            | "customer.vat_id"
    ) {
        return true;
    }
    field.strip_prefix("lines[").is_some_and(|rest| {
        rest.split_once("].")
            .is_some_and(|(_, suffix)| matches!(suffix, "name" | "quantity" | "unit_price" | "rate" | "vat_category"))
    })
}

fn category_label(category: VatCategory) -> &'static str {
    match category {
        VatCategory::Standard => "S standard",
        VatCategory::ZeroRated => "Z zero rated",
        VatCategory::Exempt => "E exempt",
        VatCategory::ReverseCharge => "AE reverse charge",
        VatCategory::IntraCommunity => "K intra-EU",
        VatCategory::Export => "G export",
        VatCategory::OutsideScope => "O outside scope",
        VatCategory::CanaryIslands => "L Canary",
        VatCategory::CeutaMelilla => "M Ceuta",
    }
}

/// Whether a passphrase change is well formed, with the reason when it is not.
fn check_password_change(current: &str, new: &str, repeat: &str) -> Result<(), &'static str> {
    if current.trim().is_empty() {
        return Err("enter the current passphrase");
    }
    if new.trim().is_empty() {
        return Err("enter a new passphrase");
    }
    if new != repeat {
        return Err("the new passphrases do not match");
    }
    if new == current {
        return Err("the new passphrase must differ from the current one");
    }
    Ok(())
}

fn money_text(value: Money) -> String {
    match value.to_fixed_string(2) {
        Ok(text) => format!("{} {}", text, value.currency()),
        Err(_) => "0.00".to_owned(),
    }
}

fn nonempty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crux::Address;
    use store::{Database, KdfParams, KeyStore, insert_draft};

    const NOW: &str = "2026-10-28T00:00:00Z";

    fn seller() -> SellerRecord {
        SellerRecord {
            id: SellerId::new(),
            party: Party::new("Seller BV", Address::new("Amsterdam", CountryCode::NL)),
            default_currency: Currency::EUR,
            default_locale: "nl-NL".to_owned(),
        }
    }

    fn editor() -> Editor {
        let mut editor = Editor::new(Some(CustomerId::new()), render::Language::default());
        editor.lines[0].name = "Consulting".to_owned();
        editor.lines[0].quantity = "2".to_owned();
        editor.lines[0].unit_price = "50.00".to_owned();
        editor
    }

    /// A seller, one customer, and one issued invoice.
    fn seeded_archive() -> (tempfile::TempDir, Database, SellerId) {
        let dir = tempfile::TempDir::new().unwrap();
        let keystore = KeyStore::create(dir.path().join("keystore.bin"), "pass", KdfParams::TEST).unwrap();
        let mut database = Database::create(dir.path().join("bonk.db"), &keystore).unwrap();

        let seller = seller();
        insert_seller(database.connection(), &seller, NOW).unwrap();

        let customer = CustomerRecord {
            id: CustomerId::new(),
            party: Party::new("Acme BV", Address::new("Rotterdam", CountryCode::NL)),
        };
        insert_customer(database.connection(), &customer, NOW).unwrap();

        let mut form = editor();
        form.customer = Some(customer.id);
        form.buyer_reference = "PO-1".to_owned();
        let draft = build_draft(&form, &seller).unwrap();
        insert_draft(database.connection(), &draft, NOW).unwrap();
        issue_draft(
            database.connection_mut(),
            IssueRequest {
                draft_id: draft.id,
                actor: "test",
                device: "test",
                renderer_version: "test",
                now: NOW,
            },
            |_| Ok(b"<Invoice/>".to_vec()),
        )
        .unwrap();

        (dir, database, seller.id)
    }

    #[test]
    fn the_find_panel_runs_what_it_describes() {
        let (_dir, database, seller_id) = seeded_archive();

        let mut view = SearchView {
            query: "acme".to_owned(),
            ..SearchView::default()
        };
        view.run(&database, seller_id).unwrap();
        assert!(view.ran);
        assert_eq!(view.results.len(), 1);
        assert_eq!(view.results[0].party, "Acme BV");
        assert_eq!(view.results[0].status, DocumentStatus::Issued);

        view.query = "nothing-matches-this".to_owned();
        view.run(&database, seller_id).unwrap();
        assert!(view.results.is_empty(), "a run with no hits is still a run");

        // A blank query lists what the filters allow, which is how the panel
        // doubles as a browse view.
        view.query = "   ".to_owned();
        view.run(&database, seller_id).unwrap();
        assert_eq!(view.results.len(), 1);

        view.direction = Some(Direction::Inbound);
        view.run(&database, seller_id).unwrap();
        assert!(view.results.is_empty(), "the side filter is applied");
    }

    #[test]
    fn a_result_opens_read_only_with_its_lines_and_totals() {
        let (_dir, database, seller_id) = seeded_archive();

        let mut view = SearchView::default();
        view.run(&database, seller_id).unwrap();
        let hit = view.results[0].clone();

        view.open(&database, hit.id);

        let opened = view.opened.as_ref().expect("the result must open");
        assert!(view.open_error.is_none());
        assert_eq!(opened.id, hit.id);
        assert_eq!(opened.number, hit.number);
        assert_eq!(opened.buyer.legal_name, "Acme BV");
        assert_eq!(opened.lines.len(), 1);
        assert_eq!(opened.totals.gross, hit.gross);
        assert_eq!(opened.artefacts.len(), 1);
        assert_eq!(opened.status, DocumentStatus::Issued);
    }

    #[test]
    fn a_saved_search_can_be_stored_and_applied() {
        let (_dir, database, seller_id) = seeded_archive();

        let mut view = SearchView {
            query: "acme".to_owned(),
            save_name: "Acme".to_owned(),
            ..SearchView::default()
        };
        view.save_current(&database, seller_id);
        assert!(view.saved_error.is_none());
        assert_eq!(view.saved.len(), 1);
        assert_eq!(view.saved[0].name, "Acme");

        // The panel moves on; applying the saved search brings the filters back
        // and runs them, so a saved view shows results and not just fields.
        view.query = "nothing-matches-this".to_owned();
        view.run(&database, seller_id).unwrap();
        assert!(view.results.is_empty());

        view.apply_saved("Acme", &database, seller_id);
        assert_eq!(view.query, "acme");
        assert_eq!(view.results.len(), 1);

        view.delete_saved("Acme", &database, seller_id);
        assert!(view.saved.is_empty());
        assert!(view.applied.is_none());
    }

    #[test]
    fn a_saved_search_keeps_the_side_filter() {
        let (_dir, database, seller_id) = seeded_archive();
        let mut view = SearchView {
            direction: Some(Direction::Inbound),
            save_name: "Bills".to_owned(),
            ..SearchView::default()
        };
        view.save_current(&database, seller_id);
        assert_eq!(view.saved[0].direction, Some(Direction::Inbound));

        view.direction = None;
        view.apply_saved("Bills", &database, seller_id);
        assert_eq!(view.direction, Some(Direction::Inbound));
        assert!(view.results.is_empty(), "the only document in the fixture was issued");
    }

    #[test]
    fn a_saved_search_needs_a_name_in_the_panel_too() {
        let (_dir, database, seller_id) = seeded_archive();
        let mut view = SearchView {
            query: "acme".to_owned(),
            save_name: "  ".to_owned(),
            ..SearchView::default()
        };

        view.save_current(&database, seller_id);

        assert!(view.saved.is_empty());
        assert!(view.saved_error.as_deref().unwrap_or_default().contains("needs a name"));
    }

    #[test]
    fn opening_a_missing_document_says_so() {
        let (_dir, database, _seller_id) = seeded_archive();
        let mut view = SearchView::default();

        view.open(&database, DocumentId::new());

        assert!(view.opened.is_none());
        assert!(
            view.open_error
                .as_deref()
                .unwrap_or_default()
                .contains("no longer in this archive")
        );
    }

    #[test]
    fn the_workspace_draws_without_panicking() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut session = Session::create(
            dir.path().join("bonk.db"),
            dir.path().join("keystore.bin"),
            "passphrase",
        )
        .unwrap();
        let seller = seller();
        insert_seller(session.database().connection(), &seller, NOW).unwrap();
        let customer = CustomerRecord {
            id: CustomerId::new(),
            party: Party::new("Acme BV", Address::new("Rotterdam", CountryCode::NL)),
        };
        insert_customer(session.database().connection(), &customer, NOW).unwrap();
        let mut form = editor();
        form.customer = Some(customer.id);
        let draft = build_draft(&form, &seller).unwrap();
        insert_draft(session.database().connection(), &draft, NOW).unwrap();
        let issued = issue_draft(
            session.database_mut().connection_mut(),
            IssueRequest {
                draft_id: draft.id,
                actor: "test",
                device: "test",
                renderer_version: "test",
                now: NOW,
            },
            |_| Ok(b"<Invoice/>".to_vec()),
        )
        .unwrap();

        let mut workspace = Workspace::new(&session).unwrap();
        // An opened result draws a grid of facts, a line table, and hashes; the
        // panel is closed by default, so open it here to exercise it.
        workspace.search.opened = store::archive::load(session.database().connection(), issued.id).unwrap();
        // Likewise the security panel, including the block that shows a key once.
        workspace.security.current = "passphrase".to_owned();
        workspace.security.recovery_key = Some("AAAA-BBBB".to_owned());

        let context = egui::Context::default();
        let mut action = WorkspaceAction::None;
        // Drawing is where duplicate widget ids and missing fonts show up as a
        // panic, so a headless frame is worth more than it looks.
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            action = workspace.ui(ui, &mut session);
            // Panels are collapsed by default, so draw their contents directly:
            // a broken widget inside a closed header is still a broken widget.
            let _ = workspace.health_details_ui(ui);
            workspace.security_details_ui(ui, &session);
        });
        output.drop_without_applying_deltas();
        assert_eq!(action, WorkspaceAction::None);
    }

    #[test]
    fn a_passphrase_change_must_be_well_formed() {
        assert_eq!(
            check_password_change("", "new", "new"),
            Err("enter the current passphrase")
        );
        assert_eq!(check_password_change("old", "  ", "  "), Err("enter a new passphrase"));
        assert_eq!(
            check_password_change("old", "new", "other"),
            Err("the new passphrases do not match")
        );
        assert_eq!(
            check_password_change("same", "same", "same"),
            Err("the new passphrase must differ from the current one")
        );
        assert_eq!(check_password_change("old", "new", "new"), Ok(()));
    }

    #[test]
    fn the_printed_document_speaks_the_chosen_language() {
        let seller = seller();
        let customer = CustomerRecord {
            id: CustomerId::new(),
            party: Party::new("Acme BV", Address::new("Rotterdam", CountryCode::NL)),
        };
        let draft = build_draft(&editor(), &seller).unwrap();
        let totals = calculate_totals(&draft.lines).unwrap();

        let dutch = crate::pdf_data(
            "2026-0001",
            &draft,
            &seller,
            &customer,
            &totals,
            render::Language::Dutch,
        );
        assert_eq!(dutch.title, "Factuur");
        assert_eq!(dutch.labels.vat, "BTW");
        assert_eq!(dutch.labels.bill_to, "Factuur aan");

        let english = crate::pdf_data(
            "2026-0001",
            &draft,
            &seller,
            &customer,
            &totals,
            render::Language::English,
        );
        assert_eq!(english.title, "Invoice");
        assert_eq!(english.labels.vat, "VAT");
    }

    #[test]
    fn a_credit_note_is_named_as_one() {
        let seller = seller();
        let customer = CustomerRecord {
            id: CustomerId::new(),
            party: Party::new("Acme BV", Address::new("Rotterdam", CountryCode::NL)),
        };
        let mut editor = editor();
        editor.doc_type = DocType::CreditNote;
        let draft = build_draft(&editor, &seller).unwrap();
        let totals = calculate_totals(&draft.lines).unwrap();

        for (language, expected) in [
            (render::Language::English, "Credit note"),
            (render::Language::Dutch, "Creditnota"),
        ] {
            let data = crate::pdf_data("2026-0001-C", &draft, &seller, &customer, &totals, language);
            assert_eq!(data.title, expected);
        }
    }

    #[test]
    fn builds_a_valid_draft_from_the_form() {
        let draft = build_draft(&editor(), &seller()).unwrap();
        assert_eq!(draft.lines.len(), 1);
        assert_eq!(draft.lines[0].net().unwrap().micros(), 100_000_000);
    }

    #[test]
    fn rejects_bad_dates() {
        let mut editor = editor();
        editor.issue_date = "27-10-2026".to_owned();
        let error = build_draft(&editor, &seller()).unwrap_err();
        assert!(error.contains("Issue date"), "{error}");
    }

    #[test]
    fn rejects_a_missing_item_name() {
        let mut editor = editor();
        editor.lines[0].name = "  ".to_owned();
        let error = build_draft(&editor, &seller()).unwrap_err();
        assert!(error.contains("item name"), "{error}");
    }

    #[test]
    fn rejects_bad_numbers() {
        let mut editor = editor();
        editor.lines[0].quantity = "two".to_owned();
        assert!(build_draft(&editor, &seller()).is_err());
    }

    #[test]
    fn category_labels_are_stable() {
        assert_eq!(category_label(VatCategory::Standard), "S standard");
        assert_eq!(category_label(VatCategory::ReverseCharge), "AE reverse charge");
    }

    #[test]
    fn only_editable_fields_offer_focus() {
        for field in [
            "issue_date",
            "delivery_date",
            "seller.vat_id",
            "customer.country",
            "lines[12].quantity",
            "lines[0].vat_category",
        ] {
            assert!(focusable(field), "{field} should be focusable");
        }
        for field in [
            "lines",
            "totals.net",
            "tax_breakdown[0].tax",
            "lines[0].position",
            "seller",
        ] {
            assert!(!focusable(field), "{field} should not be focusable");
        }
    }

    #[test]
    fn only_matching_warnings_are_suppressed() {
        let suppressions = vec![Suppression {
            code: "T1-013".to_owned(),
            field: "lines[0].exemption_reason".to_owned(),
            reason: "Known exempt supply".to_owned(),
        }];
        let warning = Finding::warning(
            "T1-013",
            Some("BT-120"),
            "lines[0].exemption_reason",
            "an exemption reason is expected",
        );
        assert!(suppressed_by(&warning, &suppressions));
        assert!(!suppressed_by(&warning, &[]));

        let blocking = Finding::blocking(
            "T1-013",
            Some("BT-120"),
            "lines[0].exemption_reason",
            "an exemption reason is required",
        );
        assert!(!suppressed_by(&blocking, &suppressions));
    }
}
