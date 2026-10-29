//! The unlocked workspace: onboarding, customers, and the invoice editor.
//!
//! The UI never touches SQL directly; it builds a domain draft, asks the domain
//! for totals and findings, and calls the same store functions as the CLI.

// Panel and column geometry is floating point. Amounts are integers from `crux`
// and never pass through this arithmetic.
#![allow(clippy::float_arithmetic)]

use std::path::{Path, PathBuf};

use crux::{
    Address, CountryCode, Currency, CustomerId, DocType, DocumentId, DraftLine, Finding, InvoiceDraft, LineId,
    LocalDate, Money, Party, PhoneNumber, PrecedingInvoice, Quantity, SavedSearchId, SellerId, Severity, TaxTreatment,
    UnitCode, VatCategory, VatRate, calculate_totals, has_blocking, validate_draft,
};
use store::{
    CustomerRecord, Direction, DocumentRecord, DocumentStatus, IssueRequest, SavedSearch, SearchHit, SearchRequest,
    SellerRecord, StoreError, Suppression, archive::DocumentView, attach_artefact, credit_note_for, first_seller,
    insert_customer, insert_seller, issue_draft, list_customers, list_suppressions, save_draft, save_suppression,
    search,
};
use zeroize::Zeroize;

use crate::Session;
use crate::customers;
use crate::health::Health;
use crate::table::{Cell, Table};
use crate::text::editor::Field;
use crate::text::find::Fact;
use crate::text::{self, Language};
use crate::theme;

const CATEGORIES: [VatCategory; 6] = [
    VatCategory::Standard,
    VatCategory::ZeroRated,
    VatCategory::Exempt,
    VatCategory::ReverseCharge,
    VatCategory::IntraCommunity,
    VatCategory::Export,
];

/// The three places the window can be.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Tab {
    /// Find and read documents, and write a new one.
    #[default]
    Search,
    /// The party list.
    Customers,
    /// Archive health, backups, and credentials.
    Settings,
}

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
    customers_view: CustomersView,
    customer_edit: Option<CustomerEdit>,
    tab: Tab,
    editor: Option<Editor>,
    issued: Option<DocumentRecord>,
    pdf: Option<PdfDocument>,
    pdf_path: String,
    /// Where an export of the open stored document goes; the extension decides
    /// which artefact is written.
    export_path: String,
    /// The preview of the composer's invoice, once one has been asked for.
    preview: Option<Preview>,
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
struct SearchView {
    query: String,
    direction: Option<Direction>,
    status: Option<DocumentStatus>,
    results: Vec<SearchHit>,
    /// Column shares and the chosen sort.
    table: Table,
    /// The request the visible results came from, so a change can re-run it.
    last_request: Option<(String, Option<Direction>, Option<DocumentStatus>)>,
    /// True once a query has run, which is what separates "nothing found" from
    /// "nothing asked yet".
    ran: bool,
    /// Why the last query failed, if it did.
    query_error: Option<String>,
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

impl Default for SearchView {
    fn default() -> Self {
        Self {
            query: String::new(),
            direction: None,
            status: None,
            results: Vec::new(),
            table: Table::new(&[1.2, 0.8, 2.6, 1.0, 1.1, 1.0]),
            last_request: None,
            ran: false,
            query_error: None,
            opened: None,
            open_error: None,
            saved: Vec::new(),
            save_name: String::new(),
            applied: None,
            saved_error: None,
        }
    }
}

/// What the customer tab holds between frames.
struct CustomersView {
    filter: String,
    table: Table,
    /// Where import reads and export writes.
    path: String,
    /// The customer whose detail editor is open, if any.
    opened: Option<CustomerId>,
}

/// A working copy of one customer, so Cancel leaves the archive alone.
#[derive(Clone)]
struct CustomerEdit {
    id: CustomerId,
    form: CustomerForm,
}

impl CustomerEdit {
    fn from_record(customer: &CustomerRecord) -> Self {
        Self {
            id: customer.id,
            form: CustomerForm {
                name: customer.party.legal_name.clone(),
                street: customer.party.address.street.clone().unwrap_or_default(),
                postal: customer.party.address.postal_code.clone().unwrap_or_default(),
                city: customer.party.address.city.clone(),
                country: customer.party.address.country.as_str().to_owned(),
                vat: customer.party.vat_id.clone().unwrap_or_default(),
                kvk: customer.party.kvk_number.clone().unwrap_or_default(),
                peppol: customer.party.peppol_id.clone().unwrap_or_default(),
                contact: customer.party.contact_name.clone().unwrap_or_default(),
                email: customer.party.email.clone().unwrap_or_default(),
                phone: customer.party.phone.clone().unwrap_or_default(),
            },
        }
    }
}

impl SearchView {
    /// The filters currently on screen. A query re-runs when this changes.
    fn request_key(&self) -> (String, Option<Direction>, Option<DocumentStatus>) {
        (self.query.trim().to_owned(), self.direction, self.status)
    }

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
        let outcome = match search(database.connection(), owner, &request) {
            Ok(results) => {
                self.results = results;
                self.ran = true;
                self.query_error = None;
                Ok(())
            }
            Err(error) => {
                self.query_error = Some(error.to_string());
                Err(error)
            }
        };
        // Record the request even on failure: retrying the same impossible
        // query every frame would turn one error into a busy loop.
        self.last_request = Some(self.request_key());
        self.sort_results();
        outcome
    }

    /// Re-applies the chosen sort to the current results.
    fn sort_results(&mut self) {
        if let Some((column, descending)) = self.table.sort {
            sort_hits(&mut self.results, column, descending);
        }
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

/// A rendered preview of the invoice being written.
///
/// It is a snapshot, not a live view: rendering costs a Typst run, so it happens
/// when the user asks and stays until the composer closes.
struct Preview {
    texture: egui::TextureHandle,
    size: egui::Vec2,
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

/// A working copy of one customer form, used both to create and to edit.
#[derive(Clone)]
struct CustomerForm {
    name: String,
    street: String,
    postal: String,
    city: String,
    country: String,
    vat: String,
    kvk: String,
    peppol: String,
    contact: String,
    email: String,
    phone: String,
}

impl CustomerForm {
    /// Builds the party the form describes, normalising the phone number.
    ///
    /// The phone is parsed against the party's own country, so `06 12 34 56 78`
    /// and `+31 6 1234 5678` are one stored value.
    fn to_party(&self) -> Result<Party, String> {
        let country = CountryCode::parse(self.country.trim()).map_err(|error| error.to_string())?;
        let mut party = Party::new(
            self.name.trim().to_owned(),
            Address {
                street: nonempty(&self.street),
                city: self.city.trim().to_owned(),
                postal_code: nonempty(&self.postal),
                country,
            },
        );
        party.vat_id = nonempty(&self.vat).map(|value| value.to_ascii_uppercase());
        party.kvk_number = nonempty(&self.kvk);
        party.peppol_id = nonempty(&self.peppol);
        party.contact_name = nonempty(&self.contact);
        party.email = nonempty(&self.email);
        party.phone = match nonempty(&self.phone) {
            Some(raw) => Some(
                PhoneNumber::parse(&raw, country)
                    .map_err(|error| error.to_string())?
                    .as_str()
                    .to_owned(),
            ),
            None => None,
        };
        Ok(party)
    }

    fn field_mut(&mut self, field: Field) -> &mut String {
        match field {
            Field::Name => &mut self.name,
            Field::Street => &mut self.street,
            Field::PostCode => &mut self.postal,
            Field::City => &mut self.city,
            Field::Country => &mut self.country,
            Field::VatId => &mut self.vat,
            Field::Kvk => &mut self.kvk,
            Field::Peppol => &mut self.peppol,
            Field::Contact => &mut self.contact,
            Field::Email => &mut self.email,
            Field::Phone => &mut self.phone,
            Field::LegalName => &mut self.name,
        }
    }
}

impl Default for CustomerForm {
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
            contact: String::new(),
            email: String::new(),
            phone: String::new(),
        }
    }
}

/// Which editor field a calendar popup is filling.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DateField {
    Issue,
    Due,
    Delivery,
}

impl DateField {
    /// The widget id used for focus and for the popup's text field.
    fn id(self) -> &'static str {
        match self {
            Self::Issue => "issue_date",
            Self::Due => "due_date",
            Self::Delivery => "delivery_date",
        }
    }
}

