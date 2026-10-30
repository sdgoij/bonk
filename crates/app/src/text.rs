//! User-facing interface text.
//!
//! English first. Every string the shell shows lives here rather than in the
//! widget call, so adding a language means a second table and a lookup, not a
//! hunt through the drawing code. Text the user wrote — invoice descriptions,
//! notes, customer names — is theirs and never comes from here.
//!
//! The `_` parameter on each function is the language it will be selected by;
//! until a second table exists every function ignores it, and the call sites do
//! not change when one arrives.

use crate::strength::{Reason, Strength};

/// Languages the interface can speak.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Language {
    /// The only table so far.
    #[default]
    English,
}

/// The lock screen.
pub mod shell {
    use super::Language;

    /// The product's working name. A name does not translate.
    pub const PRODUCT: &str = "Bonk";

    /// The line under the name, before anything is unlocked.
    pub fn tagline(_: Language) -> &'static str {
        "Local-first invoicing. Nothing leaves this machine unless you send it."
    }

    /// Label above the database path.
    pub fn database_label(_: Language) -> &'static str {
        "Database"
    }

    /// Label above the keystore path.
    pub fn keystore_label(_: Language) -> &'static str {
        "Keystore"
    }

    /// Label above the passphrase field.
    pub fn passphrase_label(_: Language) -> &'static str {
        "Passphrase"
    }

    /// Opens an existing archive.
    pub fn unlock_action(_: Language) -> &'static str {
        "Unlock"
    }

    /// Starts creating one.
    pub fn create_database_action(_: Language) -> &'static str {
        "Create new database"
    }

    /// Switches to the recovery route.
    pub fn forgotten_action(_: Language) -> &'static str {
        "Forgotten the passphrase?"
    }

    /// Opens the archive with the key this device kept.
    pub fn unlock_with_device(_: Language) -> &'static str {
        "Unlock with this device"
    }

    /// Explains what creating an archive does.
    pub fn create_note(_: Language) -> &'static str {
        "A new archive is created empty and sealed with this passphrase. Nothing else can open it, so type it twice."
    }

    /// Label above the first field when creating.
    pub fn new_passphrase_label(_: Language) -> &'static str {
        "New passphrase"
    }

    /// Label above the second field when creating.
    pub fn repeat_passphrase_label(_: Language) -> &'static str {
        "Repeat the passphrase"
    }

    /// Confirms a creation.
    pub fn create_action(_: Language) -> &'static str {
        "Create"
    }

    /// Leaves the current form.
    pub fn back_action(_: Language) -> &'static str {
        "Back"
    }

    /// Explains the recovery route.
    pub fn reset_note(_: Language) -> &'static str {
        "A recovery key opens this archive without the passphrase. The key used here is replaced by the reset, because it has now been typed somewhere."
    }

    /// Label above the recovery-key field.
    pub fn recovery_key_label(_: Language) -> &'static str {
        "Recovery key"
    }

    /// Label above the second field when resetting.
    pub fn repeat_new_passphrase_label(_: Language) -> &'static str {
        "Repeat the new passphrase"
    }

    /// Performs the reset.
    pub fn reset_action(_: Language) -> &'static str {
        "Reset passphrase"
    }

    /// Tells the user the reset happened and the old key is spent.
    pub fn reset_done_note(_: Language) -> &'static str {
        "Passphrase replaced. The recovery key that was used no longer works."
    }

    /// Introduces the replacement key.
    pub fn new_recovery_key_label(_: Language) -> &'static str {
        "New recovery key, shown once:"
    }

    /// Tells the user what to do with a key.
    pub fn keep_key_note(_: Language) -> &'static str {
        "Write it down and keep it apart from the database and the passphrase."
    }

    /// Acknowledges the replacement key.
    pub fn written_down_action(_: Language) -> &'static str {
        "I have written it down"
    }
}

/// The strength estimate shown under a passphrase field.
pub mod strength {
    use super::{Language, Reason, Strength};

    /// Label before the level itself.
    pub fn label(_: Language) -> &'static str {
        "Strength"
    }

    /// The level, as one word.
    pub fn level(_: Language, strength: Strength) -> &'static str {
        match strength {
            Strength::Weak => "weak",
            Strength::Fair => "fair",
            Strength::Strong => "strong",
        }
    }

    /// One sentence of advice for the level.
    pub fn reason(_: Language, reason: Reason) -> &'static str {
        match reason {
            Reason::Empty => "enter a passphrase",
            Reason::Common => "this is one of the first phrases an attacker tries",
            Reason::Repetition => "repeating a short pattern adds almost nothing",
            Reason::TooShort => "too short; a few unrelated words beat a short scramble",
            Reason::Fair => "acceptable; length helps more than clever substitutions",
            Reason::Strong => "good: length and variety both count",
        }
    }

    /// The hover text, which explains what the number means.
    pub fn detail(language: Language, bits: u32) -> String {
        match language {
            Language::English => format!(
                "At least {bits} bits by this estimate. Every guess costs the attacker one Argon2id run, so \
                 length is what makes a copied keystore expensive to open."
            ),
        }
    }
}

