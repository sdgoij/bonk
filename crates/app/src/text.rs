//! User-facing interface text.
//!
//! English first. Every string the shell shows lives here rather than in the
//! widget call, so adding a language means a second table and a lookup, not a
//! hunt through the drawing code. Text the user wrote — invoice descriptions,
//! notes, customer names — is theirs and never comes from here.
//!
//! The `_` parameter on each function is the language it will be selected by;
//! the call sites do not change when a table grows. The Dutch column exists for
//! the shell, the find screen, customers, and settings; the invoice editor and
//! the security panels still answer in English, and the interface does not offer
//! the choice yet, which is what keeps a half-translated window impossible.

use crate::strength::{Reason, Strength};

/// Languages the interface can speak.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Language {
    /// The table the code was written in.
    #[default]
    English,
    /// Dutch.
    Dutch,
}

/// Both tables for one sentence.
///
/// The unused arm is not evaluated, so a `format!` there costs nothing.
macro_rules! sentence {
    ($language:expr, $english:expr, $dutch:expr) => {
        match $language {
            $crate::text::Language::English => $english,
            $crate::text::Language::Dutch => $dutch,
        }
    };
}

/// The storage key the window writes the interface language under.
pub const STORAGE_KEY: &str = "interface-language";

impl Language {
    /// The name written to the window's storage.
    pub const fn name(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Dutch => "nl",
        }
    }

    /// Reads a stored name back, if this build knows it.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "en" => Some(Self::English),
            "nl" => Some(Self::Dutch),
            _ => None,
        }
    }
}

/// The lock screen.
pub mod shell {
    use super::Language;

    /// The product's working name. A name does not translate.
    pub const PRODUCT: &str = "Bonk";

    /// The line under the name, before anything is unlocked.
    pub fn tagline(language: Language) -> &'static str {
        sentence!(
            language,
            "Local-first invoicing. Nothing leaves this machine unless you send it.",
            "Factureren op je eigen machine. Niets verlaat deze computer tenzij je het verstuurt."
        )
    }

    /// Label above the database path.
    pub fn database_label(language: Language) -> &'static str {
        sentence!(language, "Database", "Database")
    }

    /// Label above the keystore path.
    pub fn keystore_label(language: Language) -> &'static str {
        sentence!(language, "Keystore", "Sleutelkluis")
    }

    /// Opens the file picker for a path field.
    pub fn browse_action(language: Language) -> &'static str {
        sentence!(language, "Choose…", "Kiezen…")
    }

    /// Label above the passphrase field.
    pub fn passphrase_label(language: Language) -> &'static str {
        sentence!(language, "Passphrase", "Wachtwoordzin")
    }

    /// Opens an existing archive.
    pub fn unlock_action(language: Language) -> &'static str {
        sentence!(language, "Unlock", "Ontgrendelen")
    }

    /// Starts creating one.
    pub fn create_database_action(language: Language) -> &'static str {
        sentence!(language, "Create new database", "Nieuwe database maken")
    }

    /// Switches to the recovery route.
    pub fn forgotten_action(language: Language) -> &'static str {
        sentence!(language, "Forgotten the passphrase?", "Wachtwoordzin vergeten?")
    }

    /// Opens the archive with the key this device kept.
    pub fn unlock_with_device(language: Language) -> &'static str {
        sentence!(language, "Unlock with this device", "Ontgrendelen met dit apparaat")
    }

    /// Explains what creating an archive does.
    pub fn create_note(language: Language) -> &'static str {
        sentence!(
            language,
            "A new archive is created empty and sealed with this passphrase. Nothing else can open it, so type it twice.",
            "Een nieuw archief wordt leeg aangemaakt en met deze wachtwoordzin verzegeld. Niets anders kan het openen, dus typ hem twee keer."
        )
    }

    /// Label above the first field when creating.
    pub fn new_passphrase_label(language: Language) -> &'static str {
        sentence!(language, "New passphrase", "Nieuwe wachtwoordzin")
    }

    /// Label above the second field when creating.
    pub fn repeat_passphrase_label(language: Language) -> &'static str {
        sentence!(language, "Repeat the passphrase", "Herhaal de wachtwoordzin")
    }

    /// Confirms a creation.
    pub fn create_action(language: Language) -> &'static str {
        sentence!(language, "Create", "Aanmaken")
    }

    /// Leaves the current form.
    pub fn back_action(language: Language) -> &'static str {
        sentence!(language, "Back", "Terug")
    }

    /// Explains the recovery route.
    pub fn reset_note(language: Language) -> &'static str {
        sentence!(
            language,
            "A recovery key opens this archive without the passphrase. The key used here is replaced by the reset, because it has now been typed somewhere.",
            "Een herstelsleutel opent dit archief zonder de wachtwoordzin. De sleutel die je hier gebruikt, wordt door het herstellen vervangen, omdat hij nu ergens is ingetypt."
        )
    }

    /// Label above the recovery-key field.
    pub fn recovery_key_label(language: Language) -> &'static str {
        sentence!(language, "Recovery key", "Herstelsleutel")
    }

    /// Label above the second field when resetting.
    pub fn repeat_new_passphrase_label(language: Language) -> &'static str {
        sentence!(language, "Repeat the new passphrase", "Herhaal de nieuwe wachtwoordzin")
    }

    /// Performs the reset.
    pub fn reset_action(language: Language) -> &'static str {
        sentence!(language, "Reset passphrase", "Wachtwoordzin herstellen")
    }

    /// Tells the user the reset happened and the old key is spent.
    pub fn reset_done_note(language: Language) -> &'static str {
        sentence!(
            language,
            "Passphrase replaced. The recovery key that was used no longer works.",
            "Wachtwoordzin vervangen. De gebruikte herstelsleutel werkt niet meer."
        )
    }

    /// Introduces the replacement key.
    pub fn new_recovery_key_label(language: Language) -> &'static str {
        sentence!(
            language,
            "New recovery key, shown once:",
            "Nieuwe herstelsleutel, eenmalig te zien:"
        )
    }

    /// Tells the user what to do with a key.
    pub fn keep_key_note(language: Language) -> &'static str {
        sentence!(
            language,
            "Write it down and keep it apart from the database and the passphrase.",
            "Schrijf hem op en bewaar hem los van de database en de wachtwoordzin."
        )
    }

    /// Acknowledges the replacement key.
    pub fn written_down_action(language: Language) -> &'static str {
        sentence!(language, "I have written it down", "Ik heb hem opgeschreven")
    }
}

/// The strength estimate shown under a passphrase field.
pub mod strength {
    use super::{Language, Reason, Strength};

    /// Label before the level itself.
    pub fn label(language: Language) -> &'static str {
        sentence!(language, "Strength", "Sterkte")
    }

    /// The level, as one word.
    pub fn level(language: Language, strength: Strength) -> &'static str {
        match strength {
            Strength::Weak => sentence!(language, "weak", "zwak"),
            Strength::Fair => sentence!(language, "fair", "redelijk"),
            Strength::Strong => sentence!(language, "strong", "sterk"),
        }
    }

    /// One sentence of advice for the level.
    pub fn reason(language: Language, reason: Reason) -> &'static str {
        match reason {
            Reason::Empty => sentence!(language, "enter a passphrase", "vul een wachtwoordzin in"),
            Reason::Common => sentence!(
                language,
                "this is one of the first phrases an attacker tries",
                "dit is een van de eerste zinnen die een aanvaller probeert"
            ),
            Reason::Repetition => sentence!(
                language,
                "repeating a short pattern adds almost nothing",
                "een kort patroon herhalen voegt bijna niets toe"
            ),
            Reason::TooShort => sentence!(
                language,
                "too short; a few unrelated words beat a short scramble",
                "te kort; een paar losse woorden verslaan een korte warboel"
            ),
            Reason::Fair => sentence!(
                language,
                "acceptable; length helps more than clever substitutions",
                "acceptabel; lengte helpt meer dan slimme vervangingen"
            ),
            Reason::Strong => sentence!(
                language,
                "good: length and variety both count",
                "goed: lengte en variatie tellen allebei"
            ),
        }
    }

    /// The hover text, which explains what the number means.
    pub fn detail(language: Language, bits: u32) -> String {
        sentence!(
            language,
            format!(
                "At least {bits} bits by this estimate. Every guess costs the attacker one Argon2id run, so \
                 length is what makes a copied keystore expensive to open."
            ),
            format!(
                "Naar schatting minstens {bits} bits. Elke gok kost de aanvaller één Argon2id-ronde, dus \
                 lengte maakt een gekopieerde sleutelkluis duur om te openen."
            )
        )
    }
}