/// An open calendar. The month is the one on screen, not the selected date.
#[derive(Clone, Copy, Debug)]
struct DatePopup {
    field: DateField,
    month: time::Date,
    pos: egui::Pos2,
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
    date_popup: Option<DatePopup>,
    /// True when this composer reopened a draft that is already stored, which is
    /// only used to say so in the heading.
    saved: bool,
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
            date_popup: self.date_popup,
            saved: self.saved,
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
            date_popup: None,
            saved: false,
        }
    }

    /// An editor filled from a stored draft, so unfinished work reopens where it
    /// was left.
    fn from_draft(draft: &InvoiceDraft, document_language: render::Language) -> Self {
        let mut editor = Self::new(Some(draft.customer_id), document_language);
        editor.saved = true;
        editor.id = draft.id;
        editor.doc_type = draft.doc_type;
        editor.preceding = draft.preceding_invoice.clone();
        editor.issue_date = draft.issue_date.to_iso_string();
        editor.due_date = draft.due_date.map(|date| date.to_iso_string()).unwrap_or_default();
        editor.delivery_date = draft.delivery_date.map(|date| date.to_iso_string()).unwrap_or_default();
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
        editor
    }

    /// An editor pre-filled from a credit-note draft copied from an issued
    /// invoice: the reference is fixed, the lines and customer are editable.
    fn credit_note(draft: &InvoiceDraft, document_language: render::Language) -> Option<Self> {
        draft.preceding_invoice.clone()?;
        let mut editor = Self::from_draft(draft, document_language);
        editor.doc_type = DocType::CreditNote;
        editor.due_date.clear();
        editor.saved = false;
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
        let csv_path = session
            .database_path()
            .parent()
            .unwrap_or(Path::new("."))
            .join("customers.csv")
            .display()
            .to_string();
        let mut workspace = Self {
            seller: first_seller(connection)?,
            customers: list_customers(connection)?,
            seller_form: SellerForm::default(),
            customer_form: CustomerForm::default(),
            show_customer_form: false,
            customers_view: CustomersView {
                filter: String::new(),
                table: Table::new(&[2.6, 1.4, 0.8, 1.6, 1.6]),
                path: csv_path,
                opened: None,
            },
            customer_edit: None,
            tab: Tab::Search,
            editor: None,
            issued: None,
            pdf: None,
            pdf_path: String::new(),
            export_path: String::new(),
            preview: None,
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
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        appearance: &mut egui::ThemePreference,
    ) -> WorkspaceAction {
        let mut lock = false;
        let mut focus = self.focus_request.take();
        let mut requested = None;
        ui.input(|input| {
            if input.modifiers.command {
                if input.key_pressed(egui::Key::F) {
                    requested = Some(Tab::Search);
                    focus = Some("search.query".to_owned());
                } else if input.key_pressed(egui::Key::Num1) {
                    requested = Some(Tab::Search);
                } else if input.key_pressed(egui::Key::Num2) {
                    requested = Some(Tab::Customers);
                } else if input.key_pressed(egui::Key::Num3) {
                    requested = Some(Tab::Settings);
                }
            }
        });
        if let Some(tab) = requested {
            self.select_tab(tab);
        }

        let rail_frame = egui::Frame::new()
            .fill(theme::palette(ui.ctx()).rail)
            .inner_margin(egui::Margin::symmetric(6, 8));
        egui::Panel::left("workspace-rail")
            .exact_size(56.0)
            .resizable(false)
            .frame(rail_frame)
            .show(ui, |ui| {
                lock = self.rail_ui(ui);
            });

        // A margin between the rail and the content, so a table does not start
        // against the icon column and the last column does not touch the window
        // edge. Tables size themselves from `available_width`, so they take the
        // reduced width naturally.
        let content = egui::Frame::new().inner_margin(egui::Margin {
            left: 18,
            right: 18,
            top: 2,
            bottom: 8,
        });
        content.show(ui, |ui| {
            self.header_ui(ui);
            self.message_ui(ui);

            if self.seller.is_none() {
                self.seller_onboarding(ui, session, focus.as_deref());
            } else {
                match self.tab {
                    Tab::Search => self.search_ui(ui, session, focus.as_deref()),
                    Tab::Customers => self.customers_ui(ui, session, focus.as_deref()),
                    Tab::Settings => self.settings_ui(ui, session, appearance),
                }
            }
        });

        self.suppression_dialog(ui, session);

        if lock {
            WorkspaceAction::Lock
        } else {
            WorkspaceAction::None
        }
    }

    /// The icon rail: tabs at the top, lock at the bottom.
    fn rail_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let language = self.language;
        ui.vertical_centered(|ui| {
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new("B")
                    .strong()
                    .size(18.0)
                    .color(theme::palette(ui.ctx()).accent),
            );
        });
        ui.add_space(10.0);

        let tabs = [
            (Tab::Search, theme::Icon::Search, text::tabs::search(language)),
            (Tab::Customers, theme::Icon::Customers, text::tabs::customers(language)),
            (Tab::Settings, theme::Icon::Settings, text::tabs::settings(language)),
        ];
        for (tab, icon, label) in tabs {
            if theme::rail_button(ui, icon, self.tab == tab, label).clicked() {
                self.select_tab(tab);
            }
            ui.add_space(2.0);
        }

        let mut lock = false;
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
            ui.add_space(2.0);
            if theme::rail_button(ui, theme::Icon::Lock, false, text::workspace::lock_action(language)).clicked() {
                lock = true;
            }
        });
        lock
    }

    /// Switches tabs.
    fn select_tab(&mut self, tab: Tab) {
        self.tab = tab;
    }

    /// The tab title, the seller, and nothing that scrolls.
    fn header_ui(&mut self, ui: &mut egui::Ui) {
        let language = self.language;
        let title = match self.tab {
            Tab::Search => text::tabs::search(language),
            Tab::Customers => text::tabs::customers(language),
            Tab::Settings => text::tabs::settings(language),
        };
        ui.horizontal(|ui| {
            ui.heading(title);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(seller) = &self.seller {
                    ui.weak(seller.party.legal_name.as_str());
                }
            });
        });
        ui.add_space(2.0);
    }

    /// The archive-wide banner and the last result of an action.
    fn message_ui(&mut self, ui: &mut egui::Ui) {
        let language = self.language;
        let tokens = *theme::palette(ui.ctx());
        if !store::cipher_enabled() {
            ui.colored_label(tokens.error, text::workspace::unencrypted_warning(language));
        }
        let Some((message, error)) = self
            .message
            .as_ref()
            .map(|message| (message.text.clone(), message.error))
        else {
            return;
        };
        let colour = if error { tokens.error } else { tokens.success };
        ui.horizontal(|ui| {
            ui.colored_label(colour, message.as_str());
            if ui
                .small_button("×")
                .on_hover_text(text::workspace::dismiss_action(language))
                .clicked()
            {
                self.message = None;
            }
        });
        ui.add_space(2.0);
    }

    /// Draws the find screen. A blank query lists everything; changing any
    /// filter re-runs the query without a button.
    fn search_ui(&mut self, ui: &mut egui::Ui, session: &mut Session, focus: Option<&str>) {
        let language = self.language;
        let Some(owner) = self.seller.as_ref().map(|seller| seller.id) else {
            return;
        };
        let tokens = *theme::palette(ui.ctx());
        let composing = self.editor.is_some() || self.issued.is_some();
        let mut new_invoice = false;

        ui.horizontal(|ui| {
            // The trailing controls are laid out from the right edge so the
            // query field can take whatever space is left over.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                new_invoice = ui
                    .add_enabled(
                        !composing,
                        egui::Button::new(text::editor::new_invoice_action(language)),
                    )
                    .clicked();
                if ui.button(text::find::clear_action(language)).clicked() {
                    self.search.query.clear();
                    self.search.direction = None;
                    self.search.status = None;
                }
                status_combo(ui, language, &mut self.search.status);
                direction_combo(ui, language, &mut self.search.direction);
                let width = ui.available_width().max(160.0);
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.search.query)
                        .id(egui::Id::unique("search.query"))
                        .hint_text(text::find::hint(language))
                        .desired_width(width),
                );
                if focus == Some("search.query") {
                    response.request_focus();
                }
            });
        });
        if new_invoice {
            self.start_invoice();
        }

        let mut apply_saved = None;
        let mut save_current = false;
        let mut delete_saved = false;
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
                    for saved in &self.search.saved {
                        let chosen = self.search.applied.as_deref() == Some(saved.name.as_str());
                        if ui.selectable_label(chosen, saved.name.as_str()).clicked() {
                            apply_saved = Some(saved.name.clone());
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
            if let Some(message) = &self.search.saved_error {
                ui.colored_label(tokens.error, message.as_str());
            }
        });

        if let Some(name) = apply_saved {
            self.search.apply_saved(&name, session.database(), owner);
        }
        if save_current {
            self.search.save_current(session.database(), owner);
        }
        if delete_saved && let Some(name) = self.search.applied.clone() {
            self.search.delete_saved(&name, session.database(), owner);
        }

        if self.search.last_request.as_ref() != Some(&self.search.request_key()) {
            let _ = self.search.run(session.database(), owner);
        }
        if let Some(error) = &self.search.query_error {
            ui.colored_label(tokens.error, error.as_str());
        }

        if self.editor.is_some() || self.issued.is_some() {
            let mut close = false;
            let max_height = (ui.available_height() * 0.62).max(240.0);
            egui::Frame::new()
                .fill(tokens.surface_alt)
                .stroke(egui::Stroke::new(1.0, tokens.border))
                .corner_radius(8)
                .inner_margin(egui::Margin::same(12))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.strong(self.composer_heading(language));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            close = ui
                                .small_button("×")
                                .on_hover_text(text::editor::close_composer(language))
                                .clicked();
                        });
                    });
                    egui::ScrollArea::vertical()
                        .id_salt("composer")
                        .max_height(max_height)
                        .auto_shrink([false, false])
                        .show(ui, |ui| self.composer_ui(ui, session, focus));
                });
            if close {
                self.reset_document();
            }
            ui.add_space(8.0);
        }

        let width = (ui.available_width() - ui.spacing().scroll.bar_width).max(320.0);
        if self
            .search
            .table
            .header(ui, "find", &text::find::columns(language), width)
        {
            self.search.sort_results();
        }

        let mut toggle = None;
        let mut close_panel = false;
        let mut save_artefact = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(width);
            if self.search.results.is_empty() {
                ui.add_space(10.0);
                ui.weak(if self.search.query.trim().is_empty() {
                    text::find::empty_archive(language)
                } else {
                    text::find::no_matches(language)
                });
            }

            for (index, hit) in self.search.results.iter().enumerate() {
                let selected = self.search.opened.as_ref().is_some_and(|view| view.id == hit.id);
                // A draft has no number and no side yet; the status column says
                // what it is, so the other two stay blank rather than repeating
                // it in a column that means something else.
                let draft = hit.status == DocumentStatus::Draft;
                let number = if draft {
                    text::find::no_value(language)
                } else {
                    hit.number.as_str()
                };
                let side = if draft {
                    text::find::no_value(language)
                } else {
                    text::find::direction(language, hit.direction)
                };
                let cells = [
                    Cell::left(number).strong(),
                    Cell::left(side).muted(),
                    Cell::left(hit.party.as_str()),
                    Cell::left(hit.issue_date.to_iso_string()),
                    Cell::right(format!("{} {}", money_text(hit.gross), hit.currency.as_str())),
                    Cell::left(text::find::status(language, hit.status)),
                ];
                if self
                    .search
                    .table
                    .row(ui, width, &cells, selected, index % 2 == 1)
                    .clicked()
                {
                    toggle = Some(hit.id);
                }
                if selected {
                    ui.add_space(2.0);
                    let action = egui::Frame::new()
                        .fill(tokens.surface_alt)
                        .stroke(egui::Stroke::new(1.0, tokens.border))
                        .corner_radius(6)
                        .inner_margin(egui::Margin::same(12))
                        .show(ui, |ui| match &self.search.opened {
                            Some(view) => document_panel(ui, language, view, &mut self.export_path),
                            None => DocumentAction::None,
                        })
                        .inner;
                    match action {
                        DocumentAction::None => {}
                        DocumentAction::Close => close_panel = true,
                        DocumentAction::Save(kind) => save_artefact = Some(kind),
                    }
                    ui.add_space(4.0);
                }
            }

            if let Some(error) = &self.search.open_error {
                ui.colored_label(tokens.error, error.as_str());
            }
            ui.add_space(8.0);
            ui.weak(text::find::sort_hint(language));
            ui.weak(text::find::footer(
                language,
                self.search.results.len(),
                store::SEARCH_LIMIT,
            ));
        });

        if let Some(id) = toggle {
            let draft = self
                .search
                .results
                .iter()
                .any(|hit| hit.id == id && hit.status == DocumentStatus::Draft);
            if draft {
                self.open_draft(session, id);
            } else if self.search.opened.as_ref().is_some_and(|view| view.id == id) {
                self.search.opened = None;
            } else {
                self.search.open(session.database(), id);
                if let Some(number) = self.search.opened.as_ref().map(|view| view.number.clone()) {
                    self.export_path = default_pdf_path(session, &number);
                }
            }
        }
        if let Some(kind) = save_artefact {
            self.save_artefact(session, kind);
        }
        if close_panel {
            self.search.opened = None;
        }
    }

    /// The seller's locale is the sensible default for what gets printed, and
    /// only a default: the editor can change it per document.
    fn default_document_language(&self) -> render::Language {
        self.seller.as_ref().map_or_else(render::Language::default, |seller| {
            render::Language::from_locale(&seller.default_locale)
        })
    }

    /// Draws the customer list and its import/export controls.
    fn customers_ui(&mut self, ui: &mut egui::Ui, session: &Session, focus: Option<&str>) {
        let language = self.language;
        let mut import = false;
        let mut export = false;
        let mut toggle_form = false;
        let mut create = false;

        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.customers_view.filter)
                    .hint_text(text::customers::filter_hint(language))
                    .desired_width(260.0),
            );
            if ui.button(text::customers::new_customer_action(language)).clicked() {
                toggle_form = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(text::customers::export_action(language)).clicked() {
                    export = true;
                }
                if ui.button(text::customers::import_action(language)).clicked() {
                    import = true;
                }
                ui.add(egui::TextEdit::singleline(&mut self.customers_view.path).desired_width(300.0));
                ui.label(text::customers::path_label(language));
            });
        });
        ui.weak(text::customers::csv_note(language));
        ui.add_space(4.0);

        if toggle_form {
            self.show_customer_form = !self.show_customer_form;
            if self.show_customer_form {
                self.customers_view.opened = None;
                self.customer_edit = None;
            }
        }
        if self.show_customer_form {
            self.customer_form_ui(ui, focus, &mut create);
        }
        if create {
            self.create_customer(session);
        }

        let width = (ui.available_width() - ui.spacing().scroll.bar_width).max(320.0);
        self.customers_view
            .table
            .header(ui, "customers", &text::customers::columns(language), width);

        let needle = self.customers_view.filter.trim().to_lowercase();
        let mut visible: Vec<&CustomerRecord> = self
            .customers
            .iter()
            .filter(|customer| matches_filter(&customer.party, &needle))
            .collect();
        if let Some((column, descending)) = self.customers_view.table.sort {
            sort_customers(&mut visible, column, descending);
        }

        let mut toggle = None;
        let mut save_edit = false;
        let mut cancel_edit = false;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(width);
            if visible.is_empty() {
                ui.add_space(10.0);
                ui.weak(if self.customers.is_empty() {
                    text::customers::empty(language)
                } else {
                    text::customers::no_matches(language)
                });
            }
            for (index, customer) in visible.iter().enumerate() {
                let party = &customer.party;
                let selected = self.customers_view.opened == Some(customer.id);
                let cells = [
                    Cell::left(party.legal_name.as_str()),
                    Cell::left(party.address.city.as_str()),
                    Cell::left(party.address.country.as_str()),
                    Cell::left(party.vat_id.as_deref().unwrap_or("—")).muted(),
                    Cell::left(party.peppol_id.as_deref().unwrap_or("—")).muted(),
                ];
                if self
                    .customers_view
                    .table
                    .row(ui, width, &cells, selected, index % 2 == 1)
                    .clicked()
                {
                    toggle = Some(customer.id);
                }
                if selected && let Some(edit) = self.customer_edit.as_mut() {
                    let (save, cancel) = customer_edit_ui(ui, language, edit, focus);
                    save_edit |= save;
                    cancel_edit |= cancel;
                }
            }
            ui.add_space(8.0);
            ui.weak(format!("{} customer(s)", visible.len()));
        });

        if let Some(id) = toggle {
            if self.customers_view.opened == Some(id) {
                self.customers_view.opened = None;
                self.customer_edit = None;
            } else if let Some(record) = self.customers.iter().find(|customer| customer.id == id).cloned() {
                self.customer_edit = Some(CustomerEdit::from_record(&record));
                self.customers_view.opened = Some(id);
                self.show_customer_form = false;
            }
        }
        if cancel_edit {
            self.customers_view.opened = None;
            self.customer_edit = None;
        }
        if save_edit {
            self.save_customer_edit(session);
        }

        if import || export {
            self.transfer_customers(session, import);
        }
    }

    /// The new-customer form, shown inside the customer tab.
    fn customer_form_ui(&mut self, ui: &mut egui::Ui, focus: Option<&str>, create: &mut bool) {
        let language = self.language;
        let tokens = *theme::palette(ui.ctx());
        egui::Frame::new()
            .fill(tokens.surface_alt)
            .stroke(egui::Stroke::new(1.0, tokens.border))
            .corner_radius(8)
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.strong(text::editor::new_customer_heading(language));
                egui::Grid::new("customer-form").num_columns(2).show(ui, |ui| {
                    for field in CUSTOMER_FIELDS {
                        ui.label(text::editor::field(language, field));
                        let id = format!("customer.{}", customer_key(field));
                        text_field(ui, self.customer_form.field_mut(field), &id, 264.0, focus);
                        ui.end_row();
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button(text::editor::save_customer_action(language)).clicked() {
                        *create = true;
                    }
                    if ui.button(text::customers::cancel_action(language)).clicked() {
                        self.show_customer_form = false;
                    }
                });
            });
        ui.add_space(10.0);
    }

    /// Saves the open customer detail editor.
    fn save_customer_edit(&mut self, session: &Session) {
        let language = self.language;
        let Some(edit) = self.customer_edit.clone() else {
            return;
        };
        let party = match edit.form.to_party() {
            Ok(party) => party,
            Err(error) => {
                self.message = Some(Message::error(error));
                return;
            }
        };

        let record = CustomerRecord { id: edit.id, party };
        match store::update_customer(session.database().connection(), &record, &store::now_rfc3339()) {
            Ok(()) => {
                if let Ok(list) = list_customers(session.database().connection()) {
                    self.customers = list;
                }
                self.customer_edit = None;
                self.customers_view.opened = None;
                self.message = Some(Message::ok(text::customers::updated(
                    language,
                    &record.party.legal_name,
                )));
            }
            Err(error) => self.message = Some(Message::error(error.to_string())),
        }
    }

    /// Reads or writes the customer CSV named in the tab.
    fn transfer_customers(&mut self, session: &Session, import: bool) {
        let language = self.language;
        let path = self.customers_view.path.trim().to_owned();
        if path.is_empty() {
            self.message = Some(Message::error(text::customers::failed(language, "the path is empty")));
            return;
        }

        if import {
            let parsed = std::fs::read_to_string(&path)
                .map_err(|error| error.to_string())
                .and_then(|text| customers::parse(&text));
            match parsed {
                Ok(rows) => match customers::import(session.database(), &rows, &store::now_rfc3339()) {
                    Ok(report) => {
                        if let Ok(list) = list_customers(session.database().connection()) {
                            self.customers = list;
                        }
                        let summary =
                            text::customers::imported(language, report.imported, report.skipped, report.errors.len());
                        self.message = Some(if report.errors.is_empty() {
                            Message::ok(summary)
                        } else {
                            let detail = report.errors.iter().take(3).cloned().collect::<Vec<_>>().join("; ");
                            Message::error(format!("{summary} {detail}"))
                        });
                    }
                    Err(error) => self.message = Some(Message::error(error.to_string())),
                },
                Err(error) => self.message = Some(Message::error(text::customers::failed(language, &error))),
            }
        } else {
            match std::fs::write(&path, customers::export(&self.customers)) {
                Ok(()) => {
                    self.message = Some(Message::ok(text::customers::exported(
                        language,
                        self.customers.len(),
                        &path,
                    )));
                }
                Err(error) => {
                    self.message = Some(Message::error(text::customers::failed(language, &error.to_string())))
                }
            }
        }
    }

    /// Draws archive settings, health, backup, and credentials.
    fn settings_ui(&mut self, ui: &mut egui::Ui, session: &Session, appearance: &mut egui::ThemePreference) {
        let language = self.language;
        let mut check = false;
        let mut backup = false;

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(ui.available_width());

            theme::card(ui, text::settings::appearance(language), |ui| {
                let mut changed = false;
                ui.horizontal(|ui| {
                    changed |= ui
                        .selectable_value(
                            appearance,
                            egui::ThemePreference::System,
                            text::settings::theme_system(language),
                        )
                        .changed();
                    changed |= ui
                        .selectable_value(
                            appearance,
                            egui::ThemePreference::Light,
                            text::settings::theme_light(language),
                        )
                        .changed();
                    changed |= ui
                        .selectable_value(
                            appearance,
                            egui::ThemePreference::Dark,
                            text::settings::theme_dark(language),
                        )
                        .changed();
                });
                if changed {
                    theme::set(ui.ctx(), *appearance);
                }
                ui.weak(text::settings::theme_note(language));
            });
            ui.add_space(8.0);

            theme::card(ui, text::settings::archive(language), |ui| {
                ui.horizontal(|ui| {
                    ui.weak(text::settings::database(language));
                    theme::value_mono(ui, session.database_path().display().to_string());
                });
                ui.horizontal(|ui| {
                    ui.weak(text::settings::encryption(language));
                    theme::value(ui, store::encryption_label());
                });
            });
            ui.add_space(8.0);

            check = theme::card(ui, text::settings::health(language), |ui| self.health_details_ui(ui));
            ui.add_space(8.0);

            theme::card(ui, text::settings::backup(language), |ui| {
                ui.weak(text::settings::backup_note(language));
                theme::value_mono(
                    ui,
                    text::settings::backup_directory(language, &backup_directory(session).display().to_string()),
                );
                if ui.button(text::settings::backup_now(language)).clicked() {
                    backup = true;
                }
            });
            ui.add_space(8.0);

            theme::card(ui, text::settings::credentials(language), |ui| {
                self.security_details_ui(ui, session);
            });
        });

        if check {
            self.run_health_check(session);
        }
        if backup {
            self.run_backup(session);
        }
    }

    /// Writes and verifies a backup beside the archive.
    fn run_backup(&mut self, session: &Session) {
        let language = self.language;
        let directory = backup_directory(session);
        match session.backup(&directory, &store::now_rfc3339(), store::backup::RETENTION) {
            Ok(report) => {
                self.message = Some(Message::ok(text::settings::backup_done(
                    language,
                    &report.path.display().to_string(),
                    report.bytes / 1024,
                )));
                self.health = Health::snapshot(session.database(), session.database_path()).ok();
            }
            Err(error) => {
                self.message = Some(Message::error(text::settings::backup_failed(
                    language,
                    &error.to_string(),
                )))
            }
        }
    }

    /// The security panel's contents, drawn directly by the headless test.
    fn security_details_ui(&mut self, ui: &mut egui::Ui, session: &Session) {
        let language = self.language;
        let tokens = *theme::palette(ui.ctx());
        let mut change = false;
        let mut add_key = false;
        let mut done = false;
        let mut remember = false;
        let mut forget = false;

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
            ui.colored_label(tokens.warning, text::shell::keep_key_note(language));
            theme::value_mono(ui, key.as_str());
            if ui.button(text::security::done_action(language)).clicked() {
                done = true;
            }
        }

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button(text::security::remember_action(language)).clicked() {
                remember = true;
            }
            if ui.button(text::security::forget_action(language)).clicked() {
                forget = true;
            }
        });

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

        if remember {
            if self.security.current.trim().is_empty() {
                self.message = Some(Message::error(text::security::enter_current(language)));
            } else {
                match session.remember_device(&self.security.current, &store::device::Keychain) {
                    Ok(()) => {
                        self.security.forget_secrets();
                        self.message = Some(Message::ok(text::security::device_remembered(language)));
                    }
                    Err(error) => self.message = Some(Message::error(error.to_string())),
                }
            }
        }

        if forget {
            match session.forget_device(&store::device::Keychain) {
                Ok(true) => {
                    self.message = Some(Message::ok(text::security::device_forgotten(language)));
                }
                Ok(false) => {
                    self.message = Some(Message::ok(text::security::device_no_key(language)));
                }
                Err(error) => self.message = Some(Message::error(error.to_string())),
            }
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
        let tokens = *theme::palette(ui.ctx());
        let Some(health) = &self.health else {
            ui.label(text::health::unreadable(language));
            return false;
        };

        ui.horizontal(|ui| {
            theme::value(ui, text::health::schema(language, health.schema_version));
            ui.label("·");
            theme::value(ui, health.encryption);
            ui.label("·");
            theme::value(ui, text::health::size(language, health.database_bytes / 1024));
        });

        match &health.chain {
            Some(head) => {
                let short = health.short_head().unwrap_or_default();
                ui.horizontal(|ui| {
                    theme::value(ui, text::health::chain(language, head.seq));
                    theme::value_mono(ui, short).on_hover_text(head.hash.as_str());
                });
            }
            None => {
                ui.label(text::health::empty_chain(language));
            }
        }

        match (&health.report, &health.error) {
            (_, Some(error)) => {
                ui.colored_label(tokens.error, text::health::integrity_failed(language, error));
            }
            (Some(report), None) if report.orphaned_blobs > 0 => {
                ui.colored_label(
                    tokens.warning,
                    text::health::integrity_with_orphans(language, report.blobs, report.orphaned_blobs),
                );
            }
            (Some(report), None) => {
                ui.colored_label(tokens.success, text::health::integrity_ok(language, report.blobs));
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
                theme::value(
                    ui,
                    text::health::last_backup(language, &backup.name, backup.bytes / 1024),
                );
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

    /// The heading over the composer card.
    fn composer_heading(&self, language: Language) -> &'static str {
        if let Some(issued) = &self.issued {
            text::editor::issued_heading(issued.number.contains("-C"))
        } else if self.editor.as_ref().is_some_and(|editor| editor.saved) {
            text::editor::draft_heading(language)
        } else {
            text::editor::new_invoice_action(language)
        }
    }

    /// The invoice being written, or the one just issued.
    ///
    /// It lives in the Search tab rather than a tab of its own: writing an
    /// invoice starts from the list of what is already there, and the list stays
    /// on screen underneath while the composer is open.
    fn composer_ui(&mut self, ui: &mut egui::Ui, session: &mut Session, focus: Option<&str>) {
        let language = self.language;
        let mut start_credit = false;
        if let Some(issued) = &self.issued {
            theme::value(ui, text::editor::number_line(language, &issued.number));
            theme::value(
                ui,
                text::editor::issue_date_line(language, &issued.issue_date.to_string()),
            );
            theme::value(
                ui,
                text::editor::payable_line(language, &money_text(issued.totals.gross)),
            );
            theme::value_mono(ui, text::editor::ubl_hash_line(language, &issued.ubl_sha256));
        }

        if self.issued.is_some() {
            let is_credit_note = self.issued.as_ref().is_some_and(|issued| issued.number.contains("-C"));
            let mut save_pdf = false;
            if let Some(pdf) = &self.pdf {
                theme::value_mono(ui, text::editor::pdf_hash_line(language, &pdf.hash));
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
                self.start_invoice();
            } else if start_credit {
                self.start_credit_note(session);
            }
            return;
        }

        if self.customers.is_empty() {
            ui.horizontal(|ui| {
                ui.label(text::editor::no_customers(language));
                if ui.button(text::editor::go_to_customers(language)).clicked() {
                    self.tab = Tab::Customers;
                }
            });
            return;
        }

        if let Some(seller) = &self.seller {
            ui.label(text::editor::seller_line(language, &seller.party.legal_name));
        }
        self.editor_ui(ui, session, focus);
        self.preview_ui(ui);
    }

    /// Shows the rendered preview, if one has been asked for.
    fn preview_ui(&mut self, ui: &mut egui::Ui) {
        let language = self.language;
        let Some(preview) = &self.preview else {
            return;
        };
        ui.add_space(10.0);
        let close = ui.horizontal(|ui| {
            ui.strong(text::editor::preview_heading(language));
            ui.small_button("×")
                .on_hover_text(text::editor::close_preview(language))
                .clicked()
        });
        let width = ui.available_width().min(760.0);
        let scale = (width / preview.size.x).min(1.0);
        ui.add(egui::Image::new(egui::load::SizedTexture::new(
            preview.texture.id(),
            preview.size * scale,
        )));
        if close.inner {
            self.preview = None;
        }
    }

    /// Renders what the composer currently holds and keeps it as a texture.
    ///
    /// This is the same template that issuing compiles, so the preview cannot
    /// disagree with the PDF that gets filed.
    fn render_preview(&mut self, ctx: &egui::Context) {
        let language = self.language;
        let Some(seller) = self.seller.clone() else {
            return;
        };
        let Some(editor) = self.editor.clone() else {
            return;
        };
        let Some(customer) = self
            .customers
            .iter()
            .find(|customer| Some(customer.id) == editor.customer)
            .cloned()
        else {
            self.message = Some(Message::error(text::editor::select_customer_required(language)));
            return;
        };
        let draft = match build_draft(&editor, &seller) {
            Ok(draft) => draft,
            Err(error) => {
                self.message = Some(Message::error(error));
                return;
            }
        };
        let totals = match calculate_totals(&draft.lines) {
            Ok(totals) => totals,
            Err(error) => {
                self.message = Some(Message::error(error.to_string()));
                return;
            }
        };

        let data = crate::pdf_data(
            text::editor::preview_number(language),
            &draft,
            &seller,
            &customer,
            &totals,
            editor.document_language,
        );
        let bytes = match render::TypstCliRenderer::from_environment().render_png(&data, 110.0) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.message = Some(Message::error(text::editor::preview_failed(
                    language,
                    &error.to_string(),
                )));
                return;
            }
        };
        match image::load_from_memory(&bytes) {
            Ok(decoded) => {
                let rgba = decoded.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let pixels = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                let texture = ctx.load_texture("invoice-preview", pixels, egui::TextureOptions::LINEAR);
                self.preview = Some(Preview {
                    texture,
                    size: egui::vec2(size[0] as f32, size[1] as f32),
                });
            }
            Err(error) => self.message = Some(Message::error(error.to_string())),
        }
    }

    /// Opens an empty invoice in the composer.
    fn start_invoice(&mut self) {
        self.reset_document();
        let customer = self.customers.first().map(|customer| customer.id);
        self.editor = Some(Editor::new(customer, self.default_document_language()));
        self.search.opened = None;
        self.search.open_error = None;
    }

    /// Reopens a stored draft in the composer, so unfinished work is editable
    /// from the list rather than being a dead row.
    fn open_draft(&mut self, session: &Session, id: DocumentId) {
        let language = self.default_document_language();
        match store::load_draft(session.database().connection(), id) {
            Ok(Some(draft)) => {
                self.reset_document();
                self.editor = Some(Editor::from_draft(&draft, language));
                self.search.opened = None;
                self.search.open_error = None;
                if let Ok(suppressions) = list_suppressions(session.database().connection(), id) {
                    self.suppressions = suppressions;
                }
            }
            Ok(None) => {
                self.message = Some(Message::error(text::editor::draft_missing(self.language)));
            }
            Err(error) => self.message = Some(Message::error(error.to_string())),
        }
    }

    fn editor_ui(&mut self, ui: &mut egui::Ui, session: &mut Session, focus: Option<&str>) {
        let language = self.language;
        let tokens = *theme::palette(ui.ctx());
        let Some(seller) = self.seller.clone() else {
            return;
        };
        let customers = self.customers.clone();
        let save;
        let issue;
        let preview;
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
                ui.colored_label(tokens.success, text::editor::credit_note_banner(language, number));
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
                date_field(
                    ui,
                    language,
                    &mut editor.issue_date,
                    &mut editor.date_popup,
                    DateField::Issue,
                    false,
                    focus,
                );
                ui.label(text::editor::due_date_label(language));
                date_field(
                    ui,
                    language,
                    &mut editor.due_date,
                    &mut editor.date_popup,
                    DateField::Due,
                    true,
                    focus,
                );
                ui.end_row();
                ui.label(text::editor::buyer_reference_label(language));
                text_field(ui, &mut editor.buyer_reference, "buyer_reference", 130.0, focus);
                ui.label(text::editor::order_reference_label(language));
                text_field(ui, &mut editor.order_reference, "order_reference", 130.0, focus);
                ui.end_row();
                ui.label(text::editor::delivery_date_label(language));
                date_field(
                    ui,
                    language,
                    &mut editor.delivery_date,
                    &mut editor.date_popup,
                    DateField::Delivery,
                    true,
                    focus,
                );
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

            let mut popup_action = CalendarOutcome::Keep;
            if let Some(popup) = editor.date_popup.as_mut() {
                popup_action = date_picker(ui.ctx(), language, popup);
            }
            match popup_action {
                CalendarOutcome::Keep => {}
                CalendarOutcome::Close => editor.date_popup = None,
                CalendarOutcome::Pick(day) => {
                    match editor.date_popup.map(|popup| popup.field) {
                        Some(DateField::Issue) => editor.issue_date = day,
                        Some(DateField::Due) => editor.due_date = day,
                        Some(DateField::Delivery) => editor.delivery_date = day,
                        None => {}
                    }
                    editor.date_popup = None;
                }
            }

            ui.add_space(8.0);
            ui.label(text::editor::note_label(language));
            let note = ui.add(
                egui::TextEdit::multiline(&mut editor.note)
                    .id(egui::Id::unique("note"))
                    .desired_rows(3)
                    .desired_width(ui.available_width()),
            );
            if focus == Some("note") {
                note.request_focus();
            }

            ui.add_space(10.0);
            let name_width = (ui.available_width() - 380.0).max(220.0);
            egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
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
                            text_field(ui, &mut line.name, &format!("lines[{index}].name"), name_width, focus);
                            text_field(ui, &mut line.quantity, &format!("lines[{index}].quantity"), 64.0, focus);
                            text_field(
                                ui,
                                &mut line.unit_price,
                                &format!("lines[{index}].unit_price"),
                                96.0,
                                focus,
                            );
                            text_field(ui, &mut line.rate, &format!("lines[{index}].rate"), 56.0, focus);
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
            let mut clicked_save = false;
            let mut clicked_issue = false;
            let mut clicked_preview = false;
            ui.horizontal(|ui| {
                clicked_save = ui.button(text::editor::save_draft_action(language)).clicked();
                clicked_issue = ui.button(text::editor::issue_action(language)).clicked();
                clicked_preview = ui.button(text::editor::preview_action(language)).clicked();
            });
            save = clicked_save;
            issue = clicked_issue;
            preview = clicked_preview;
        }

        if preview {
            self.render_preview(ui.ctx());
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
                            ui.colored_label(tokens.error, error.to_string());
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
                            ui.colored_label(tokens.error, text::editor::select_customer_to_validate(language));
                        }
                    }
                }
                Err(error) => {
                    ui.colored_label(tokens.error, error);
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
        self.preview = None;
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
        let party = match self.customer_form.to_party() {
            Ok(party) => party,
            Err(error) => {
                self.message = Some(Message::error(error));
                return;
            }
        };

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
                self.refresh_search(session);
            }
            Err(error) => self.message = Some(Message::error(error.to_string())),
        }
    }

    /// Re-runs the visible query after the archive changed underneath it, so a
    /// saved or issued document appears in the list it was written from.
    fn refresh_search(&mut self, session: &Session) {
        let Some(owner) = self.seller.as_ref().map(|seller| seller.id) else {
            return;
        };
        if self.search.last_request.is_some() {
            let _ = self.search.run(session.database(), owner);
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
                self.refresh_search(session);
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

    /// Writes a stored document's artefact where the export field points.
    ///
    /// The bytes are the ones filed with the document, hash-checked by the
    /// store, so this is a copy and not a re-render.
    fn save_artefact(&mut self, session: &Session, kind: &str) {
        let Some(view) = &self.search.opened else {
            return;
        };
        let id = view.id;
        if self.export_path.trim().is_empty() {
            self.message = Some(Message::error(text::find::choose_export_path(self.language)));
            return;
        }
        match store::artefact_bytes(session.database().connection(), id, kind) {
            Ok(bytes) => {
                let path = sibling_path(&self.export_path, artefact_extension(kind));
                match std::fs::write(&path, &bytes) {
                    Ok(()) => {
                        self.message = Some(Message::ok(text::find::export_saved(
                            self.language,
                            &path.display().to_string(),
                        )))
                    }
                    Err(error) => {
                        self.message = Some(Message::error(text::find::export_failed(
                            self.language,
                            &error.to_string(),
                        )))
                    }
                }
            }
            Err(error) => self.message = Some(Message::error(error.to_string())),
        }
    }
}

fn default_pdf_path(session: &Session, number: &str) -> String {
    let directory = session.database_path().parent().unwrap_or(std::path::Path::new("."));
    directory.join(format!("{number}.pdf")).display().to_string()
}

/// The file extension an artefact kind is written with.
fn artefact_extension(kind: &str) -> &'static str {
    match kind {
        "ubl" => "xml",
        _ => "pdf",
    }
}