/// The workspace chrome around the screens.
pub mod workspace {
    use super::Language;

    /// Leaves the archive open and returns to the lock screen.
    pub fn lock_action(_: Language) -> &'static str {
        "Lock"
    }

    /// The line naming the open archive.
    pub fn database_line(language: Language, path: &str) -> String {
        match language {
            Language::English => format!("Database: {path}"),
        }
    }

    /// Shown when the build stores the archive unencrypted.
    pub fn unencrypted_warning(_: Language) -> &'static str {
        "UNENCRYPTED BUILD - test only, never store real data"
    }

    /// Hides the last message.
    pub fn dismiss_action(_: Language) -> &'static str {
        "Dismiss"
    }
}

/// The navigation rail. The rail shows icons; these are the names they carry.
pub mod tabs {
    use super::Language;

    /// Find documents.
    pub fn search(_: Language) -> &'static str {
        "Search"
    }

    /// The party list.
    pub fn customers(_: Language) -> &'static str {
        "Customers"
    }

    /// Archive settings.
    pub fn settings(_: Language) -> &'static str {
        "Settings"
    }
}

/// The find screen and the document it opens.
pub mod find {
    use super::Language;
    use store::{Direction, DocumentStatus};

    /// Panel heading.
    pub fn title(_: Language) -> &'static str {
        "Find documents"
    }

    /// The keyboard hint beside the heading.
    pub fn shortcut(_: Language) -> &'static str {
        "Ctrl+F"
    }

    /// Hint in the search field.
    pub fn hint(_: Language) -> &'static str {
        "Search number, party, reference, or note"
    }

    /// Runs the query.
    pub fn search_action(_: Language) -> &'static str {
        "Search"
    }

    /// Resets the panel.
    pub fn clear_action(_: Language) -> &'static str {
        "Clear"
    }

    /// Label before the saved-filter list.
    pub fn saved_label(_: Language) -> &'static str {
        "Saved"
    }

    /// Shown in the saved list when nothing is applied.
    pub fn no_saved(_: Language) -> &'static str {
        "none"
    }

    /// Hint in the field that names a saved search.
    pub fn save_name_hint(_: Language) -> &'static str {
        "name"
    }

    /// Stores the current filters under the typed name.
    pub fn save_current_action(_: Language) -> &'static str {
        "Save current"
    }

    /// Removes the applied saved search.
    pub fn delete_action(_: Language) -> &'static str {
        "Delete"
    }

    /// Nothing to list, before any query.
    pub fn empty_archive(_: Language) -> &'static str {
        "No documents in this archive yet."
    }

    /// A query that found nothing.
    pub fn no_matches(_: Language) -> &'static str {
        "No documents match. Words are combined, so every word has to appear."
    }

    /// What the panel is for, before it is used.
    pub fn prompt(_: Language) -> &'static str {
        "Search by number, party, reference, or note. Leave it blank to list everything."
    }

    /// No side filter.
    pub fn any_side(_: Language) -> &'static str {
        "any side"
    }

    /// No status filter.
    pub fn any_status(_: Language) -> &'static str {
        "any status"
    }

    /// A side of the books.
    pub fn direction(_: Language, direction: Direction) -> &'static str {
        match direction {
            Direction::Outbound => "issued",
            Direction::Inbound => "received",
        }
    }

    /// Where a document stands.
    pub fn status(_: Language, status: DocumentStatus) -> &'static str {
        match status {
            DocumentStatus::Draft => "draft",
            DocumentStatus::Issued => "issued",
            DocumentStatus::Received => "received",
            DocumentStatus::Approved => "approved",
            DocumentStatus::Paid => "paid",
            DocumentStatus::Archived => "archived",
        }
    }

    /// What an empty cell shows.
    pub fn no_value(_: Language) -> &'static str {
        "—"
    }

    /// Result columns, in order.
    pub fn columns(_: Language) -> [&'static str; 6] {
        ["Number", "Side", "Party", "Issued", "Gross", "Status"]
    }

    /// The pinned footer: how many the filters match, and how many the archive
    /// holds. `42/1337` is easier to read than a sentence, and the second number
    /// says what a wider search would find.
    pub fn count_line(_: Language, matched: u64, total: u64) -> String {
        format!("{matched}/{total}")
    }

    /// The heading of an opened document.
    pub fn document_title(language: Language, number: &str, credit_note: bool) -> String {
        let kind = match (language, credit_note) {
            (Language::English, false) => "invoice",
            (Language::English, true) => "credit note",
        };
        format!("{number} · {kind}")
    }

    /// Says the opened document cannot be edited.
    pub fn stored_note(_: Language) -> &'static str {
        "Stored as it was issued. Nothing here can be edited; a correction is a credit note."
    }

    /// Closes the opened document.
    pub fn close_action(_: Language) -> &'static str {
        "Close"
    }

    /// Labels in the fact grid.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Fact {
        /// Which side of the books.
        Side,
        /// Where it stands.
        Status,
        /// Issue date.
        Issued,
        /// Due date.
        Due,
        /// Delivery date.
        Delivered,
        /// Buyer reference.
        BuyerReference,
        /// Order reference.
        OrderReference,
        /// Who issued it.
        Seller,
        /// Who received it.
        Buyer,
        /// Free-text note.
        Note,
        /// When the archive recorded it.
        Recorded,
        /// Hash of the semantic record.
        SemanticHash,
    }

    /// The word for a fact.
    pub fn fact(_: Language, fact: Fact) -> &'static str {
        match fact {
            Fact::Side => "Side",
            Fact::Status => "Status",
            Fact::Issued => "Issued",
            Fact::Due => "Due",
            Fact::Delivered => "Delivered",
            Fact::BuyerReference => "Buyer reference",
            Fact::OrderReference => "Order reference",
            Fact::Seller => "Seller",
            Fact::Buyer => "Buyer",
            Fact::Note => "Note",
            Fact::Recorded => "Recorded",
            Fact::SemanticHash => "Semantic SHA-256",
        }
    }

    /// Line columns, in order.
    pub fn line_columns(_: Language) -> [&'static str; 6] {
        ["#", "Item", "Qty", "Unit price", "VAT", "Net"]
    }

    /// The totals row.
    pub fn totals(language: Language, net: &str, tax: &str, gross: &str) -> String {
        match language {
            Language::English => format!("Net {net} · VAT {tax} · Payable {gross}"),
        }
    }

    /// Heading over the status events, when there are any.
    pub fn history_label(_: Language) -> &'static str {
        "History"
    }

    /// Heading over the artefact hashes, when there are any.
    pub fn artefacts_label(_: Language) -> &'static str {
        "Artefacts"
    }

    /// One artefact: kind, size, hash.
    pub fn artefact_line(language: Language, kind: &str, size: i64, hash: &str) -> String {
        match language {
            Language::English => format!("{kind} · {size} bytes · {hash}"),
        }
    }

    /// Label beside the export path field.
    pub fn save_to(_: Language) -> &'static str {
        "Save to"
    }

    /// Writes the filed PDF.
    pub fn save_pdf_action(_: Language) -> &'static str {
        "Save PDF"
    }

    /// Writes the filed UBL.
    pub fn save_ubl_action(_: Language) -> &'static str {
        "Save UBL"
    }

    /// No export path was typed.
    pub fn choose_export_path(_: Language) -> &'static str {
        "Choose where to save the export first."
    }

    /// What was written, and where.
    pub fn export_saved(_: Language, path: &str) -> String {
        format!("Saved {path}")
    }

    /// Writing failed.
    pub fn export_failed(_: Language, error: &str) -> String {
        format!("Could not save the export: {error}")
    }
}