/// The workspace chrome around the screens.
pub mod workspace {
    use super::Language;

    /// Leaves the archive open and returns to the lock screen.
    pub fn lock_action(language: Language) -> &'static str {
        sentence!(language, "Lock", "Vergrendelen")
    }

    /// The line naming the open archive.
    pub fn database_line(language: Language, path: &str) -> String {
        sentence!(language, format!("Database: {path}"), format!("Database: {path}"))
    }

    /// Shown when the build stores the archive unencrypted.
    pub fn unencrypted_warning(language: Language) -> &'static str {
        sentence!(
            language,
            "UNENCRYPTED BUILD - test only, never store real data",
            "ONGECODEERDE BUILD - alleen voor tests, bewaar hier nooit echte gegevens"
        )
    }

    /// Hides the last message.
    pub fn dismiss_action(language: Language) -> &'static str {
        sentence!(language, "Dismiss", "Sluiten")
    }
}

/// The navigation rail. The rail shows icons; these are the names they carry.
pub mod tabs {
    use super::Language;

    /// Find documents.
    pub fn search(language: Language) -> &'static str {
        sentence!(language, "Search", "Zoeken")
    }

    /// The party list.
    pub fn customers(language: Language) -> &'static str {
        sentence!(language, "Customers", "Klanten")
    }

    /// Archive settings.
    pub fn settings(language: Language) -> &'static str {
        sentence!(language, "Settings", "Instellingen")
    }
}

/// The find screen and the document it opens.
pub mod find {
    use super::Language;
    use store::{Direction, DocumentStatus};

    /// Panel heading.
    pub fn title(language: Language) -> &'static str {
        sentence!(language, "Find documents", "Documenten zoeken")
    }

    /// The keyboard hint beside the heading.
    pub fn shortcut(_: Language) -> &'static str {
        "Ctrl+F"
    }

    /// Hint in the search field.
    pub fn hint(language: Language) -> &'static str {
        sentence!(
            language,
            "Search number, party, reference, or note",
            "Zoek op nummer, partij, referentie of notitie"
        )
    }

    /// Runs the query.
    pub fn search_action(language: Language) -> &'static str {
        sentence!(language, "Search", "Zoeken")
    }

    /// Resets the panel.
    pub fn clear_action(language: Language) -> &'static str {
        sentence!(language, "Clear", "Wissen")
    }

    /// Label before the saved-filter list.
    pub fn saved_label(language: Language) -> &'static str {
        sentence!(language, "Saved", "Opgeslagen")
    }

    /// Shown in the saved list when nothing is applied.
    pub fn no_saved(language: Language) -> &'static str {
        sentence!(language, "none", "geen")
    }

    /// Hint in the field that names a saved search.
    pub fn save_name_hint(language: Language) -> &'static str {
        sentence!(language, "name", "naam")
    }

    /// Stores the current filters under the typed name.
    pub fn save_current_action(language: Language) -> &'static str {
        sentence!(language, "Save current", "Huidige opslaan")
    }

    /// Removes the applied saved search.
    pub fn delete_action(language: Language) -> &'static str {
        sentence!(language, "Delete", "Verwijderen")
    }

    /// Nothing to list, before any query.
    pub fn empty_archive(language: Language) -> &'static str {
        sentence!(
            language,
            "No documents in this archive yet.",
            "Nog geen documenten in dit archief."
        )
    }

    /// A query that found nothing.
    pub fn no_matches(language: Language) -> &'static str {
        sentence!(
            language,
            "No documents match. Words are combined, so every word has to appear.",
            "Geen documenten gevonden. Woorden worden gecombineerd, dus elk woord moet voorkomen."
        )
    }

    /// What the panel is for, before it is used.
    pub fn prompt(language: Language) -> &'static str {
        sentence!(
            language,
            "Search by number, party, reference, or note. Leave it blank to list everything.",
            "Zoek op nummer, partij, referentie of notitie. Laat leeg om alles te tonen."
        )
    }

    /// No side filter.
    pub fn any_side(language: Language) -> &'static str {
        sentence!(language, "any side", "elke richting")
    }

    /// No status filter.
    pub fn any_status(language: Language) -> &'static str {
        sentence!(language, "any status", "elke status")
    }

    /// A side of the books.
    pub fn direction(language: Language, direction: Direction) -> &'static str {
        match direction {
            Direction::Outbound => sentence!(language, "issued", "verstuurd"),
            Direction::Inbound => sentence!(language, "received", "ontvangen"),
        }
    }

    /// Where a document stands.
    pub fn status(language: Language, status: DocumentStatus) -> &'static str {
        match status {
            DocumentStatus::Draft => sentence!(language, "draft", "concept"),
            DocumentStatus::Issued => sentence!(language, "issued", "verstuurd"),
            DocumentStatus::Received => sentence!(language, "received", "ontvangen"),
            DocumentStatus::Approved => sentence!(language, "approved", "goedgekeurd"),
            DocumentStatus::Paid => sentence!(language, "paid", "betaald"),
            DocumentStatus::Archived => sentence!(language, "archived", "gearchiveerd"),
        }
    }

    /// What an empty cell shows.
    pub fn no_value(language: Language) -> &'static str {
        sentence!(language, "—", "—")
    }

    /// Result columns, in order.
    pub fn columns(language: Language) -> [&'static str; 6] {
        match language {
            Language::English => ["Number", "Side", "Party", "Issued", "Gross", "Status"],
            Language::Dutch => ["Nummer", "Richting", "Partij", "Verstuurd", "Bruto", "Status"],
        }
    }

    /// The pinned footer: how many the filters match, and how many the archive
    /// holds. `42/1337` is easier to read than a sentence, and the second number
    /// says what a wider search would find.
    pub fn count_line(_: Language, matched: u64, total: u64) -> String {
        format!("{matched}/{total}")
    }

    /// The heading of an opened document.
    pub fn document_title(language: Language, number: &str, credit_note: bool) -> String {
        let kind = match credit_note {
            false => sentence!(language, "invoice", "factuur"),
            true => sentence!(language, "credit note", "creditnota"),
        };
        format!("{number} · {kind}")
    }

    /// Says the opened document cannot be edited.
    pub fn stored_note(language: Language) -> &'static str {
        sentence!(
            language,
            "Stored as it was issued. Nothing here can be edited; a correction is a credit note.",
            "Opgeslagen zoals het is verstuurd. Hier kan niets worden gewijzigd; een correctie is een creditnota."
        )
    }

    /// Closes the opened document.
    pub fn close_action(language: Language) -> &'static str {
        sentence!(language, "Close", "Sluiten")
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
    pub fn fact(language: Language, fact: Fact) -> &'static str {
        match fact {
            Fact::Side => sentence!(language, "Side", "Richting"),
            Fact::Status => sentence!(language, "Status", "Status"),
            Fact::Issued => sentence!(language, "Issued", "Factuurdatum"),
            Fact::Due => sentence!(language, "Due", "Vervaldatum"),
            Fact::Delivered => sentence!(language, "Delivered", "Leverdatum"),
            Fact::BuyerReference => sentence!(language, "Buyer reference", "Referentie afnemer"),
            Fact::OrderReference => sentence!(language, "Order reference", "Orderreferentie"),
            Fact::Seller => sentence!(language, "Seller", "Verkoper"),
            Fact::Buyer => sentence!(language, "Buyer", "Koper"),
            Fact::Note => sentence!(language, "Note", "Notitie"),
            Fact::Recorded => sentence!(language, "Recorded", "Vastgelegd"),
            Fact::SemanticHash => sentence!(language, "Semantic SHA-256", "Semantische SHA-256"),
        }
    }

    /// Line columns, in order.
    pub fn line_columns(language: Language) -> [&'static str; 6] {
        match language {
            Language::English => ["#", "Item", "Qty", "Unit price", "VAT", "Net"],
            Language::Dutch => ["#", "Omschrijving", "Aantal", "Prijs", "Btw", "Netto"],
        }
    }

    /// The totals row.
    pub fn totals(language: Language, net: &str, tax: &str, gross: &str) -> String {
        sentence!(
            language,
            format!("Net {net} · VAT {tax} · Payable {gross}"),
            format!("Netto {net} · Btw {tax} · Te betalen {gross}")
        )
    }

    /// Heading over the status events, when there are any.
    pub fn history_label(language: Language) -> &'static str {
        sentence!(language, "History", "Geschiedenis")
    }

    /// Heading over the artefact hashes, when there are any.
    pub fn artefacts_label(language: Language) -> &'static str {
        sentence!(language, "Artefacts", "Bestanden")
    }

    /// One artefact: kind, size, hash, and the layout that drew it when one did.
    ///
    /// The digest is passed already shortened: the window shows enough of it to
    /// tell two layouts apart, and the full one lives in the record.
    pub fn artefact_line(language: Language, kind: &str, size: i64, hash: &str, template: Option<&str>) -> String {
        match template {
            Some(template) => sentence!(
                language,
                format!("{kind} · {size} bytes · {hash} · template {template}"),
                format!("{kind} · {size} bytes · {hash} · sjabloon {template}")
            ),
            None => sentence!(
                language,
                format!("{kind} · {size} bytes · {hash}"),
                format!("{kind} · {size} bytes · {hash}")
            ),
        }
    }

    /// Writes the filed PDF, after the platform's save panel asks where.
    pub fn save_pdf_action(language: Language) -> &'static str {
        sentence!(language, "Save PDF…", "PDF opslaan…")
    }

    /// Writes the filed UBL, after the platform's save panel asks where.
    pub fn save_ubl_action(language: Language) -> &'static str {
        sentence!(language, "Save UBL…", "UBL opslaan…")
    }

    /// What was written, and where.
    pub fn export_saved(language: Language, path: &str) -> String {
        sentence!(language, format!("Saved {path}"), format!("Opgeslagen: {path}"))
    }

    /// Writing failed.
    pub fn export_failed(language: Language, error: &str) -> String {
        sentence!(
            language,
            format!("Could not save the export: {error}"),
            format!("De export kon niet worden opgeslagen: {error}")
        )
    }

    /// Footer action that opens a file and records the invoice it holds.
    pub fn import_action(language: Language) -> &'static str {
        sentence!(language, "Import received…", "Ontvangen importeren…")
    }

    /// What an import did.
    pub fn imported(language: Language, number: &str, supplier: Option<&str>, duplicate: bool) -> String {
        if duplicate {
            return sentence!(
                language,
                format!("{number} is already in the archive."),
                format!("{number} staat al in het archief.")
            );
        }
        match supplier {
            Some(name) => sentence!(
                language,
                format!("Imported {number} from {name}."),
                format!("{number} geïmporteerd van {name}.")
            ),
            None => sentence!(
                language,
                format!("Imported {number}; the supplier is not matched yet."),
                format!("{number} geïmporteerd; de leverancier is nog niet gekoppeld.")
            ),
        }
    }

    /// The import failed.
    pub fn import_failed(language: Language, error: &str) -> String {
        sentence!(
            language,
            format!("Could not import that file: {error}"),
            format!("Dat bestand kon niet worden geïmporteerd: {error}")
        )
    }

    /// Label for the supplier row of a received document.
    pub fn supplier_label(language: Language) -> &'static str {
        sentence!(language, "Supplier", "Leverancier")
    }

    /// Shown when no party matched the issuer.
    pub fn unmatched(language: Language) -> &'static str {
        sentence!(language, "not matched", "niet gekoppeld")
    }

    /// Records the chosen supplier.
    pub fn match_action(language: Language) -> &'static str {
        sentence!(language, "Match supplier", "Leverancier koppelen")
    }

    /// Moves a received document to approved.
    pub fn approve_action(language: Language) -> &'static str {
        sentence!(language, "Approve", "Goedkeuren")
    }

    /// Moves an approved document to paid.
    pub fn mark_paid_action(language: Language) -> &'static str {
        sentence!(language, "Mark paid", "Markeer als betaald")
    }

    /// What a status change did.
    pub fn settled(language: Language, target: DocumentStatus) -> String {
        sentence!(
            language,
            format!("Marked as {}.", status(language, target)),
            format!("Gemarkeerd als {}.", status(language, target))
        )
    }

    /// The supplier match was recorded.
    pub fn matched(language: Language, name: &str) -> String {
        sentence!(
            language,
            format!("Matched to {name}."),
            format!("Gekoppeld aan {name}.")
        )
    }
}