/// Replaces the extension of the export path, so one stem names both formats.
fn sibling_path(base: &str, extension: &str) -> PathBuf {
    let path = Path::new(base.trim());
    if path.as_os_str().is_empty() {
        return PathBuf::from(format!("export.{extension}"));
    }
    path.with_extension(extension)
}

/// Where the window keeps its backups.
fn backup_directory(session: &Session) -> PathBuf {
    session
        .database_path()
        .parent()
        .unwrap_or(Path::new("."))
        .join("backups")
}

/// The fields the customer forms edit, in the order they are shown.
const CUSTOMER_FIELDS: [Field; 11] = [
    Field::Name,
    Field::Street,
    Field::PostCode,
    Field::City,
    Field::Country,
    Field::VatId,
    Field::Kvk,
    Field::Peppol,
    Field::Contact,
    Field::Email,
    Field::Phone,
];

fn customer_key(field: Field) -> &'static str {
    match field {
        Field::LegalName | Field::Name => "name",
        Field::Street => "street",
        Field::PostCode => "postal_code",
        Field::City => "city",
        Field::Country => "country",
        Field::VatId => "vat_id",
        Field::Kvk => "kvk_number",
        Field::Peppol => "peppol",
        Field::Contact => "contact",
        Field::Email => "email",
        Field::Phone => "phone",
    }
}