/// The customer tab.
pub mod customers {
    use super::Language;

    /// The tab heading.
    pub fn title(_: Language) -> &'static str {
        "Customers"
    }

    /// Hint in the filter field.
    pub fn filter_hint(_: Language) -> &'static str {
        "Filter by name, city, or VAT"
    }

    /// Opens the new-customer form.
    pub fn new_customer_action(_: Language) -> &'static str {
        "New customer"
    }

    /// Heading of the inline detail editor.
    pub fn edit_heading(_: Language) -> &'static str {
        "Edit customer"
    }

    /// A saved edit.
    pub fn updated(language: Language, name: &str) -> String {
        match language {
            Language::English => format!("Saved {name}."),
        }
    }

    /// Closes the new-customer form.
    pub fn cancel_action(_: Language) -> &'static str {
        "Cancel"
    }

    /// Table headings, in order.
    pub fn columns(_: Language) -> [&'static str; 5] {
        ["Name", "City", "Country", "VAT", "Peppol"]
    }

    /// Nothing on file.
    pub fn empty(_: Language) -> &'static str {
        "No customers yet. Add one below, or import a CSV."
    }

    /// The filter found nothing.
    pub fn no_matches(_: Language) -> &'static str {
        "No customers match that filter."
    }

    /// Label above the CSV path.
    pub fn path_label(_: Language) -> &'static str {
        "CSV file"
    }

    /// Reads the CSV.
    pub fn import_action(_: Language) -> &'static str {
        "Import"
    }

    /// Writes the CSV.
    pub fn export_action(_: Language) -> &'static str {
        "Export"
    }

    /// What the file holds.
    pub fn csv_note(_: Language) -> &'static str {
        "Columns: name, street, postal, city, country, vat, peppol, contact, email."
    }

    /// A successful export.
    pub fn exported(language: Language, count: usize, path: &str) -> String {
        match language {
            Language::English => format!("Exported {count} customer(s) to {path}."),
        }
    }

    /// A successful import.
    pub fn imported(language: Language, imported: usize, skipped: usize, errors: usize) -> String {
        match language {
            Language::English => {
                let mut text = format!("Imported {imported} customer(s); {skipped} already on file");
                if errors > 0 {
                    text.push_str(&format!(", {errors} row(s) skipped with an error"));
                }
                text.push('.');
                text
            }
        }
    }

    /// Why a file operation failed.
    pub fn failed(language: Language, error: &str) -> String {
        match language {
            Language::English => format!("The customer file could not be read: {error}"),
        }
    }
}