/// The customer tab.
pub mod customers {
    use super::Language;

    /// The tab heading.
    pub fn title(language: Language) -> &'static str {
        sentence!(language, "Customers", "Klanten")
    }

    /// Hint in the filter field.
    pub fn filter_hint(language: Language) -> &'static str {
        sentence!(
            language,
            "Filter by name, city, or VAT",
            "Filter op naam, plaats of btw"
        )
    }

    /// Opens the new-customer form.
    pub fn new_customer_action(language: Language) -> &'static str {
        sentence!(language, "New customer", "Nieuwe klant")
    }

    /// Heading of the inline detail editor.
    pub fn edit_heading(language: Language) -> &'static str {
        sentence!(language, "Edit customer", "Klant bewerken")
    }

    /// A saved edit.
    pub fn updated(language: Language, name: &str) -> String {
        sentence!(language, format!("Saved {name}."), format!("{name} opgeslagen."))
    }

    /// Closes the new-customer form.
    pub fn cancel_action(language: Language) -> &'static str {
        sentence!(language, "Cancel", "Annuleren")
    }

    /// Table headings, in order.
    pub fn columns(language: Language) -> [&'static str; 5] {
        match language {
            Language::English => ["Name", "City", "Country", "VAT", "Peppol"],
            Language::Dutch => ["Naam", "Plaats", "Land", "Btw", "Peppol"],
        }
    }

    /// Nothing on file.
    pub fn empty(language: Language) -> &'static str {
        sentence!(
            language,
            "No customers yet. Add one below, or import a CSV.",
            "Nog geen klanten. Voeg er hieronder een toe of importeer een CSV."
        )
    }

    /// The filter found nothing.
    pub fn no_matches(language: Language) -> &'static str {
        sentence!(
            language,
            "No customers match that filter.",
            "Geen klanten die bij dat filter passen."
        )
    }

    /// A successful export.
    /// Opens a CSV of customers to read.
    pub fn import_action(language: Language) -> &'static str {
        sentence!(language, "Import…", "Importeren…")
    }

    /// Opens a save panel for the customer CSV.
    pub fn export_action(language: Language) -> &'static str {
        sentence!(language, "Export…", "Exporteren…")
    }

    /// What the file holds.
    pub fn csv_note(language: Language) -> &'static str {
        sentence!(
            language,
            "Columns: name, street, postal, city, country, vat, peppol, contact, email.",
            "Kolommen: naam, straat, postcode, plaats, land, btw, peppol, contact, e-mail."
        )
    }

    /// A successful export.
    pub fn exported(language: Language, count: usize, path: &str) -> String {
        sentence!(
            language,
            format!("Exported {count} customer(s) to {path}."),
            format!("{count} klant(en) geëxporteerd naar {path}.")
        )
    }

    /// A successful import.
    pub fn imported(language: Language, imported: usize, skipped: usize, errors: usize) -> String {
        let mut text = sentence!(
            language,
            format!("Imported {imported} customer(s); {skipped} already on file"),
            format!("{imported} klant(en) geïmporteerd; {skipped} al bekend")
        );
        if errors > 0 {
            text.push_str(&sentence!(
                language,
                format!(", {errors} row(s) skipped with an error"),
                format!(", {errors} regel(s) overgeslagen met een fout")
            ));
        }
        text.push('.');
        text
    }

    /// Why a file operation failed.
    pub fn failed(language: Language, error: &str) -> String {
        sentence!(
            language,
            format!("The customer file could not be read: {error}"),
            format!("Het klantenbestand kon niet worden gelezen: {error}")
        )
    }
}

