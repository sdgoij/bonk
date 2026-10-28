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
            DocumentStatus::Issued => "issued",
            DocumentStatus::Received => "received",
            DocumentStatus::Approved => "approved",
            DocumentStatus::Paid => "paid",
            DocumentStatus::Archived => "archived",
        }
    }

    /// Result columns, in order.
    pub fn columns(_: Language) -> [&'static str; 6] {
        ["Number", "Side", "Party", "Issued", "Gross", "Status"]
    }

    /// Under the results: how many, in what order, and how to open one.
    pub fn footer(language: Language, count: usize, limit: u32) -> String {
        match language {
            Language::English => {
                format!("{count} document(s), best match or newest first, at most {limit}. Click a number to read it.")
            }
        }
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

    /// Nothing has been started yet.
    pub fn empty_editor(_: Language) -> &'static str {
        "Open a new invoice to start. Existing drafts are not listed yet."
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

        let footer = find::footer(language, 3, 500);
        assert!(footer.contains('3') && footer.contains("500"), "{footer}");

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