/// The settings tab.
pub mod settings {
    use super::Language;

    /// The tab heading.
    pub fn title(_: Language) -> &'static str {
        "Settings"
    }

    /// The appearance card.
    pub fn appearance(_: Language) -> &'static str {
        "Appearance"
    }

    /// Follow the operating system.
    pub fn theme_system(_: Language) -> &'static str {
        "System"
    }

    /// The light theme.
    pub fn theme_light(_: Language) -> &'static str {
        "Light"
    }

    /// The dark theme.
    pub fn theme_dark(_: Language) -> &'static str {
        "Dark"
    }

    /// What the choice means.
    pub fn theme_note(_: Language) -> &'static str {
        "Follows the operating system unless you choose otherwise."
    }

    /// The archive card.
    pub fn archive(_: Language) -> &'static str {
        "Archive"
    }

    /// The database path.
    pub fn database(_: Language) -> &'static str {
        "Database"
    }

    /// The encryption label.
    pub fn encryption(_: Language) -> &'static str {
        "Encryption"
    }

    /// The health card.
    pub fn health(_: Language) -> &'static str {
        "Archive health"
    }

    /// The backup card.
    pub fn backup(_: Language) -> &'static str {
        "Backup"
    }

    /// What a backup is.
    pub fn backup_note(_: Language) -> &'static str {
        "A verified, encrypted copy kept beside the archive."
    }

    /// Runs a backup now.
    pub fn backup_now(_: Language) -> &'static str {
        "Back up now"
    }

    /// Where backups go.
    pub fn backup_directory(language: Language, path: &str) -> String {
        match language {
            Language::English => format!("Backups: {path}"),
        }
    }

    /// A successful backup.
    pub fn backup_done(language: Language, path: &str, kilobytes: u64) -> String {
        match language {
            Language::English => format!("Backed up to {path} ({kilobytes} KB), verified."),
        }
    }

    /// Why a backup failed.
    pub fn backup_failed(language: Language, error: &str) -> String {
        match language {
            Language::English => format!("The backup could not be written: {error}"),
        }
    }

    /// The credentials card.
    pub fn credentials(_: Language) -> &'static str {
        "Credentials"
    }
}

/// The seller and customer forms, the invoice editor, and what follows issuing.
pub mod editor {
    use super::Language;