/// The settings tab.
pub mod settings {
    use super::Language;

    /// The tab heading.
    pub fn title(language: Language) -> &'static str {
        sentence!(language, "Settings", "Instellingen")
    }

    /// The appearance card.
    pub fn appearance(language: Language) -> &'static str {
        sentence!(language, "Appearance", "Weergave")
    }

    /// Follow the operating system.
    pub fn theme_system(language: Language) -> &'static str {
        sentence!(language, "System", "Systeem")
    }

    /// The light theme.
    pub fn theme_light(language: Language) -> &'static str {
        sentence!(language, "Light", "Licht")
    }

    /// The dark theme.
    pub fn theme_dark(language: Language) -> &'static str {
        sentence!(language, "Dark", "Donker")
    }

    /// What the choice means.
    pub fn theme_note(language: Language) -> &'static str {
        sentence!(
            language,
            "Follows the operating system unless you choose otherwise.",
            "Volgt het besturingssysteem tenzij je anders kiest."
        )
    }

    /// The archive card.
    pub fn archive(language: Language) -> &'static str {
        sentence!(language, "Archive", "Archief")
    }

    /// The database path.
    pub fn database(language: Language) -> &'static str {
        sentence!(language, "Database", "Database")
    }

    /// The encryption label.
    pub fn encryption(language: Language) -> &'static str {
        sentence!(language, "Encryption", "Versleuteling")
    }

    /// The health card.
    pub fn health(language: Language) -> &'static str {
        sentence!(language, "Archive health", "Gezondheid van het archief")
    }

    /// The backup card.
    pub fn backup(language: Language) -> &'static str {
        sentence!(language, "Backup", "Back-up")
    }

    /// What a backup is.
    pub fn backup_note(language: Language) -> &'static str {
        sentence!(
            language,
            "A verified, encrypted copy kept beside the archive.",
            "Een gecontroleerde, versleutelde kopie naast het archief."
        )
    }

    /// Runs a backup now.
    pub fn backup_now(language: Language) -> &'static str {
        sentence!(language, "Back up now", "Nu een back-up maken")
    }

    /// Where backups go.
    pub fn backup_directory(language: Language, path: &str) -> String {
        sentence!(language, format!("Backups: {path}"), format!("Back-ups: {path}"))
    }

    /// A successful backup.
    pub fn backup_done(language: Language, path: &str, kilobytes: u64) -> String {
        sentence!(
            language,
            format!("Backed up to {path} ({kilobytes} KB), verified."),
            format!("Back-up gemaakt naar {path} ({kilobytes} KB), gecontroleerd.")
        )
    }

    /// Why a backup failed.
    pub fn backup_failed(language: Language, error: &str) -> String {
        sentence!(
            language,
            format!("The backup could not be written: {error}"),
            format!("De back-up kon niet worden geschreven: {error}")
        )
    }

    /// The watcher card.
    pub fn watching(language: Language) -> &'static str {
        sentence!(language, "Watching a folder", "Een map volgen")
    }

    /// What watching a folder does, and what it deliberately does not.
    pub fn watching_note(language: Language) -> &'static str {
        sentence!(
            language,
            "Invoices dropped into the folder are imported once they stop changing. Files already there when watching starts are left alone.",
            "Facturen in de map worden geïmporteerd zodra ze niet meer veranderen. Bestanden die er al stonden bij het starten blijven staan."
        )
    }

    /// The folder being watched.
    pub fn watching_folder(language: Language) -> &'static str {
        sentence!(language, "Folder", "Map")
    }

    /// No folder has been chosen yet.
    pub fn watching_none(language: Language) -> &'static str {
        sentence!(language, "none chosen", "geen gekozen")
    }

    /// Opens the platform's folder picker.
    pub fn watching_choose(language: Language) -> &'static str {
        sentence!(language, "Choose folder…", "Map kiezen…")
    }

    /// Arms the watcher.
    pub fn watching_start(language: Language) -> &'static str {
        sentence!(language, "Start watching", "Volgen starten")
    }

    /// Disarms the watcher.
    pub fn watching_stop(language: Language) -> &'static str {
        sentence!(language, "Stop watching", "Volgen stoppen")
    }

    /// What a running watcher has done.
    pub fn watching_running(language: Language, imported: usize, duplicates: usize, failed: usize) -> String {
        sentence!(
            language,
            format!("watching · {imported} imported, {duplicates} already there, {failed} unreadable"),
            format!("volgt · {imported} geïmporteerd, {duplicates} al aanwezig, {failed} onleesbaar")
        )
    }

    /// A dropped file that is not an invoice.
    pub fn watching_skipped(language: Language, file: &str, error: &str) -> String {
        sentence!(
            language,
            format!("{file} was left alone: {error}"),
            format!("{file} is blijven staan: {error}")
        )
    }

    /// Why the folder could not be read; watching stops.
    pub fn watching_failed(language: Language, error: &str) -> String {
        sentence!(
            language,
            format!("Watching stopped: {error}"),
            format!("Volgen gestopt: {error}")
        )
    }

    /// The credentials card.
    pub fn credentials(language: Language) -> &'static str {
        sentence!(language, "Credentials", "Inloggegevens")
    }

    /// The interface language.
    pub fn language(language: Language) -> &'static str {
        sentence!(language, "Language", "Taal")
    }

    /// The layout used to draw documents.
    pub fn layout(language: Language) -> &'static str {
        sentence!(language, "Template", "Sjabloon")
    }

    /// No template in the archive, so the built-in one draws.
    pub fn builtin_layout(language: Language) -> &'static str {
        sentence!(language, "built in", "ingebouwd")
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
    pub fn field(language: Language, field: Field) -> &'static str {
        match field {
            Field::LegalName => sentence!(language, "Legal name", "Juridische naam"),
            Field::Name => sentence!(language, "Name", "Naam"),
            Field::Street => sentence!(language, "Street", "Straat"),
            Field::PostCode => sentence!(language, "Post code", "Postcode"),
            Field::City => sentence!(language, "City", "Plaats"),
            Field::Country => sentence!(language, "Country", "Land"),
            Field::VatId => sentence!(language, "VAT ID", "Btw-nummer"),
            Field::Kvk => sentence!(language, "KvK number", "KvK-nummer"),
            Field::Peppol => sentence!(language, "Peppol ID", "Peppol-id"),
            Field::Contact => sentence!(language, "Contact", "Contactpersoon"),
            Field::Email => sentence!(language, "Email", "E-mail"),
            Field::Phone => sentence!(language, "Phone", "Telefoon"),
        }
    }

    /// Heading of the first-run seller form.
    pub fn seller_heading(language: Language) -> &'static str {
        sentence!(language, "Create your seller profile", "Maak je verkopersprofiel")
    }

    /// What the first-run form promises.
    pub fn seller_note(language: Language) -> &'static str {
        sentence!(
            language,
            "One legal entity per database in v1.",
            "Eén rechtspersoon per database in v1."
        )
    }

    /// Stores the seller.
    pub fn create_seller_action(language: Language) -> &'static str {
        sentence!(language, "Create seller", "Verkoper aanmaken")
    }

    /// Heading of the customer form.
    pub fn new_customer_heading(language: Language) -> &'static str {
        sentence!(language, "New customer", "Nieuwe klant")
    }

    /// Stores the customer.
    pub fn save_customer_action(language: Language) -> &'static str {
        sentence!(language, "Save customer", "Klant opslaan")
    }

    /// Opens the customer form.
    pub fn add_customer_action(language: Language) -> &'static str {
        sentence!(language, "Add customer", "Klant toevoegen")
    }

    /// The draft the user asked for is gone.
    pub fn draft_missing(language: Language) -> &'static str {
        sentence!(
            language,
            "That draft is no longer in this archive; run the search again.",
            "Dat concept staat niet meer in dit archief; voer de zoekopdracht opnieuw uit."
        )
    }

    /// The editor needs a party to bill.
    pub fn no_customers(language: Language) -> &'static str {
        sentence!(
            language,
            "Add a customer before starting an invoice.",
            "Voeg een klant toe voordat je een factuur begint."
        )
    }

    /// Tooltip on the calendar button.
    pub fn choose_date(language: Language) -> &'static str {
        sentence!(language, "Choose a date", "Kies een datum")
    }

    /// Tooltip on the button that empties an optional date.
    pub fn clear_date(language: Language) -> &'static str {
        sentence!(language, "Clear the date", "Datum wissen")
    }

    /// Weekday headings in the calendar, Monday first.
    pub fn weekdays(language: Language) -> [&'static str; 7] {
        match language {
            Language::English => ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"],
            Language::Dutch => ["ma", "di", "wo", "do", "vr", "za", "zo"],
        }
    }

    /// A month's name, for the calendar heading.
    pub fn month_name(language: Language, month: time::Month) -> &'static str {
        match month {
            time::Month::January => sentence!(language, "January", "januari"),
            time::Month::February => sentence!(language, "February", "februari"),
            time::Month::March => sentence!(language, "March", "maart"),
            time::Month::April => sentence!(language, "April", "april"),
            time::Month::May => sentence!(language, "May", "mei"),
            time::Month::June => sentence!(language, "June", "juni"),
            time::Month::July => sentence!(language, "July", "juli"),
            time::Month::August => sentence!(language, "August", "augustus"),
            time::Month::September => sentence!(language, "September", "september"),
            time::Month::October => sentence!(language, "October", "oktober"),
            time::Month::November => sentence!(language, "November", "november"),
            time::Month::December => sentence!(language, "December", "december"),
        }
    }

    /// Switches to the customer tab.
    pub fn go_to_customers(language: Language) -> &'static str {
        sentence!(language, "Go to customers", "Naar klanten")
    }

    /// The name of the seller the workspace belongs to.
    pub fn seller_line(language: Language, name: &str) -> String {
        sentence!(language, format!("Seller: {name}"), format!("Verkoper: {name}"))
    }

    /// Heading after issuing.
    pub fn issued_heading(language: Language, credit_note: bool) -> &'static str {
        match credit_note {
            false => sentence!(language, "Issued", "Verstuurd"),
            true => sentence!(language, "Issued credit note", "Creditnota verstuurd"),
        }
    }

    /// Heading over a composer that reopened a stored draft.
    pub fn draft_heading(language: Language) -> &'static str {
        sentence!(language, "Draft", "Concept")
    }

    /// Toggles the rendered preview.
    pub fn preview_action(language: Language) -> &'static str {
        sentence!(language, "Preview", "Voorbeeld")
    }

    /// Title over the question asked when a row click would lose work.
    pub fn unsaved_title(language: Language) -> &'static str {
        sentence!(language, "Unsaved invoice", "Niet-opgeslagen factuur")
    }

    /// What that question asks.
    pub fn unsaved_note(language: Language) -> &'static str {
        sentence!(
            language,
            "This invoice has not been saved yet. Save it as a draft, or discard it?",
            "Deze factuur is nog niet opgeslagen. Als concept opslaan of weggooien?"
        )
    }

    /// Throws the work away and does what was clicked.
    pub fn discard_action(language: Language) -> &'static str {
        sentence!(language, "Discard", "Weggooien")
    }

    /// Stays on the invoice.
    pub fn keep_editing_action(language: Language) -> &'static str {
        sentence!(language, "Keep editing", "Doorgaan met bewerken")
    }

    /// Heading over the rendered preview.
    pub fn preview_heading(language: Language) -> &'static str {
        sentence!(language, "Preview", "Voorbeeld")
    }

    /// The number field of a preview: nothing has been allocated yet.
    pub fn preview_number(language: Language) -> &'static str {
        sentence!(language, "(not issued)", "(nog niet verstuurd)")
    }

    /// The preview could not be rendered.
    pub fn preview_failed(language: Language, error: &str) -> String {
        sentence!(
            language,
            format!("Could not render the preview: {error}"),
            format!("Het voorbeeld kon niet worden opgemaakt: {error}")
        )
    }

    /// A note under a preview that runs to more than one page.
    pub fn preview_more_pages(language: Language, pages: usize) -> String {
        sentence!(
            language,
            format!("The document runs to {pages} pages; the preview shows the first."),
            format!("Het document telt {pages} pagina's; het voorbeeld toont de eerste.")
        )
    }

    /// Number of the issued document.
    pub fn number_line(language: Language, number: &str) -> String {
        sentence!(language, format!("Number: {number}"), format!("Nummer: {number}"))
    }

    /// Issue date of the issued document.
    pub fn issue_date_line(language: Language, date: &str) -> String {
        sentence!(language, format!("Issue date: {date}"), format!("Factuurdatum: {date}"))
    }

    /// Amount payable.
    pub fn payable_line(language: Language, amount: &str) -> String {
        sentence!(language, format!("Payable: {amount}"), format!("Te betalen: {amount}"))
    }

    /// Hash of the UBL artefact.
    pub fn ubl_hash_line(language: Language, hash: &str) -> String {
        sentence!(language, format!("UBL SHA-256: {hash}"), format!("UBL SHA-256: {hash}"))
    }

    /// Hash of the PDF artefact.
    pub fn pdf_hash_line(language: Language, hash: &str) -> String {
        sentence!(language, format!("PDF SHA-256: {hash}"), format!("PDF SHA-256: {hash}"))
    }

    /// Label above the PDF path field.
    /// Writes the PDF, after the platform's save panel asks where.
    pub fn save_pdf_action(language: Language) -> &'static str {
        sentence!(language, "Save PDF…", "PDF opslaan…")
    }

    /// Starts a credit note for the issued invoice.
    pub fn credit_note_action(language: Language) -> &'static str {
        sentence!(language, "Credit note", "Creditnota")
    }

    /// Starts an empty invoice.
    pub fn new_invoice_action(language: Language) -> &'static str {
        sentence!(language, "New invoice", "Nieuwe factuur")
    }

    /// Banner at the top of a credit-note editor.
    pub fn credit_note_banner(language: Language, number: &str) -> String {
        sentence!(
            language,
            format!("Credit note for invoice {number}"),
            format!("Creditnota bij factuur {number}")
        )
    }

    /// Label above the customer picker.
    pub fn customer_label(language: Language) -> &'static str {
        sentence!(language, "Customer", "Klant")
    }

    /// Shown in the customer picker before one is chosen.
    pub fn select_customer(language: Language) -> &'static str {
        sentence!(language, "Select customer", "Kies een klant")
    }

    /// Issue date field.
    pub fn issue_date_label(language: Language) -> &'static str {
        sentence!(language, "Issue date", "Factuurdatum")
    }

    /// Due date field.
    pub fn due_date_label(language: Language) -> &'static str {
        sentence!(language, "Due date", "Vervaldatum")
    }

    /// Buyer reference field.
    pub fn buyer_reference_label(language: Language) -> &'static str {
        sentence!(language, "Buyer ref", "Ref. afnemer")
    }

    /// Order reference field.
    pub fn order_reference_label(language: Language) -> &'static str {
        sentence!(language, "Order ref", "Orderref.")
    }

    /// Delivery date field.
    pub fn delivery_date_label(language: Language) -> &'static str {
        sentence!(language, "Delivery date", "Leverdatum")
    }

    /// Language picker for the printed document.
    pub fn document_language_label(language: Language) -> &'static str {
        sentence!(language, "Document language", "Taal van het document")
    }

    /// The layout this document is printed with.
    pub fn layout_label(language: Language) -> &'static str {
        sentence!(language, "Template", "Sjabloon")
    }

    /// No particular layout, so the archive's default draws.
    pub fn layout_default(language: Language) -> &'static str {
        sentence!(language, "Archive default", "Standaard van het archief")
    }

    /// The chosen layout when its file is no longer in the archive: the draft
    /// still names it, and the picker says so rather than silently showing the
    /// default that will actually draw.
    pub fn layout_missing(language: Language, name: &str) -> String {
        sentence!(language, format!("{name} (removed)"), format!("{name} (verwijderd)"))
    }

    /// Label beside the currency picker.
    pub fn currency_label(language: Language) -> &'static str {
        sentence!(language, "Currency", "Valuta")
    }

    /// Free-text note field.
    pub fn note_label(language: Language) -> &'static str {
        sentence!(language, "Note", "Notitie")
    }

    /// Line entry columns, in order.
    pub fn line_columns(language: Language) -> [&'static str; 5] {
        match language {
            Language::English => ["Item", "Qty", "Unit price", "Rate %", "VAT"],
            Language::Dutch => ["Omschrijving", "Aantal", "Prijs", "Tarief %", "Btw"],
        }
    }

    /// Removes a line.
    pub fn remove_line_action(language: Language) -> &'static str {
        sentence!(language, "Remove", "Verwijderen")
    }

    /// Appends a line.
    pub fn add_line_action(language: Language) -> &'static str {
        sentence!(language, "Add line", "Regel toevoegen")
    }

    /// Stores the draft without issuing it.
    pub fn save_draft_action(language: Language) -> &'static str {
        sentence!(language, "Save draft", "Concept opslaan")
    }

    /// Issues the draft.
    pub fn issue_action(language: Language) -> &'static str {
        sentence!(language, "Issue", "Versturen")
    }

    /// Live totals under the lines.
    pub fn totals(language: Language, net: &str, tax: &str, gross: &str) -> String {
        sentence!(
            language,
            format!("Net {net}   VAT {tax}   Total {gross}"),
            format!("Netto {net}   Btw {tax}   Totaal {gross}")
        )
    }

    /// Title of the suppression dialog.
    pub fn suppress_title(language: Language) -> &'static str {
        sentence!(language, "Suppress warning", "Waarschuwing onderdrukken")
    }

    /// Which finding is being suppressed.
    pub fn suppress_target(language: Language, code: &str, field: &str) -> String {
        sentence!(language, format!("{code} at {field}"), format!("{code} bij {field}"))
    }

    /// Label above the suppression reason.
    pub fn reason_label(language: Language) -> &'static str {
        sentence!(language, "Reason", "Reden")
    }

    /// Stores the suppression.
    pub fn record_suppression_action(language: Language) -> &'static str {
        sentence!(language, "Record suppression", "Onderdrukking vastleggen")
    }

    /// Closes a dialog without acting.
    pub fn cancel_action(language: Language) -> &'static str {
        sentence!(language, "Cancel", "Annuleren")
    }

    /// Points the form at the field a finding names.
    pub fn focus_action(language: Language) -> &'static str {
        sentence!(language, "Focus", "Naar veld")
    }

    /// Records a reason for ignoring a warning.
    pub fn suppress_action(language: Language) -> &'static str {
        sentence!(language, "Suppress", "Onderdrukken")
    }

    /// The draft was stored.
    pub fn draft_saved(language: Language) -> &'static str {
        sentence!(language, "Draft saved.", "Concept opgeslagen.")
    }

    /// The seller record was stored.
    pub fn seller_created(language: Language) -> &'static str {
        sentence!(language, "Seller created.", "Verkoper aangemaakt.")
    }

    /// The customer record was stored.
    pub fn customer_created(language: Language) -> &'static str {
        sentence!(language, "Customer created.", "Klant aangemaakt.")
    }

    /// The editor has no customer yet.
    pub fn select_customer_required(language: Language) -> &'static str {
        sentence!(language, "Select a customer.", "Kies een klant.")
    }

    /// The live totals cannot be calculated without a customer.
    pub fn select_customer_to_validate(language: Language) -> &'static str {
        sentence!(
            language,
            "Select a customer to validate.",
            "Kies een klant om te controleren."
        )
    }

    /// Suppressing a finding requires a seller.
    pub fn seller_required_to_suppress(language: Language) -> &'static str {
        sentence!(
            language,
            "Create the seller before suppressing findings.",
            "Maak eerst de verkoper aan voordat je bevindingen onderdrukt."
        )
    }

    /// Suppressing a finding requires an open invoice.
    pub fn invoice_required_to_suppress(language: Language) -> &'static str {
        sentence!(
            language,
            "Open an invoice before suppressing findings.",
            "Open een factuur voordat je bevindingen onderdrukt."
        )
    }

    /// A suppression needs a reason.
    pub fn suppression_reason_required(language: Language) -> &'static str {
        sentence!(
            language,
            "A suppression reason is required.",
            "Een reden voor onderdrukking is verplicht."
        )
    }

    /// The suppression was recorded.
    pub fn warning_suppressed(language: Language) -> &'static str {
        sentence!(
            language,
            "Warning suppressed with a recorded reason.",
            "Waarschuwing onderdrukt met een vastgelegde reden."
        )
    }

    /// How many findings this document has suppressed, with reasons on file.
    pub fn suppressed_warnings(language: Language, count: usize) -> String {
        sentence!(
            language,
            format!("{count} warning(s) suppressed with a recorded reason"),
            format!("{count} waarschuwing(en) onderdrukt met een vastgelegde reden")
        )
    }

    /// A credit note was started from an invoice.
    pub fn credit_note_started(language: Language, number: &str) -> String {
        sentence!(
            language,
            format!("Credit note for {number}"),
            format!("Creditnota bij {number}")
        )
    }

    /// The number a credit note should correct was not found.
    pub fn invoice_not_found(language: Language, number: &str) -> String {
        sentence!(
            language,
            format!("invoice {number} was not found"),
            format!("factuur {number} is niet gevonden")
        )
    }

    /// A draft handed to `start_credit_note` had no preceding invoice.
    pub fn no_preceding_invoice(language: Language) -> &'static str {
        sentence!(
            language,
            "the draft has no preceding invoice",
            "het concept heeft geen voorafgaande factuur"
        )
    }

    /// Counts the findings that stopped an issue.
    pub fn blocking_findings(language: Language, count: usize) -> String {
        sentence!(
            language,
            format!("{count} blocking finding(s); nothing was issued."),
            format!("{count} blokkerende bevinding(en); er is niets verstuurd.")
        )
    }

    /// Counts the conformance findings that stopped an issue.
    pub fn conformance_findings(language: Language, count: usize) -> String {
        sentence!(
            language,
            format!("{count} conformance finding(s); nothing was issued."),
            format!("{count} conformiteitsbevinding(en); er is niets verstuurd.")
        )
    }

    /// The invoice was issued but no PDF could be made.
    pub fn issued_without_pdf(language: Language, error: &str) -> String {
        sentence!(
            language,
            format!("Issued. PDF not rendered: {error}"),
            format!("Verstuurd. PDF niet opgemaakt: {error}")
        )
    }

    /// The PDF was made but could not be attached to the archive.
    pub fn issued_with_unattached_pdf(language: Language, error: &str) -> String {
        sentence!(
            language,
            format!("Issued, but the PDF was not attached: {error}"),
            format!("Verstuurd, maar de PDF is niet toegevoegd: {error}")
        )
    }

    /// The PDF was written, and where.
    pub fn pdf_saved(language: Language, path: &str) -> String {
        sentence!(
            language,
            format!("PDF saved to {path}"),
            format!("PDF opgeslagen in {path}")
        )
    }

    /// The PDF could not be written.
    pub fn pdf_save_failed(language: Language, error: &str) -> String {
        sentence!(
            language,
            format!("Could not save the PDF: {error}"),
            format!("De PDF kon niet worden opgeslagen: {error}")
        )
    }
}