/// Draws the inline editor for one customer. Returns whether Save or Cancel was
/// clicked; the caller applies it after the table has finished borrowing.
fn customer_edit_ui(
    ui: &mut egui::Ui,
    language: Language,
    edit: &mut CustomerEdit,
    focus: Option<&str>,
) -> (bool, bool) {
    let tokens = *theme::palette(ui.ctx());
    let mut save = false;
    let mut cancel = false;
    ui.add_space(2.0);
    egui::Frame::new()
        .fill(tokens.surface_alt)
        .stroke(egui::Stroke::new(1.0, tokens.border))
        .corner_radius(6)
        .inner_margin(egui::Margin::same(14))
        .show(ui, |ui| {
            ui.strong(text::customers::edit_heading(language));
            egui::Grid::new("customer-edit").num_columns(2).show(ui, |ui| {
                for field in CUSTOMER_FIELDS {
                    ui.label(text::editor::field(language, field));
                    let id = format!("edit.customer.{}", customer_key(field));
                    text_field(ui, edit.form.field_mut(field), &id, 300.0, focus);
                    ui.end_row();
                }
            });
            ui.horizontal(|ui| {
                if ui.button(text::editor::save_customer_action(language)).clicked() {
                    save = true;
                }
                if ui.button(text::customers::cancel_action(language)).clicked() {
                    cancel = true;
                }
            });
        });
    ui.add_space(4.0);
    (save, cancel)
}