    /// Labels a party form uses.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Field {
        /// Registered name of a legal entity.
        LegalName,
        /// Name of a customer.
        Name,
        /// Street and number.
        Street,
        /// Post code.
        PostCode,
        /// City.
        City,
        /// Country code.
        Country,
        /// VAT identifier.
        VatId,
        /// Chamber of commerce number.
        Kvk,
        /// Peppol participant identifier.
        Peppol,
        /// Contact person.
        Contact,
        /// Contact email.
        Email,
        /// Contact phone, in E.164 form.
        Phone,
    }

    /// The word for a party field.
    pub fn field(_: Language, field: Field) -> &'static str {
        match field {
            Field::LegalName => "Legal name",
            Field::Name => "Name",
            Field::Street => "Street",
            Field::PostCode => "Post code",
            Field::City => "City",
            Field::Country => "Country",
            Field::VatId => "VAT ID",
            Field::Kvk => "KvK number",
            Field::Peppol => "Peppol ID",
            Field::Contact => "Contact",
            Field::Email => "Email",
            Field::Phone => "Phone",
        }
    }

    /// Heading of the first-run seller form.
    pub fn seller_heading(_: Language) -> &'static str {
        "Create your seller profile"
    }

    /// What the first-run form promises.
    pub fn seller_note(_: Language) -> &'static str {
        "One legal entity per database in v1."
    }

    /// Stores the seller.
    pub fn create_seller_action(_: Language) -> &'static str {
        "Create seller"
    }

    /// Heading of the customer form.
    pub fn new_customer_heading(_: Language) -> &'static str {
        "New customer"
    }

    /// Stores the customer.
    pub fn save_customer_action(_: Language) -> &'static str {
        "Save customer"
    }

    /// Opens the customer form.
    pub fn add_customer_action(_: Language) -> &'static str {
        "Add customer"
    }

    /// The draft the user asked for is gone.
    pub fn draft_missing(_: Language) -> &'static str {
        "That draft is no longer in this archive; run the search again."
    }

    /// The editor needs a party to bill.
    pub fn no_customers(_: Language) -> &'static str {
        "Add a customer before starting an invoice."
    }

    /// Tooltip on the calendar button.
    pub fn choose_date(_: Language) -> &'static str {
        "Choose a date"
    }

    /// Tooltip on the button that empties an optional date.
    pub fn clear_date(_: Language) -> &'static str {
        "Clear the date"
    }

    /// Weekday headings in the calendar, Monday first.
    pub fn weekdays(_: Language) -> [&'static str; 7] {
        ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"]
    }

    /// A month's name, for the calendar heading.
    pub fn month_name(_: Language, month: time::Month) -> &'static str {
        match month {
            time::Month::January => "January",
            time::Month::February => "February",
            time::Month::March => "March",
            time::Month::April => "April",
            time::Month::May => "May",
            time::Month::June => "June",
            time::Month::July => "July",
            time::Month::August => "August",
            time::Month::September => "September",
            time::Month::October => "October",
            time::Month::November => "November",
            time::Month::December => "December",
        }
    }

    /// Switches to the customer tab.
    pub fn go_to_customers(_: Language) -> &'static str {
        "Go to customers"
    }

    /// The name of the seller the workspace belongs to.
    pub fn seller_line(language: Language, name: &str) -> String {
        match language {
            Language::English => format!("Seller: {name}"),
        }
    }

    /// Heading after issuing.
    pub fn issued_heading(credit_note: bool) -> &'static str {
        if credit_note { "Issued credit note" } else { "Issued" }
    }

    /// Heading over a composer that reopened a stored draft.
    pub fn draft_heading(_: Language) -> &'static str {
        "Draft"
    }

    /// Tooltip on the button that puts the composer away.
    pub fn close_composer(_: Language) -> &'static str {
        "Close without issuing"
    }

    /// Renders what the invoice will look like.
    pub fn preview_action(_: Language) -> &'static str {
        "Preview"
    }

    /// Heading over the rendered preview.
    pub fn preview_heading(_: Language) -> &'static str {
        "Preview"
    }

    /// Tooltip on the button that hides the preview.
    pub fn close_preview(_: Language) -> &'static str {
        "Hide the preview"
    }

    /// The number field of a preview: nothing has been allocated yet.
    pub fn preview_number(_: Language) -> &'static str {
        "(not issued)"
    }

    /// The preview could not be rendered.
    pub fn preview_failed(_: Language, error: &str) -> String {
        format!("Could not render the preview: {error}")
    }

    /// A note under a preview that runs to more than one page.
    pub fn preview_more_pages(language: Language, pages: usize) -> String {
        match language {
            Language::English => format!("The document runs to {pages} pages; the preview shows the first."),
        }
    }

    /// Number of the issued document.
    pub fn number_line(language: Language, number: &str) -> String {
        match language {
            Language::English => format!("Number: {number}"),
        }
    }

    /// Issue date of the issued document.
    pub fn issue_date_line(language: Language, date: &str) -> String {
        match language {
            Language::English => format!("Issue date: {date}"),
        }
    }

    /// Amount payable.
    pub fn payable_line(language: Language, amount: &str) -> String {
        match language {
            Language::English => format!("Payable: {amount}"),
        }
    }

    /// Hash of the UBL artefact.
    pub fn ubl_hash_line(language: Language, hash: &str) -> String {
        match language {
            Language::English => format!("UBL SHA-256: {hash}"),
        }
    }

    /// Hash of the PDF artefact.
    pub fn pdf_hash_line(language: Language, hash: &str) -> String {
        match language {
            Language::English => format!("PDF SHA-256: {hash}"),
        }
    }

    /// Label above the PDF path field.
    pub fn save_pdf_to(_: Language) -> &'static str {
        "Save PDF to"
    }

    /// Writes the PDF to the named path.
    pub fn save_pdf_action(_: Language) -> &'static str {
        "Save PDF"
    }

    /// Starts a credit note for the issued invoice.
    pub fn credit_note_action(_: Language) -> &'static str {
        "Credit note"
    }

    /// Starts an empty invoice.
    pub fn new_invoice_action(_: Language) -> &'static str {
        "New invoice"
    }

    /// Banner at the top of a credit-note editor.
    pub fn credit_note_banner(language: Language, number: &str) -> String {
        match language {
            Language::English => format!("Credit note for invoice {number}"),
        }
    }

    /// Label above the customer picker.
    pub fn customer_label(_: Language) -> &'static str {
        "Customer"
    }

    /// Shown in the customer picker before one is chosen.
    pub fn select_customer(_: Language) -> &'static str {
        "Select customer"
    }

    /// Issue date field.
    pub fn issue_date_label(_: Language) -> &'static str {
        "Issue date"
    }

    /// Due date field.
    pub fn due_date_label(_: Language) -> &'static str {
        "Due date"
    }

    /// Buyer reference field.
    pub fn buyer_reference_label(_: Language) -> &'static str {
        "Buyer ref"
    }

    /// Order reference field.
    pub fn order_reference_label(_: Language) -> &'static str {
        "Order ref"
    }

    /// Delivery date field.
    pub fn delivery_date_label(_: Language) -> &'static str {
        "Delivery date"
    }

    /// Language picker for the printed document.
    pub fn document_language_label(_: Language) -> &'static str {
        "Document language"
    }

    /// Free-text note field.
    pub fn note_label(_: Language) -> &'static str {
        "Note"
    }

    /// Line entry columns, in order.
    pub fn line_columns(_: Language) -> [&'static str; 5] {
        ["Item", "Qty", "Unit price", "Rate %", "VAT"]
    }

    /// Removes a line.
    pub fn remove_line_action(_: Language) -> &'static str {
        "Remove"
    }

    /// Appends a line.
    pub fn add_line_action(_: Language) -> &'static str {
        "Add line"
    }

    /// Stores the draft without issuing it.
    pub fn save_draft_action(_: Language) -> &'static str {
        "Save draft"
    }

    /// Issues the draft.
    pub fn issue_action(_: Language) -> &'static str {
        "Issue"
    }

    /// Live totals under the lines.
    pub fn totals(language: Language, net: &str, tax: &str, gross: &str) -> String {
        match language {
            Language::English => format!("Net {net}   VAT {tax}   Total {gross}"),
        }
    }

    /// Title of the suppression dialog.
    pub fn suppress_title(_: Language) -> &'static str {
        "Suppress warning"
    }

    /// Which finding is being suppressed.
    pub fn suppress_target(language: Language, code: &str, field: &str) -> String {
        match language {
            Language::English => format!("{code} at {field}"),
        }
    }

    /// Label above the suppression reason.
    pub fn reason_label(_: Language) -> &'static str {
        "Reason"
    }

    /// Stores the suppression.
    pub fn record_suppression_action(_: Language) -> &'static str {
        "Record suppression"
    }

    /// Closes a dialog without acting.
    pub fn cancel_action(_: Language) -> &'static str {
        "Cancel"
    }

    /// Points the form at the field a finding names.
    pub fn focus_action(_: Language) -> &'static str {
        "Focus"
    }

    /// Records a reason for ignoring a warning.
    pub fn suppress_action(_: Language) -> &'static str {
        "Suppress"
    }

    /// The draft was stored.
    pub fn draft_saved(_: Language) -> &'static str {
        "Draft saved."
    }

    /// The seller record was stored.
    pub fn seller_created(_: Language) -> &'static str {
        "Seller created."
    }

    /// The customer record was stored.
    pub fn customer_created(_: Language) -> &'static str {
        "Customer created."
    }

    /// The editor has no customer yet.
    pub fn select_customer_required(_: Language) -> &'static str {
        "Select a customer."
    }

    /// The live totals cannot be calculated without a customer.
    pub fn select_customer_to_validate(_: Language) -> &'static str {
        "Select a customer to validate."
    }

    /// Suppressing a finding requires a seller.
    pub fn seller_required_to_suppress(_: Language) -> &'static str {
        "Create the seller before suppressing findings."
    }

    /// Suppressing a finding requires an open invoice.
    pub fn invoice_required_to_suppress(_: Language) -> &'static str {
        "Open an invoice before suppressing findings."
    }

    /// A suppression needs a reason.
    pub fn suppression_reason_required(_: Language) -> &'static str {
        "A suppression reason is required."
    }

    /// The suppression was recorded.
    pub fn warning_suppressed(_: Language) -> &'static str {
        "Warning suppressed with a recorded reason."
    }

    /// A credit note was started from an invoice.
    pub fn credit_note_started(language: Language, number: &str) -> String {
        match language {
            Language::English => format!("Credit note for {number}"),
        }
    }

    /// The number a credit note should correct was not found.
    pub fn invoice_not_found(language: Language, number: &str) -> String {
        match language {
            Language::English => format!("invoice {number} was not found"),
        }
    }

    /// A draft handed to `start_credit_note` had no preceding invoice.
    pub fn no_preceding_invoice(_: Language) -> &'static str {
        "the draft has no preceding invoice"
    }

    /// Counts the findings that stopped an issue.
    pub fn blocking_findings(language: Language, count: usize) -> String {
        match language {
            Language::English => format!("{count} blocking finding(s); nothing was issued."),
        }
    }

    /// Counts the conformance findings that stopped an issue.
    pub fn conformance_findings(language: Language, count: usize) -> String {
        match language {
            Language::English => format!("{count} conformance finding(s); nothing was issued."),
        }
    }

    /// The invoice was issued but no PDF could be made.
    pub fn issued_without_pdf(language: Language, error: &str) -> String {
        match language {
            Language::English => format!("Issued. PDF not rendered: {error}"),
        }
    }

    /// The PDF was made but could not be attached to the archive.
    pub fn issued_with_unattached_pdf(language: Language, error: &str) -> String {
        match language {
            Language::English => format!("Issued, but the PDF was not attached: {error}"),
        }
    }

    /// The PDF path field is empty.
    pub fn choose_pdf_path(_: Language) -> &'static str {
        "Choose a file name for the PDF."
    }

    /// The PDF was written.
    pub fn pdf_saved(language: Language, path: &str) -> String {
        match language {
            Language::English => format!("PDF saved to {path}"),
        }
    }

    /// The PDF could not be written.
    pub fn pdf_save_failed(language: Language, error: &str) -> String {
        match language {
            Language::English => format!("Could not save the PDF: {error}"),
        }
    }
}