/// The archive health panel.
pub mod health {
    use super::Language;

    /// Title of the archive health panel.
    pub fn panel_title(language: Language) -> &'static str {
        sentence!(language, "Archive health", "Gezondheid van het archief")
    }

    /// The archive's facts could not be read.
    pub fn unreadable(language: Language) -> &'static str {
        sentence!(
            language,
            "Health could not be read from this archive.",
            "De gezondheid kon niet uit dit archief worden gelezen."
        )
    }

    /// Schema version.
    pub fn schema(language: Language, version: i64) -> String {
        sentence!(language, format!("schema {version}"), format!("schema {version}"))
    }

    /// How large the archive is.
    pub fn size(language: Language, kibibytes: u64) -> String {
        sentence!(language, format!("{kibibytes} KB"), format!("{kibibytes} KB"))
    }

    /// The audit chain, when it has links.
    pub fn chain(language: Language, events: i64) -> String {
        sentence!(
            language,
            format!("audit: {events} events, head"),
            format!("audit: {events} gebeurtenissen, kop")
        )
    }

    /// An archive that has never recorded an event.
    pub fn empty_chain(language: Language) -> &'static str {
        sentence!(language, "audit: empty chain", "audit: lege keten")
    }

    /// The payload audit failed.
    pub fn integrity_failed(language: Language, error: &str) -> String {
        sentence!(language, format!("integrity: {error}"), format!("integriteit: {error}"))
    }

    /// A payload audit is walking the file's pages.
    pub fn checking_structure(language: Language) -> &'static str {
        sentence!(
            language,
            "integrity: checking the file structure…",
            "integriteit: bestandsstructuur controleren…"
        )
    }

    /// How far the chain pass has got.
    pub fn chain_progress(language: Language, done: usize, total: usize) -> String {
        sentence!(
            language,
            format!("audit chain {done}/{total}"),
            format!("auditketen {done}/{total}")
        )
    }

    /// How far the payload pass has got.
    pub fn payloads_progress(language: Language, done: usize, total: usize) -> String {
        sentence!(
            language,
            format!("payloads {done}/{total}"),
            format!("payloads {done}/{total}")
        )
    }

    /// How far the reference sweep has got.
    pub fn references_progress(language: Language, done: usize, total: usize) -> String {
        sentence!(
            language,
            format!("references {done}/{total}"),
            format!("verwijzingen {done}/{total}")
        )
    }

    /// The chain pass finished.
    pub fn chain_verified(language: Language, links: usize) -> String {
        sentence!(
            language,
            format!("chain: {links} links verified"),
            format!("keten: {links} schakels gecontroleerd")
        )
    }

    /// The worker stopped without answering.
    pub fn check_stopped(language: Language) -> &'static str {
        sentence!(
            language,
            "integrity: the check stopped unexpectedly",
            "integriteit: de controle stopte onverwacht"
        )
    }

    /// Every payload verified.
    pub fn integrity_ok(language: Language, payloads: usize) -> String {
        sentence!(
            language,
            format!("integrity: {payloads} payloads verified"),
            format!("integriteit: {payloads} payloads gecontroleerd")
        )
    }

    /// Everything verified, but some blobs are unreferenced.
    pub fn integrity_with_orphans(language: Language, payloads: usize, orphans: usize) -> String {
        sentence!(
            language,
            format!("integrity: {payloads} payloads verified, {orphans} unreferenced (wasted space only)"),
            format!(
                "integriteit: {payloads} payloads gecontroleerd, {orphans} ongerefereerd (alleen verspilde ruimte)"
            )
        )
    }

    /// The audit has not been run.
    pub fn not_checked(language: Language) -> &'static str {
        sentence!(language, "integrity: not checked", "integriteit: niet gecontroleerd")
    }

    /// When the audit last ran.
    pub fn checked(language: Language, at: &str) -> String {
        sentence!(language, format!("checked {at}"), format!("gecontroleerd {at}"))
    }

    /// The newest backup beside the archive.
    pub fn last_backup(language: Language, name: &str, kibibytes: u64) -> String {
        sentence!(
            language,
            format!("last backup: {name} ({kibibytes} KB)"),
            format!("laatste back-up: {name} ({kibibytes} KB)")
        )
    }

    /// There is no backup yet.
    pub fn no_backup(language: Language) -> &'static str {
        sentence!(
            language,
            "last backup: none — run bonk backup",
            "laatste back-up: geen — voer bonk backup uit"
        )
    }

    /// Runs the payload audit now.
    pub fn check_action(language: Language) -> &'static str {
        sentence!(language, "Check integrity now", "Nu integriteit controleren")
    }
}