/// Case-insensitive match over the fields the customer table shows.
fn matches_filter(party: &Party, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    [
        Some(party.legal_name.as_str()),
        Some(party.address.city.as_str()),
        Some(party.address.country.as_str()),
        party.vat_id.as_deref(),
        party.peppol_id.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|value| value.to_lowercase().contains(needle))
}

/// Sorts find results by the clicked column.
fn sort_hits(hits: &mut [SearchHit], column: usize, descending: bool) {
    hits.sort_by(|left, right| {
        let ordering = match column {
            0 => left.number.cmp(&right.number),
            1 => left.direction.code().cmp(right.direction.code()),
            2 => left.party.cmp(&right.party),
            3 => left.issue_date.cmp(&right.issue_date),
            4 => left.gross.cmp(&right.gross),
            5 => left.status.cmp(&right.status),
            _ => std::cmp::Ordering::Equal,
        };
        if descending { ordering.reverse() } else { ordering }
    });
}

/// Sorts the customer table by the clicked column.
fn sort_customers(customers: &mut [&CustomerRecord], column: usize, descending: bool) {
    customers.sort_by(|left, right| {
        let ordering = match column {
            0 => left.party.legal_name.cmp(&right.party.legal_name),
            1 => left.party.address.city.cmp(&right.party.address.city),
            2 => left.party.address.country.cmp(&right.party.address.country),
            3 => left
                .party
                .vat_id
                .as_deref()
                .unwrap_or("")
                .cmp(right.party.vat_id.as_deref().unwrap_or("")),
            4 => left
                .party
                .peppol_id
                .as_deref()
                .unwrap_or("")
                .cmp(right.party.peppol_id.as_deref().unwrap_or("")),
            _ => std::cmp::Ordering::Equal,
        };
        if descending { ordering.reverse() } else { ordering }
    });
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
    // The composer may have reopened a stored draft; keeping its id is what
    // makes Save an update rather than a second copy.
    draft.id = editor.id;
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

/// What the calendar popup asks the caller to do.
enum CalendarOutcome {
    /// Still open.
    Keep,
    /// Closed without a choice.
    Close,
    /// A date was picked, as `YYYY-MM-DD`.
    Pick(String),
}

/// A text field with a calendar button. Typing stays possible, so a date can be
/// pasted or corrected without opening anything.
fn date_field(
    ui: &mut egui::Ui,
    language: Language,
    value: &mut String,
    popup: &mut Option<DatePopup>,
    field: DateField,
    optional: bool,
    focus: Option<&str>,
) {
    ui.horizontal(|ui| {
        let response = text_field(ui, value, field.id(), 104.0, focus);
        let open = popup.as_ref().is_some_and(|open| open.field == field);
        let glyph = if open { "▴" } else { "▾" };
        if ui
            .small_button(glyph)
            .on_hover_text(text::editor::choose_date(language))
            .clicked()
        {
            if open {
                *popup = None;
            } else {
                let day = LocalDate::parse_iso(value.trim())
                    .map(|date| date.as_date())
                    .unwrap_or_else(|_| LocalDate::today_utc().as_date());
                *popup = Some(DatePopup {
                    field,
                    month: first_of_month(day),
                    pos: response.rect.left_bottom(),
                });
            }
        }
        if optional
            && !value.trim().is_empty()
            && ui
                .small_button("×")
                .on_hover_text(text::editor::clear_date(language))
                .clicked()
        {
            value.clear();
        }
    });
}

/// Draws the floating month grid and reports what the user did.
fn date_picker(ctx: &egui::Context, language: Language, popup: &mut DatePopup) -> CalendarOutcome {
    let tokens = *theme::palette(ctx);
    let mut outcome = CalendarOutcome::Keep;
    egui::Window::new("date-picker")
        .id(egui::Id::unique("date-picker"))
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .fixed_pos(popup.pos)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.small_button("‹").clicked() {
                    popup.month = shift_month(popup.month, -1);
                }
                ui.add_sized(
                    [124.0, 20.0],
                    egui::Label::new(
                        egui::RichText::new(format!(
                            "{} {}",
                            text::editor::month_name(language, popup.month.month()),
                            popup.month.year()
                        ))
                        .strong(),
                    )
                    .halign(egui::Align::Center),
                );
                if ui.small_button("›").clicked() {
                    popup.month = shift_month(popup.month, 1);
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                for day in text::editor::weekdays(language) {
                    ui.add_sized(
                        [26.0, 16.0],
                        egui::Label::new(egui::RichText::new(day).small().color(tokens.muted))
                            .halign(egui::Align::Center),
                    );
                }
            });
            for week in month_grid(popup.month).chunks(7) {
                ui.horizontal(|ui| {
                    for date in week {
                        let in_month = date.month() == popup.month.month();
                        let colour = if in_month { tokens.text } else { tokens.muted };
                        let label = egui::RichText::new(date.day().to_string()).color(colour);
                        if ui
                            .add_sized([26.0, 24.0], egui::Button::new(label).frame(in_month))
                            .clicked()
                        {
                            outcome = CalendarOutcome::Pick(date.to_string());
                        }
                    }
                });
            }
            ui.add_space(4.0);
            if ui.button(text::editor::cancel_action(language)).clicked() {
                outcome = CalendarOutcome::Close;
            }
        });
    if matches!(outcome, CalendarOutcome::Keep) && ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        outcome = CalendarOutcome::Close;
    }
    outcome
}