/// The archive health panel.
pub mod health {
    use super::Language;

    /// Title of the archive health panel.
    pub fn panel_title(_: Language) -> &'static str {
        "Archive health"
    }

    /// The archive's facts could not be read.
    pub fn unreadable(_: Language) -> &'static str {
        "Health could not be read from this archive."
    }

    /// Schema version.
    pub fn schema(language: Language, version: i64) -> String {
        match language {
            Language::English => format!("schema {version}"),
        }
    }

    /// How large the archive is.
    pub fn size(language: Language, kibibytes: u64) -> String {
        match language {
            Language::English => format!("{kibibytes} KB"),
        }
    }

    /// The audit chain, when it has links.
    pub fn chain(language: Language, events: i64) -> String {
        match language {
            Language::English => format!("audit: {events} events, head"),
        }
    }

    /// An archive that has never recorded an event.
    pub fn empty_chain(_: Language) -> &'static str {
        "audit: empty chain"
    }

    /// The payload audit failed.
    pub fn integrity_failed(language: Language, error: &str) -> String {
        match language {
            Language::English => format!("integrity: {error}"),
        }
    }

    /// Every payload verified.
    pub fn integrity_ok(language: Language, payloads: usize) -> String {
        match language {
            Language::English => format!("integrity: {payloads} payloads verified"),
        }
    }

    /// Everything verified, but some blobs are unreferenced.
    pub fn integrity_with_orphans(language: Language, payloads: usize, orphans: usize) -> String {
        match language {
            Language::English => {
                format!("integrity: {payloads} payloads verified, {orphans} unreferenced (wasted space only)")
            }
        }
    }

    /// The audit has not been run.
    pub fn not_checked(_: Language) -> &'static str {
        "integrity: not checked"
    }

    /// When the audit last ran.
    pub fn checked(language: Language, at: &str) -> String {
        match language {
            Language::English => format!("checked {at}"),
        }
    }

    /// The newest backup beside the archive.
    pub fn last_backup(language: Language, name: &str, kibibytes: u64) -> String {
        match language {
            Language::English => format!("last backup: {name} ({kibibytes} KB)"),
        }
    }

    /// There is no backup yet.
    pub fn no_backup(_: Language) -> &'static str {
        "last backup: none — run bonk backup"
    }

    /// Runs the payload audit now.
    pub fn check_action(_: Language) -> &'static str {
        "Check integrity now"
    }
}