/// The security panel.
pub mod security {
    use super::Language;

    /// Title of the security panel.
    pub fn panel_title(language: Language) -> &'static str {
        sentence!(language, "Security", "Beveiliging")
    }

    /// Why the current passphrase is asked for again.
    pub fn note(language: Language) -> &'static str {
        sentence!(
            language,
            "A credential change asks for the current passphrase again.",
            "Bij een wijziging van inloggegevens wordt opnieuw om de huidige wachtwoordzin gevraagd."
        )
    }

    /// The current passphrase.
    pub fn current_label(language: Language) -> &'static str {
        sentence!(language, "Current passphrase", "Huidige wachtwoordzin")
    }

    /// Rewraps the data key.
    pub fn change_action(language: Language) -> &'static str {
        sentence!(language, "Change passphrase", "Wachtwoordzin wijzigen")
    }

    /// Adds or replaces the recovery key.
    pub fn add_key_action(language: Language) -> &'static str {
        sentence!(language, "Add recovery key", "Herstelsleutel toevoegen")
    }

    /// Acknowledges a shown key.
    pub fn done_action(language: Language) -> &'static str {
        sentence!(language, "Done", "Klaar")
    }

    /// The key was added.
    pub fn key_added(language: Language) -> &'static str {
        sentence!(language, "Recovery key added.", "Herstelsleutel toegevoegd.")
    }

    /// The passphrase was changed.
    pub fn passphrase_changed(language: Language) -> &'static str {
        sentence!(
            language,
            "Passphrase changed. Backups made before this change still need the old one.",
            "Wachtwoordzin gewijzigd. Back-ups van vóór deze wijziging vragen nog om de oude."
        )
    }

    /// The passphrase field is empty when a key is requested.
    pub fn enter_current(language: Language) -> &'static str {
        sentence!(
            language,
            "Enter the current passphrase first.",
            "Vul eerst de huidige wachtwoordzin in."
        )
    }

    /// Keeps the key in this device's credential store.
    pub fn remember_action(language: Language) -> &'static str {
        sentence!(language, "Remember this device", "Dit apparaat onthouden")
    }

    /// Stops this device from keeping the key.
    pub fn forget_action(language: Language) -> &'static str {
        sentence!(language, "Forget this device", "Dit apparaat vergeten")
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
    pub fn remember_hint(language: Language) -> &'static str {
        sentence!(
            language,
            "Keep the archive key in this machine's credential store, so your login opens it instead of the passphrase.",
            "Bewaar de archiefsleutel in de inloggegevens van deze machine, zodat je aanmelding hem opent in plaats van de wachtwoordzin."
        )
    }

    /// Tooltip when the switch is on.
    pub fn forget_hint(language: Language) -> &'static str {
        sentence!(
            language,
            "Remove the stored key; the passphrase opens this archive again.",
            "Verwijder de bewaarde sleutel; de wachtwoordzin opent dit archief weer."
        )
    }

    /// What the switch means while it is on.
    pub fn device_on_note(language: Language) -> &'static str {
        sentence!(
            language,
            "On — this archive opens with your login, not the passphrase.",
            "Aan — dit archief opent met je aanmelding, niet met de wachtwoordzin."
        )
    }

    /// What the switch means while it is off.
    pub fn device_off_note(language: Language) -> &'static str {
        sentence!(
            language,
            "Off — the passphrase opens this archive.",
            "Uit — de wachtwoordzin opent dit archief."
        )
    }

    /// The device now holds the key.
    pub fn device_remembered(language: Language) -> &'static str {
        sentence!(
            language,
            "This device now opens the archive without the passphrase; your login protects it while it does.",
            "Dit apparaat opent het archief nu zonder de wachtwoordzin; je aanmelding beschermt het zolang dat duurt."
        )
    }

    /// The device no longer holds it.
    pub fn device_forgotten(language: Language) -> &'static str {
        sentence!(
            language,
            "This device has forgotten the key; the passphrase is needed again.",
            "Dit apparaat is de sleutel vergeten; de wachtwoordzin is weer nodig."
        )
    }

    /// There was nothing to forget.
    pub fn device_no_key(language: Language) -> &'static str {
        sentence!(
            language,
            "This device held no key for the archive.",
            "Dit apparaat had geen sleutel voor het archief."
        )
    }
}

