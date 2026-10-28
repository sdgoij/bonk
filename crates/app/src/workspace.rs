//! The unlocked workspace: onboarding, customers, and the invoice editor.
//!
//! The UI never touches SQL directly; it builds a domain draft, asks the domain
//! for totals and findings, and calls the same store functions as the CLI.

use crux::{
    Address, CountryCode, Currency, CustomerId, DocType, DocumentId, DraftLine, Finding, InvoiceDraft, LineId,
    LocalDate, Money, Party, PrecedingInvoice, Quantity, SellerId, Severity, TaxTreatment, UnitCode, VatCategory,
    VatRate, calculate_totals, has_blocking, validate_draft,
};
use store::{
    CustomerRecord, Direction, DocumentRecord, DocumentStatus, IssueRequest, SearchHit, SearchRequest, SellerRecord,
    StoreError, Suppression, attach_artefact, credit_note_for, first_seller, insert_customer, insert_seller,
    issue_draft, list_customers, list_suppressions, save_draft, save_suppression, search,
};

use crate::Session;
use crate::health::Health;

const ERROR_COLOR: egui::Color32 = egui::Color32::from_rgb(200, 60, 60);
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
}

#[derive(Clone)]
struct SuppressionTarget {
    code: String,
    field: String,
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
            lines: self.lines.clone(),
        }
    }
}