/// The first day of the month containing `date`.
fn first_of_month(date: time::Date) -> time::Date {
    time::Date::from_calendar_date(date.year(), date.month(), 1).unwrap_or(date)
}

/// Moves a month back or forward, landing on its first day.
fn shift_month(date: time::Date, delta: i32) -> time::Date {
    let mut year = date.year();
    let mut month = i32::from(u8::from(date.month()));
    month += delta;
    while month < 1 {
        month += 12;
        year -= 1;
    }
    while month > 12 {
        month -= 12;
        year += 1;
    }
    u8::try_from(month)
        .ok()
        .and_then(|month| time::Month::try_from(month).ok())
        .and_then(|month| time::Date::from_calendar_date(year, month, 1).ok())
        .unwrap_or(date)
}

/// Six weeks of days covering the month, Monday first.
fn month_grid(month: time::Date) -> Vec<time::Date> {
    let first = first_of_month(month);
    let offset = i64::from(first.weekday().number_days_from_monday());
    let start = first - time::Duration::days(offset);
    (0..42)
        .filter_map(|index| start.checked_add(time::Duration::days(index)))
        .collect()
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
                DocumentStatus::Draft,
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

/// What the document panel asks the caller to do.
enum DocumentAction {
    /// Still open.
    None,
    /// Fold the panel away.
    Close,
    /// Write the artefact of this kind to the chosen path.
    Save(&'static str),
}

/// Draws a stored document read-only.
fn document_panel(
    ui: &mut egui::Ui,
    language: Language,
    view: &DocumentView,
    export_path: &mut String,
) -> DocumentAction {
    let mut action = DocumentAction::None;
    ui.horizontal(|ui| {
        ui.heading(text::find::document_title(
            language,
            &view.number,
            view.doc_type.is_credit_note(),
        ));
        if ui.button(text::find::close_action(language)).clicked() {
            action = DocumentAction::Close;
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
                theme::value(ui, line.position.to_string());
                theme::value(ui, line.name.as_str());
                theme::value(
                    ui,
                    format!("{} {}", line.quantity.to_trimmed_string(), line.unit.as_str()),
                );
                theme::value(ui, money_text(line.unit_price));
                theme::value(ui, line.vat_rate.percent_string());
                theme::value(ui, money_text(line.line_net));
                ui.end_row();
            }
        });

    ui.add_space(6.0);
    theme::value(
        ui,
        text::find::totals(
            language,
            &money_text(view.totals.net),
            &money_text(view.totals.tax),
            &money_text(view.totals.gross),
        ),
    );

    if !view.events.is_empty() {
        ui.add_space(6.0);
        ui.strong(text::find::history_label(language));
        for event in &view.events {
            theme::value(ui, format!("{} · {} · {}", event.at, event.kind, event.actor));
        }
    }

    if !view.artefacts.is_empty() {
        ui.add_space(6.0);
        ui.strong(text::find::artefacts_label(language));
        for artefact in &view.artefacts {
            theme::value(
                ui,
                text::find::artefact_line(language, &artefact.kind, artefact.size_bytes, &artefact.bytes_hash),
            );
        }

        // Exporting re-reads the stored bytes rather than re-rendering, so what
        // leaves the archive is what was filed. The path is one stem for both
        // formats: the extension decides which file is written.
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(text::find::save_to(language));
            ui.add(egui::TextEdit::singleline(export_path).desired_width(360.0));
            if view.artefacts.iter().any(|artefact| artefact.kind == "pdf")
                && ui.button(text::find::save_pdf_action(language)).clicked()
            {
                action = DocumentAction::Save("pdf");
            }
            if view.artefacts.iter().any(|artefact| artefact.kind == "ubl")
                && ui.button(text::find::save_ubl_action(language)).clicked()
            {
                action = DocumentAction::Save("ubl");
            }
        });
    }

    action
}