/// What is wrong with a form, phrased for the person who has to fix it.
///
/// These are values, not sentences built where the check failed, so the wording
/// lives with the rest of the interface text and the domain keeps its own English
/// wording for logs. A domain error is selected by [`DomainError::code`].
pub mod problem {
    use super::Language;
    use crux::DomainError;

    /// A sentence for a domain error.
    pub fn domain(language: Language, error: &DomainError) -> String {
        match error {
            DomainError::InvalidCurrency(value) => sentence!(
                language,
                format!("{value} is not a currency code."),
                format!("{value} is geen valutacode.")
            ),
            DomainError::CurrencyMismatch { left, right } => sentence!(
                language,
                format!("Amounts in {left} and {right} cannot be added up."),
                format!("Bedragen in {left} en {right} kunnen niet worden opgeteld.")
            ),
            DomainError::AmountOverflow => sentence!(
                language,
                "The amount is too large.".to_owned(),
                "Het bedrag is te groot.".to_owned()
            ),
            DomainError::InvalidDecimal(value) => {
                sentence!(
                    language,
                    format!("{value} is not a number."),
                    format!("{value} is geen getal.")
                )
            }
            DomainError::InvalidScale(places) => sentence!(
                language,
                format!("{places} decimal places cannot be stored."),
                format!("{places} decimalen kunnen niet worden opgeslagen.")
            ),
            DomainError::InvalidQuantity(value) => sentence!(
                language,
                format!("{value} is not a quantity."),
                format!("{value} is geen hoeveelheid.")
            ),
            DomainError::InvalidUnitCode(value) => sentence!(
                language,
                format!("{value} is not a unit code."),
                format!("{value} is geen eenheidscode.")
            ),
            DomainError::InvalidVatRate(value) => sentence!(
                language,
                format!("{value} is not a VAT rate."),
                format!("{value} is geen btw-tarief.")
            ),
            DomainError::InvalidVatTreatment { category, basis_points } => {
                let rate = percent(*basis_points);
                sentence!(
                    language,
                    format!("VAT category {category} cannot use {rate}%."),
                    format!("Btw-categorie {category} kan niet met {rate}%.")
                )
            }
            DomainError::InvalidDate(value) => {
                sentence!(
                    language,
                    format!("{value} is not a date."),
                    format!("{value} is geen datum.")
                )
            }
            DomainError::InvalidCountry(value) => sentence!(
                language,
                format!("{value} is not a country code."),
                format!("{value} is geen landcode.")
            ),
            DomainError::InvalidPhone(value) => sentence!(
                language,
                format!("{value} is not a phone number."),
                format!("{value} is geen telefoonnummer.")
            ),
            DomainError::EmptyInvoice => sentence!(
                language,
                "The invoice has no lines.".to_owned(),
                "De factuur heeft geen regels.".to_owned()
            ),
            DomainError::DateOutOfRange => sentence!(
                language,
                "The date is out of range.".to_owned(),
                "De datum valt buiten het bereik.".to_owned()
            ),
            DomainError::InvalidId(value) => sentence!(
                language,
                format!("{value} is not an identifier."),
                format!("{value} is geen identificatie.")
            ),
            DomainError::DivisionByZero => sentence!(
                language,
                "A division by zero was attempted.".to_owned(),
                "Er werd door nul gedeeld.".to_owned()
            ),
            DomainError::EmptySum => sentence!(
                language,
                "An empty set of amounts cannot be added up.".to_owned(),
                "Een lege reeks bedragen kan niet worden opgeteld.".to_owned()
            ),
        }
    }