impl Editor {
    fn new(customer: Option<CustomerId>) -> Self {
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
            lines: vec![LineForm::default()],
        }
    }

    /// An editor pre-filled from a credit-note draft copied from an issued
    /// invoice: the reference is fixed, the lines and customer are editable.
    fn credit_note(draft: &InvoiceDraft) -> Option<Self> {
        let preceding = draft.preceding_invoice.clone()?;
        let mut editor = Self::new(Some(draft.customer_id));
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

impl Workspace {
    /// Loads the seller and customer list for a freshly unlocked session.
    pub fn new(session: &Session) -> Result<Self, StoreError> {
        let connection = session.database().connection();
        Ok(Self {
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
            health: Health::snapshot(session.database(), session.database_path()).ok(),
        })
    }

    /// Draws the workspace.
    pub fn ui(&mut self, ui: &mut egui::Ui, session: &mut Session) -> WorkspaceAction {
        let mut lock = false;
        let mut focus = self.focus_request.take();
        if ui.input(|input| input.modifiers.command && input.key_pressed(egui::Key::F)) {
            focus = Some("search.query".to_owned());
        }

        ui.horizontal(|ui| {
            ui.heading("Bonk");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Lock").clicked() {
                    lock = true;
                }
            });
        });
        ui.label(format!("Database: {}", session.database_path().display()));
        if !store::cipher_enabled() {
            ui.colored_label(ERROR_COLOR, "UNENCRYPTED BUILD - test only, never store real data");
        }
        ui.separator();

        if let Some(message) = &self.message {
            let color = if message.error { ERROR_COLOR } else { OK_COLOR };
            ui.colored_label(color, message.text.as_str());
            ui.separator();
        }

        if !self.conformance_findings.is_empty() {
            if let Some(action) = show_findings(ui, &self.conformance_findings) {
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
        let mut run = false;
        let mut clear = false;
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.strong("Find documents");
                ui.weak("Ctrl+F");
            });
            ui.horizontal(|ui| {
                let response = text_field(ui, &mut self.search.query, "search.query", 300.0, focus);
                if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                    run = true;
                }
                direction_combo(ui, &mut self.search.direction);
                status_combo(ui, &mut self.search.status);
                if ui.button("Search").clicked() {
                    run = true;
                }
                if ui.button("Clear").clicked() {
                    clear = true;
                }
            });

            if self.search.ran {
                if self.search.results.is_empty() {
                    ui.label(if self.search.query.trim().is_empty() {
                        "No documents in this archive yet."
                    } else {
                        "No documents match. Words are combined, so every word has to appear."
                    });
                } else {
                    search_results_ui(ui, &self.search.results);
                }
            } else {
                ui.label("Search by number, party, reference, or note. Leave it blank to list everything.");
            }
        });

        if clear {
            self.search = SearchView::default();
        }
        if run {
            let Some(owner) = self.seller.as_ref().map(|seller| seller.id) else {
                return;
            };
            if let Err(error) = self.search.run(session.database(), owner) {
                self.message = Some(Message::error(error.to_string()));
            }
        }
    }

    /// Draws the health panel: cheap facts always, the payload audit on request.
    fn health_ui(&mut self, ui: &mut egui::Ui, session: &Session) {
        let mut check = false;
        egui::CollapsingHeader::new("Archive health")
            .default_open(false)
            .show(ui, |ui| {
                check = self.health_details_ui(ui);
            });

        if check {
            self.run_health_check(session);
        }
    }

    /// The panel's contents. Returns true when the user asked for a check.
    fn health_details_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let Some(health) = &self.health else {
            ui.label("Health could not be read from this archive.");
            return false;
        };

        ui.horizontal(|ui| {
            ui.label(format!("schema {}", health.schema_version));
            ui.label("·");
            ui.label(health.encryption);
            ui.label("·");
            ui.label(format!("{} KB", health.database_bytes / 1024));
        });

        match &health.chain {
            Some(head) => {
                let short = health.short_head().unwrap_or_default();
                ui.horizontal(|ui| {
                    ui.label(format!("audit: {} events, head", head.seq));
                    ui.label(egui::RichText::new(short).monospace())
                        .on_hover_text(head.hash.as_str());
                });
            }
            None => {
                ui.label("audit: empty chain");
            }
        }

        match (&health.report, &health.error) {
            (_, Some(error)) => {
                ui.colored_label(ERROR_COLOR, format!("integrity: {error}"));
            }
            (Some(report), None) if report.orphaned_blobs > 0 => {
                ui.colored_label(
                    egui::Color32::from_rgb(190, 140, 40),
                    format!(
                        "integrity: {} payloads verified, {} unreferenced (wasted space only)",
                        report.blobs, report.orphaned_blobs
                    ),
                );
            }
            (Some(report), None) => {
                ui.colored_label(OK_COLOR, format!("integrity: {} payloads verified", report.blobs));
            }
            (None, None) => {
                ui.label("integrity: not checked");
            }
        }
        if let Some(checked) = &health.checked_at {
            ui.weak(format!("checked {checked}"));
        }

        match &health.last_backup {
            Some(backup) => {
                ui.label(format!("last backup: {} ({} KB)", backup.name, backup.bytes / 1024));
            }
            None => {
                ui.label("last backup: none — run bonk backup");
            }
        }

        ui.button("Check integrity now").clicked()
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
        egui::Window::new("Suppress warning")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                ui.label(format!("{} at {}", target.code, target.field));
                ui.label("Reason");
                ui.add(
                    egui::TextEdit::multiline(&mut self.suppression_reason)
                        .desired_width(360.0)
                        .desired_rows(3),
                );
                ui.horizontal(|ui| {
                    if ui.button("Record suppression").clicked() {
                        record = true;
                    }
                    if ui.button("Cancel").clicked() {
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
        let Some(seller) = self.seller.clone() else {
            self.message = Some(Message::error("Create the seller before suppressing findings."));
            return;
        };
        let Some(editor) = self.editor.clone() else {
            self.message = Some(Message::error("Open an invoice before suppressing findings."));
            return;
        };
        let Some(target) = self.suppress_target.clone() else {
            return;
        };
        let reason = self.suppression_reason.trim();
        if reason.is_empty() {
            self.message = Some(Message::error("A suppression reason is required."));
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
                    self.message = Some(Message::ok("Warning suppressed with a recorded reason."));
                }
                Err(error) => self.message = Some(Message::error(error.to_string())),
            },
            Err(error) => self.message = Some(Message::error(error.to_string())),
        }
    }

    fn seller_onboarding(&mut self, ui: &mut egui::Ui, session: &Session, focus: Option<&str>) {
        ui.heading("Create your seller profile");
        ui.label("One legal entity per database in v1.");
        ui.add_space(8.0);

        egui::Grid::new("seller-form").num_columns(2).show(ui, |ui| {
            ui.label("Legal name");
            text_field(ui, &mut self.seller_form.name, "seller.name", 200.0, focus);
            ui.end_row();
            ui.label("Street");
            text_field(ui, &mut self.seller_form.street, "seller.street", 200.0, focus);
            ui.end_row();
            ui.label("Post code");
            text_field(ui, &mut self.seller_form.postal, "seller.postal_code", 200.0, focus);
            ui.end_row();
            ui.label("City");
            text_field(ui, &mut self.seller_form.city, "seller.city", 200.0, focus);
            ui.end_row();
            ui.label("Country");
            text_field(ui, &mut self.seller_form.country, "seller.country", 200.0, focus);
            ui.end_row();
            ui.label("VAT ID");
            text_field(ui, &mut self.seller_form.vat, "seller.vat_id", 200.0, focus);
            ui.end_row();
            ui.label("KvK number");
            text_field(ui, &mut self.seller_form.kvk, "seller.kvk_number", 200.0, focus);
            ui.end_row();
            ui.label("Peppol ID");
            text_field(ui, &mut self.seller_form.peppol, "seller.peppol", 200.0, focus);
            ui.end_row();
        });

        ui.add_space(8.0);
        if ui.button("Create seller").clicked() {
            self.create_seller(session);
        }
    }

    fn unlocked_ui(&mut self, ui: &mut egui::Ui, session: &mut Session, focus: Option<&str>) {
        let mut start_credit = false;
        if let Some(issued) = &self.issued {
            ui.heading(if issued.number.contains("-C") {
                "Issued credit note"
            } else {
                "Issued"
            });
            ui.label(format!("Number: {}", issued.number));
            ui.label(format!("Issue date: {}", issued.issue_date));
            ui.label(format!("Payable: {}", money_text(issued.totals.gross)));
            ui.label(format!("UBL SHA-256: {}", issued.ubl_sha256));
        }

        if self.issued.is_some() {
            let is_credit_note = self.issued.as_ref().is_some_and(|issued| issued.number.contains("-C"));
            let mut save_pdf = false;
            if let Some(pdf) = &self.pdf {
                ui.label(format!("PDF SHA-256: {}", pdf.hash));
                ui.horizontal(|ui| {
                    ui.label("Save PDF to");
                    ui.add(egui::TextEdit::singleline(&mut self.pdf_path).desired_width(420.0));
                    save_pdf = ui.button("Save PDF").clicked();
                });
            }

            ui.add_space(8.0);
            let mut reset = false;
            ui.horizontal(|ui| {
                if !is_credit_note && ui.button("Credit note").clicked() {
                    start_credit = true;
                }
                if ui.button("New invoice").clicked() {
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
            ui.label(format!("Seller: {}", seller.party.legal_name));
        }

        ui.horizontal(|ui| {
            if ui.button("New invoice").clicked() {
                let customer = self.customers.first().map(|customer| customer.id);
                self.editor = Some(Editor::new(customer));
                self.suppressions.clear();
                self.suppress_target = None;
                self.suppression_reason.clear();
            }
            if ui.button("Add customer").clicked() {
                self.show_customer_form = !self.show_customer_form;
            }
        });

        if self.show_customer_form {
            ui.group(|ui| {
                ui.label("New customer");
                egui::Grid::new("customer-form").num_columns(2).show(ui, |ui| {
                    ui.label("Name");
                    text_field(ui, &mut self.customer_form.name, "customer.name", 200.0, focus);
                    ui.end_row();
                    ui.label("Street");
                    text_field(ui, &mut self.customer_form.street, "customer.street", 200.0, focus);
                    ui.end_row();
                    ui.label("Post code");
                    text_field(ui, &mut self.customer_form.postal, "customer.postal_code", 200.0, focus);
                    ui.end_row();
                    ui.label("City");
                    text_field(ui, &mut self.customer_form.city, "customer.city", 200.0, focus);
                    ui.end_row();
                    ui.label("Country");
                    text_field(ui, &mut self.customer_form.country, "customer.country", 200.0, focus);
                    ui.end_row();
                    ui.label("VAT ID");
                    text_field(ui, &mut self.customer_form.vat, "customer.vat_id", 200.0, focus);
                    ui.end_row();
                    ui.label("Peppol ID");
                    text_field(ui, &mut self.customer_form.peppol, "customer.peppol", 200.0, focus);
                    ui.end_row();
                });
                if ui.button("Save customer").clicked() {
                    self.create_customer(session);
                }
            });
        }

        if self.editor.is_some() {
            self.editor_ui(ui, session, focus);
        } else {
            ui.add_space(8.0);
            ui.label("Open a new invoice to start. Existing drafts are not listed yet.");
        }
    }

    fn editor_ui(&mut self, ui: &mut egui::Ui, session: &mut Session, focus: Option<&str>) {
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
                ui.colored_label(OK_COLOR, format!("Credit note for invoice {number}"));
            }
            ui.horizontal(|ui| {
                ui.label("Customer");
                let selected = editor
                    .customer
                    .and_then(|id| customers.iter().find(|customer| customer.id == id))
                    .map(|customer| customer.party.legal_name.clone())
                    .unwrap_or_else(|| "Select customer".to_owned());
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
                ui.label("Issue date");
                text_field(ui, &mut editor.issue_date, "issue_date", 110.0, focus);
                ui.label("Due date");
                text_field(ui, &mut editor.due_date, "due_date", 110.0, focus);
                ui.end_row();
                ui.label("Buyer ref");
                text_field(ui, &mut editor.buyer_reference, "buyer_reference", 110.0, focus);
                ui.label("Order ref");
                text_field(ui, &mut editor.order_reference, "order_reference", 110.0, focus);
                ui.end_row();
                ui.label("Delivery date");
                text_field(ui, &mut editor.delivery_date, "delivery_date", 110.0, focus);
                ui.label("");
                ui.label("");
                ui.end_row();
            });
            ui.horizontal(|ui| {
                ui.label("Note");
                text_field(ui, &mut editor.note, "note", 420.0, focus);
            });

            ui.add_space(8.0);
            egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                egui::Grid::new("lines-form")
                    .num_columns(6)
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label("Item");
                        ui.label("Qty");
                        ui.label("Unit price");
                        ui.label("Rate %");
                        ui.label("VAT");
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
                            if ui.button("Remove").clicked() {
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
            if ui.button("Add line").clicked() {
                editor.lines.push(LineForm::default());
            }

            ui.add_space(8.0);
            save = ui.button("Save draft").clicked();
            issue = ui.button("Issue").clicked();
        }

        let mut finding_action = None;
        if let Some(editor) = &self.editor {
            match build_draft(editor, &seller) {
                Ok(draft) => {
                    match calculate_totals(&draft.lines) {
                        Ok(totals) => {
                            ui.label(format!(
                                "Net {}   VAT {}   Total {}",
                                money_text(totals.net),
                                money_text(totals.tax),
                                money_text(totals.gross)
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
                            if let Some(action) = show_findings(ui, &visible) {
                                finding_action = Some(action);
                            }
                            if suppressed > 0 {
                                ui.label(format!("{suppressed} warning(s) suppressed with a recorded reason"));
                            }
                        }
                        None => {
                            ui.colored_label(ERROR_COLOR, "Select a customer to validate.");
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
        let result = credit_note_for(
            session.database().connection(),
            seller_id,
            &number,
            LocalDate::today_utc(),
        );
        match result {
            Ok(Some(draft)) => match Editor::credit_note(&draft) {
                Some(editor) => {
                    self.reset_document();
                    self.editor = Some(editor);
                    self.message = Some(Message::ok(format!("Credit note for {number}")));
                }
                None => self.message = Some(Message::error("the draft has no preceding invoice".to_owned())),
            },
            Ok(None) => {
                self.message = Some(Message::error(format!("invoice {number} was not found")));
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
                self.message = Some(Message::ok("Seller created."));
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
                self.message = Some(Message::ok("Customer created."));
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
                self.message = Some(Message::ok("Draft saved."));
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
                self.message = Some(Message::error("Select a customer."));
                return;
            }
        };

        let findings = validate_draft(&draft, &seller.party, &customer.party);
        if has_blocking(&findings) {
            let count = findings
                .iter()
                .filter(|finding| finding.severity == Severity::Blocking)
                .count();
            self.message = Some(Message::error(format!(
                "{count} blocking finding(s); nothing was issued."
            )));
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
                match crate::invoice_pdf(&issued.number, &draft, &seller, &customer, &issued.totals) {
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
                        self.message = Some(Message::ok(format!("Issued. PDF not rendered: {error}")));
                    }
                }

                self.issued = Some(issued);
                self.editor = None;
                self.conformance_findings.clear();
                if let Some(error) = attach_error {
                    self.message = Some(Message::error(format!("Issued, but the PDF was not attached: {error}")));
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
                self.message = Some(Message::error(format!(
                    "{count} conformance finding(s); nothing was issued."
                )));
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
            self.message = Some(Message::error("Choose a file name for the PDF."));
            return;
        }
        match std::fs::write(path, &pdf.bytes) {
            Ok(()) => self.message = Some(Message::ok(format!("PDF saved to {path}"))),
            Err(error) => self.message = Some(Message::error(format!("Could not save the PDF: {error}"))),
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

fn direction_combo(ui: &mut egui::Ui, value: &mut Option<Direction>) {
    let label = match value {
        None => "any side",
        Some(Direction::Outbound) => "issued",
        Some(Direction::Inbound) => "received",
    };
    egui::ComboBox::from_id_salt("search.direction")
        .selected_text(label)
        .show_ui(ui, |ui| {
            ui.selectable_value(value, None, "any side");
            ui.selectable_value(value, Some(Direction::Outbound), "issued");
            ui.selectable_value(value, Some(Direction::Inbound), "received");
        });
}

fn status_combo(ui: &mut egui::Ui, value: &mut Option<DocumentStatus>) {
    let label = value
        .map(|status| status.to_string())
        .unwrap_or_else(|| "any status".to_owned());
    egui::ComboBox::from_id_salt("search.status")
        .selected_text(label)
        .show_ui(ui, |ui| {
            ui.selectable_value(value, None, "any status");
            for status in [
                DocumentStatus::Issued,
                DocumentStatus::Received,
                DocumentStatus::Approved,
                DocumentStatus::Paid,
                DocumentStatus::Archived,
            ] {
                ui.selectable_value(value, Some(status), status.to_string());
            }
        });
}

fn search_results_ui(ui: &mut egui::Ui, results: &[SearchHit]) {
    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
        egui::Grid::new("search-results")
            .num_columns(6)
            .striped(true)
            .show(ui, |ui| {
                for heading in ["Number", "Side", "Party", "Issued", "Gross", "Status"] {
                    ui.strong(heading);
                }
                ui.end_row();
                for hit in results {
                    ui.label(hit.number.as_str());
                    ui.label(hit.direction.to_string());
                    ui.label(hit.party.as_str());
                    ui.label(hit.issue_date.to_iso_string());
                    ui.label(format!("{} {}", money_text(hit.gross), hit.currency.as_str()));
                    ui.label(hit.status.to_string());
                    ui.end_row();
                }
            });
    });
    ui.weak(format!(
        "{} document(s), best match or newest first, at most {}.",
        results.len(),
        store::SEARCH_LIMIT
    ));
}

fn show_findings(ui: &mut egui::Ui, findings: &[Finding]) -> Option<FindingAction> {
    let mut action = None;
    for finding in findings.iter().take(12) {
        let color = match finding.severity {
            Severity::Blocking => ERROR_COLOR,
            Severity::Warning => egui::Color32::from_rgb(190, 140, 40),
            Severity::Info => egui::Color32::GRAY,
        };
        let bt = finding.bt.unwrap_or("-");
        let text = format!("{} [{}] {}: {}", finding.code, bt, finding.field, finding.message);
        let can_focus = focusable(&finding.field);
        let can_suppress = finding.severity == Severity::Warning;
        if can_focus || can_suppress {
            ui.horizontal(|ui| {
                ui.colored_label(color, text);
                if can_focus && ui.small_button("Focus").clicked() {
                    action = Some(FindingAction::Focus(finding.field.clone()));
                }
                if can_suppress && ui.small_button("Suppress").clicked() {
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
        let mut editor = Editor::new(Some(CustomerId::new()));
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
    fn the_workspace_draws_without_panicking() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut session = Session::create(
            dir.path().join("bonk.db"),
            dir.path().join("keystore.bin"),
            "passphrase",
        )
        .unwrap();
        insert_seller(session.database().connection(), &seller(), NOW).unwrap();

        let mut workspace = Workspace::new(&session).unwrap();
        let context = egui::Context::default();
        let mut action = WorkspaceAction::None;
        // Drawing is where duplicate widget ids and missing fonts show up as a
        // panic, so a headless frame is worth more than it looks.
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            action = workspace.ui(ui, &mut session);
            // The panel is collapsed by default, so draw its contents directly:
            // a broken widget inside a closed header is still a broken widget.
            let _ = workspace.health_details_ui(ui);
        });
        output.drop_without_applying_deltas();
        assert_eq!(action, WorkspaceAction::None);
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