fn fact(ui: &mut egui::Ui, language: Language, fact: Fact, value: &str) {
    ui.weak(text::find::fact(language, fact));
    if fact == Fact::SemanticHash {
        theme::value_mono(ui, value);
    } else {
        theme::value(ui, value);
    }
    ui.end_row();
}

fn show_findings(ui: &mut egui::Ui, language: Language, findings: &[Finding]) -> Option<FindingAction> {
    let tokens = *theme::palette(ui.ctx());
    let mut action = None;
    for finding in findings.iter().take(12) {
        let color = match finding.severity {
            Severity::Blocking => tokens.error,
            Severity::Warning => tokens.warning,
            Severity::Info => tokens.muted,
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
    fn changing_a_filter_makes_the_request_dirty() {
        let (_dir, database, seller_id) = seeded_archive();
        let mut view = SearchView::default();

        // The first frame runs an empty query, which lists the archive.
        let before = view.request_key();
        view.run(&database, seller_id).unwrap();
        assert_eq!(view.last_request.as_ref(), Some(&before));

        view.query = "acme".to_owned();
        assert_ne!(&view.request_key(), view.last_request.as_ref().unwrap());
        view.run(&database, seller_id).unwrap();
        assert_eq!(view.last_request.as_ref(), Some(&view.request_key()));
        assert_eq!(view.results.len(), 1);
    }

    #[test]
    fn sorting_follows_the_clicked_column() {
        let mut hits = vec![
            SearchHit {
                id: DocumentId::new(),
                direction: Direction::Outbound,
                number: "2026-0002".to_owned(),
                party: "Zeta BV".to_owned(),
                issue_date: LocalDate::from_ymd(2026, 10, 2).unwrap(),
                currency: Currency::EUR,
                gross: Money::from_major_units(20, Currency::EUR).unwrap(),
                status: DocumentStatus::Issued,
            },
            SearchHit {
                id: DocumentId::new(),
                direction: Direction::Outbound,
                number: "2026-0001".to_owned(),
                party: "Alpha BV".to_owned(),
                issue_date: LocalDate::from_ymd(2026, 10, 3).unwrap(),
                currency: Currency::EUR,
                gross: Money::from_major_units(10, Currency::EUR).unwrap(),
                status: DocumentStatus::Issued,
            },
        ];

        sort_hits(&mut hits, 0, false);
        assert_eq!(hits[0].number, "2026-0001");
        sort_hits(&mut hits, 4, true);
        assert_eq!(hits[0].gross.micros(), 20_000_000);
        sort_hits(&mut hits, 3, false);
        assert_eq!(hits[0].issue_date.to_iso_string(), "2026-10-02");
    }

    #[test]
    fn every_tab_draws_without_panicking() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut session = Session::create(
            dir.path().join("bonk.db"),
            dir.path().join("keystore.bin"),
            "passphrase",
        )
        .unwrap();
        insert_seller(session.database().connection(), &seller(), NOW).unwrap();
        let customer = CustomerRecord {
            id: CustomerId::new(),
            party: Party::new("Acme BV", Address::new("Rotterdam", CountryCode::NL)),
        };
        insert_customer(session.database().connection(), &customer, NOW).unwrap();
        let mut workspace = Workspace::new(&session).unwrap();

        for tab in [Tab::Search, Tab::Customers, Tab::Settings] {
            workspace.tab = tab;
            let context = egui::Context::default();
            let mut appearance = egui::ThemePreference::System;
            let output = context.run_ui(egui::RawInput::default(), |ui| {
                let _ = workspace.ui(ui, &mut session, &mut appearance);
            });
            output.drop_without_applying_deltas();
        }
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
            action = workspace.ui(ui, &mut session, &mut egui::ThemePreference::System);
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

    #[test]
    fn shifting_a_month_wraps_both_ways() {
        let january = time::Date::from_calendar_date(2026, time::Month::January, 15).unwrap();
        assert_eq!(shift_month(january, -1).to_string(), "2025-12-01");
        assert_eq!(shift_month(january, 1).to_string(), "2026-02-01");
        let december = time::Date::from_calendar_date(2026, time::Month::December, 31).unwrap();
        assert_eq!(shift_month(december, 1).to_string(), "2027-01-01");
        assert_eq!(shift_month(december, -12).to_string(), "2025-12-01");
    }

    #[test]
    fn the_calendar_grid_is_six_weeks_starting_on_monday() {
        for month in [time::Month::January, time::Month::February, time::Month::August] {
            let first = time::Date::from_calendar_date(2026, month, 1).unwrap();
            let grid = month_grid(first);
            assert_eq!(grid.len(), 42, "{month:?} needs six full weeks");
            assert_eq!(grid[0].weekday(), time::Weekday::Monday);
            assert!(grid.contains(&first));
            for pair in grid.windows(2) {
                assert_eq!(pair[1] - pair[0], time::Duration::days(1));
            }
        }
    }

    #[test]
    fn opening_a_customer_fills_the_editor_from_the_record() {
        let record = CustomerRecord {
            id: CustomerId::new(),
            party: Party::new("Klant BV", Address::new("Leiden", CountryCode::NL)),
        };
        let edit = CustomerEdit::from_record(&record);
        assert_eq!(edit.id, record.id);
        assert_eq!(edit.form.name, "Klant BV");
        assert_eq!(edit.form.city, "Leiden");
        assert_eq!(edit.form.country, "NL");
    }

    #[test]
    fn a_customer_form_normalises_the_phone_against_its_country() {
        let mut form = CustomerForm {
            name: "Klant BV".to_owned(),
            city: "Delft".to_owned(),
            phone: "06 12 34 56 78".to_owned(),
            ..CustomerForm::default()
        };
        assert_eq!(form.to_party().unwrap().phone.as_deref(), Some("+31612345678"));

        form.phone = "not a number".to_owned();
        assert!(form.to_party().is_err(), "garbage must not be stored as a phone");

        form.phone.clear();
        assert_eq!(form.to_party().unwrap().phone, None);
    }

    #[test]
    fn a_stored_draft_reopens_in_the_composer() {
        let (dir, database, _seller_id) = seeded_archive();
        let seller = first_seller(database.connection()).unwrap().unwrap();
        let customer = list_customers(database.connection()).unwrap()[0].clone();

        let mut form = editor();
        form.customer = Some(customer.id);
        form.note = "Concept".to_owned();
        form.due_date = "2026-11-30".to_owned();
        let draft = build_draft(&form, &seller).unwrap();
        insert_draft(database.connection(), &draft, NOW).unwrap();

        let reopened = Editor::from_draft(&draft, render::Language::Dutch);
        assert!(reopened.saved, "a reopened draft says so in the heading");
        assert_eq!(reopened.id, draft.id);
        assert_eq!(reopened.customer, Some(customer.id));
        assert_eq!(reopened.due_date, "2026-11-30");
        assert_eq!(reopened.note, "Concept");
        assert_eq!(reopened.document_language, render::Language::Dutch);
        assert_eq!(reopened.lines.len(), 1);
        assert_eq!(reopened.lines[0].name, "Consulting");
        assert_eq!(reopened.lines[0].unit_price, "50");

        // The round trip is lossless: rebuilding a draft from the editor gives
        // the same facts back.
        let rebuilt = build_draft(&reopened, &seller).unwrap();
        assert_eq!(rebuilt.id, draft.id);
        assert_eq!(rebuilt.note, draft.note);
        assert_eq!(rebuilt.due_date, draft.due_date);
        drop(dir);
    }

    #[test]
    fn the_composer_draws_over_the_list() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut session = Session::create(
            dir.path().join("bonk.db"),
            dir.path().join("keystore.bin"),
            "passphrase",
        )
        .unwrap();
        insert_seller(session.database().connection(), &seller(), NOW).unwrap();
        let customer = CustomerRecord {
            id: CustomerId::new(),
            party: Party::new("Acme BV", Address::new("Rotterdam", CountryCode::NL)),
        };
        insert_customer(session.database().connection(), &customer, NOW).unwrap();
        let mut workspace = Workspace::new(&session).unwrap();

        workspace.start_invoice();
        assert!(workspace.editor.is_some(), "the button starts an invoice");

        let context = egui::Context::default();
        let mut appearance = egui::ThemePreference::System;
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            let _ = workspace.ui(ui, &mut session, &mut appearance);
        });
        output.drop_without_applying_deltas();
    }

    #[test]
    fn a_draft_row_reopens_it_in_the_composer() {
        let dir = tempfile::TempDir::new().unwrap();
        let session = Session::create(
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

        let mut workspace = Workspace::new(&session).unwrap();
        workspace.open_draft(&session, draft.id);
        let reopened = workspace.editor.as_ref().expect("the draft reopened");
        assert_eq!(reopened.id, draft.id);
        assert_eq!(reopened.customer, Some(customer.id));
        assert!(workspace.message.is_none());
    }

    #[test]
    fn an_export_path_names_both_formats() {
        assert_eq!(
            sibling_path("/tmp/2026-0001.pdf", "xml"),
            Path::new("/tmp/2026-0001.xml")
        );
        assert_eq!(sibling_path("/tmp/out", "pdf"), Path::new("/tmp/out.pdf"));
        assert_eq!(sibling_path("  ", "pdf"), Path::new("export.pdf"));
        assert_eq!(artefact_extension("ubl"), "xml");
        assert_eq!(artefact_extension("pdf"), "pdf");
    }
}