    /// A rate in basis points as a percentage, without floating point.
    fn percent(basis_points: u16) -> String {
        let whole = basis_points / 100;
        let fraction = basis_points % 100;
        if fraction == 0 {
            whole.to_string()
        } else {
            format!("{whole}.{fraction:02}")
        }
    }

    /// The invoice has no customer yet.
    pub fn select_customer(language: Language) -> &'static str {
        sentence!(language, "Select a customer.", "Kies een klant.")
    }

    /// A line without a description.
    pub fn missing_item_name(language: Language) -> &'static str {
        sentence!(language, "the item name is required.", "de omschrijving is verplicht.")
    }

    /// The invoice date does not parse.
    pub fn bad_issue_date(language: Language) -> &'static str {
        sentence!(
            language,
            "The invoice date must be YYYY-MM-DD.",
            "De factuurdatum moet JJJJ-MM-DD zijn."
        )
    }

    /// The due date does not parse.
    pub fn bad_due_date(language: Language) -> &'static str {
        sentence!(
            language,
            "The due date must be YYYY-MM-DD.",
            "De vervaldatum moet JJJJ-MM-DD zijn."
        )
    }

    /// The delivery date does not parse.
    pub fn bad_delivery_date(language: Language) -> &'static str {
        sentence!(
            language,
            "The delivery date must be YYYY-MM-DD.",
            "De leverdatum moet JJJJ-MM-DD zijn."
        )
    }

    /// There are more lines than a document can carry.
    pub fn too_many_lines(language: Language) -> &'static str {
        sentence!(language, "Too many lines.", "Te veel regels.")
    }

    /// A problem on one line, with the line named first.
    pub fn on_line(language: Language, position: u32, problem: &str) -> String {
        sentence!(
            language,
            format!("Line {position}: {problem}"),
            format!("Regel {position}: {problem}")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No string is left with an English-only column.
    ///
    /// A match arm that names both languages and returns the same sentence is a
    /// translation nobody wrote, and a parameter that ignores the language is a
    /// string that never asks. The two exceptions are language-neutral by nature:
    /// the keyboard hint and the matched-over-total counter.
    #[test]
    fn no_string_falls_back_to_english() {
        let source = include_str!("text.rs");

        // Built rather than written out, so this test does not find itself.
        let fallback = concat!("Language::English | ", "Language::Dutch");
        assert!(!source.contains(fallback), "a Dutch arm still copies the English one");

        let ignored = concat!("(_: ", "Language");
        assert_eq!(
            source.matches(ignored).count(),
            2,
            "only the shortcut hint and the count line are language-neutral"
        );
    }

    #[test]
    fn the_dutch_table_says_something_different() {
        let english = Language::English;
        let dutch = Language::Dutch;

        assert_eq!(tabs::search(english), "Search");
        assert_eq!(tabs::search(dutch), "Zoeken");
        assert_eq!(editor::issue_action(dutch), "Versturen");
        assert!(find::columns(dutch).contains(&"Nummer"));
        assert!(editor::weekdays(dutch).contains(&"ma"));
        assert_ne!(shell::tagline(english), shell::tagline(dutch));
        assert_eq!(settings::language(dutch), "Taal");
    }

    #[test]
    fn a_problem_is_a_sentence_not_the_log_wording() {
        let language = Language::default();
        let error = crux::DomainError::InvalidDecimal("two".to_owned());
        let sentence = problem::domain(language, &error);
        assert!(sentence.contains("two"), "the value is kept: {sentence}");
        assert_ne!(
            sentence,
            error.to_string(),
            "the interface writes its own sentence rather than printing the log wording"
        );

        let line = problem::on_line(language, 3, problem::missing_item_name(language));
        assert_eq!(line, "Line 3: the item name is required.");
    }

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
            shell::browse_action(language),
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
        assert!(health::payloads_progress(language, 3, 9).contains("3/9"));
        assert!(health::references_progress(language, 4, 8).contains("4/8"));
        assert!(health::chain_progress(language, 2, 8).contains("2/8"));
        assert!(health::chain_verified(language, 5).contains('5'));
        assert!(!health::checking_structure(language).is_empty());
        assert!(!health::check_stopped(language).is_empty());
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