/// The security panel.
pub mod security {
    use super::Language;

    /// Title of the security panel.
    pub fn panel_title(_: Language) -> &'static str {
        "Security"
    }

    /// Why the current passphrase is asked for again.
    pub fn note(_: Language) -> &'static str {
        "A credential change asks for the current passphrase again."
    }

    /// The current passphrase.
    pub fn current_label(_: Language) -> &'static str {
        "Current passphrase"
    }

    /// Rewraps the data key.
    pub fn change_action(_: Language) -> &'static str {
        "Change passphrase"
    }

    /// Adds or replaces the recovery key.
    pub fn add_key_action(_: Language) -> &'static str {
        "Add recovery key"
    }

    /// Acknowledges a shown key.
    pub fn done_action(_: Language) -> &'static str {
        "Done"
    }

    /// The key was added.
    pub fn key_added(_: Language) -> &'static str {
        "Recovery key added."
    }

    /// The passphrase was changed.
    pub fn passphrase_changed(_: Language) -> &'static str {
        "Passphrase changed. Backups made before this change still need the old one."
    }

    /// The passphrase field is empty when a key is requested.
    pub fn enter_current(_: Language) -> &'static str {
        "Enter the current passphrase first."
    }

    /// Keeps the key in this device's credential store.
    pub fn remember_action(_: Language) -> &'static str {
        "Remember this device"
    }

    /// Stops this device from keeping the key.
    pub fn forget_action(_: Language) -> &'static str {
        "Forget this device"
    }

    /// The switch's label: what pressing it will do.
    ///
    /// The switch already shows the state by being filled or not; the words say
    /// which way it moves, so a remembered device offers to forget and a
    /// forgotten one offers to remember.
    pub fn device_action(remembered: bool, language: Language) -> &'static str {
        if remembered {
            forget_action(language)
        } else {
            remember_action(language)
        }
    }

    /// Tooltip when the switch is off.
    pub fn remember_hint(_: Language) -> &'static str {
        "Keep the archive key in this machine's credential store, so your login opens it instead of the passphrase."
    }

    /// Tooltip when the switch is on.
    pub fn forget_hint(_: Language) -> &'static str {
        "Remove the stored key; the passphrase opens this archive again."
    }

    /// What the switch means while it is on.
    pub fn device_on_note(_: Language) -> &'static str {
        "On — this archive opens with your login, not the passphrase."
    }

    /// What the switch means while it is off.
    pub fn device_off_note(_: Language) -> &'static str {
        "Off — the passphrase opens this archive."
    }

    /// The device now holds the key.
    pub fn device_remembered(_: Language) -> &'static str {
        "This device now opens the archive without the passphrase; your login protects it while it does."
    }

    /// The device no longer holds it.
    pub fn device_forgotten(_: Language) -> &'static str {
        "This device has forgotten the key; the passphrase is needed again."
    }

    /// There was nothing to forget.
    pub fn device_no_key(_: Language) -> &'static str {
        "This device held no key for the archive."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_device_switch_says_what_it_will_do() {
        let language = Language::default();
        assert_eq!(security::device_action(false, language), "Remember this device");
        assert_eq!(security::device_action(true, language), "Forget this device");
    }

    #[test]
    fn every_level_has_its_own_word() {
        let language = Language::default();
        let words = [
            strength::level(language, Strength::Weak),
            strength::level(language, Strength::Fair),
            strength::level(language, Strength::Strong),
        ];
        assert_eq!(words, ["weak", "fair", "strong"]);
        assert!(words.iter().all(|word| !word.is_empty()));
    }

    #[test]
    fn every_reason_says_something() {
        let language = Language::default();
        for reason in [
            Reason::Empty,
            Reason::Common,
            Reason::Repetition,
            Reason::TooShort,
            Reason::Fair,
            Reason::Strong,
        ] {
            let text = strength::reason(language, reason);
            assert!(!text.is_empty(), "{reason:?} has no sentence");
            assert!(!text.ends_with(' '), "{reason:?} ends with a stray space");
        }
    }

    #[test]
    fn shell_text_is_never_empty() {
        let language = Language::default();
        for text in [
            shell::tagline(language),
            shell::database_label(language),
            shell::keystore_label(language),
            shell::passphrase_label(language),
            shell::unlock_action(language),
            shell::create_database_action(language),
            shell::forgotten_action(language),
            shell::create_note(language),
            shell::new_passphrase_label(language),
            shell::repeat_passphrase_label(language),
            shell::create_action(language),
            shell::back_action(language),
            shell::reset_note(language),
            shell::recovery_key_label(language),
            shell::repeat_new_passphrase_label(language),
            shell::reset_action(language),
            shell::reset_done_note(language),
            shell::new_recovery_key_label(language),
            shell::keep_key_note(language),
            shell::written_down_action(language),
        ] {
            assert!(!text.is_empty());
        }
    }

    #[test]
    fn the_detail_mentions_the_number_it_was_given() {
        let detail = strength::detail(Language::English, 72);
        assert!(detail.contains("72"), "{detail}");
    }

    #[test]
    fn editor_text_covers_its_fields_and_columns() {
        let language = Language::default();
        for field in [
            editor::Field::LegalName,
            editor::Field::Name,
            editor::Field::Street,
            editor::Field::PostCode,
            editor::Field::City,
            editor::Field::Country,
            editor::Field::VatId,
            editor::Field::Kvk,
            editor::Field::Peppol,
        ] {
            assert!(!editor::field(language, field).is_empty(), "{field:?}");
        }
        assert_eq!(editor::line_columns(language).len(), 5);
        assert!(editor::blocking_findings(language, 2).contains('2'));
        assert!(editor::pdf_saved(language, "/tmp/x.pdf").contains("/tmp/x.pdf"));
        assert!(editor::credit_note_banner(language, "2026-0001").contains("2026-0001"));
    }

    #[test]
    fn panel_text_names_its_numbers() {
        let language = Language::default();
        assert!(health::schema(language, 3).contains('3'));
        assert!(health::chain(language, 7).contains('7'));
        assert!(health::integrity_with_orphans(language, 4, 1).contains('1'));
        assert!(health::last_backup(language, "2026-10.db", 12).contains("2026-10.db"));
        assert!(!security::note(language).is_empty());
        assert!(!security::passphrase_changed(language).is_empty());
        assert!(!health::panel_title(language).is_empty());
        assert!(!security::panel_title(language).is_empty());
    }

    #[test]
    fn find_text_names_what_it_describes() {
        let language = Language::default();
        assert_eq!(find::columns(language).len(), 6);
        assert_eq!(find::line_columns(language).len(), 6);

        assert_eq!(find::count_line(language, 3, 500), "3/500");

        let title = find::document_title(language, "2026-0001", true);
        assert!(title.contains("2026-0001"), "{title}");
        assert!(title.contains("credit note"), "{title}");

        let totals = find::totals(language, "100.00", "21.00", "121.00");
        assert!(totals.contains("121.00"), "{totals}");

        for fact in [
            find::Fact::Side,
            find::Fact::Status,
            find::Fact::Issued,
            find::Fact::Due,
            find::Fact::Delivered,
            find::Fact::BuyerReference,
            find::Fact::OrderReference,
            find::Fact::Seller,
            find::Fact::Buyer,
            find::Fact::Note,
            find::Fact::Recorded,
            find::Fact::SemanticHash,
        ] {
            assert!(!find::fact(language, fact).is_empty(), "{fact:?}");
        }
    }
}
