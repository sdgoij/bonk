package com.github.sdgoij.bonk

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue

/** The language the app draws in. */
enum class Lang {
    English,
    Dutch,
    ;

    /** Stored code, also what the settings screen saves. */
    val code: String get() = if (this == Dutch) "nl" else "en"

    companion object {
        /** Reads a stored code: anything starting with `nl` is Dutch. */
        fun parse(code: String): Lang = if (code.trim().lowercase().startsWith("nl")) Dutch else English
    }
}

/**
 * The app's own words, in one place.
 *
 * A screen never builds a sentence itself, so a new language is one file and not
 * a hunt through every composable. The two halves sit side by side on each line,
 * which is what keeps them from drifting apart.
 */
class Strings(private val dutch: Boolean) {
    private fun t(english: String, dutch: String): String = if (this.dutch) dutch else english

    // The shell.
    val tagline = t("Local-first e-invoicing", "Factureren zonder server")
    val unknownError = t("unknown error", "onbekende fout")
    val back = t("Back", "Terug")
    val home = t("Home", "Start")
    val close = t("Close", "Sluiten")
    val cancel = t("Cancel", "Annuleren")
    val saved = t("Saved.", "Opgeslagen.")
    val saving = t("Saving...", "Bezig met opslaan...")
    val working = t("Working...", "Bezig...")

    // Create.
    val createTitle = t("Create your archive", "Maak je archief")
    val createNote = t(
        "The passphrase protects the archive on this device. There is no way to recover it if it is lost.",
        "De wachtwoordzin beschermt het archief op dit toestel. Als je die kwijt bent, is er geen weg terug.",
    )
    val passphrase = t("Passphrase", "Wachtwoordzin")
    val repeatPassphrase = t("Repeat passphrase", "Herhaal wachtwoordzin")
    val rememberDevice = t("Remember this device", "Onthoud dit toestel")
    val rememberNote = t(
        "Unlocks with your fingerprint or face next time. The passphrase still works.",
        "Ontgrendelt de volgende keer met je vingerafdruk of gezicht. De wachtwoordzin blijft werken.",
    )
    val creating = t("Creating...", "Bezig met maken...")
    val createArchive = t("Create archive", "Archief maken")

    // Reset and recovery.
    val resetTitle = t("Reset the passphrase", "Wachtwoordzin herstellen")
    val resetNote = t(
        "Enter the recovery key you wrote down. It will be spent, and a new one will be shown.",
        "Vul de herstelsleutel in die je hebt opgeschreven. Die wordt verbruikt en je krijgt een nieuwe te zien.",
    )
    val recoveryKey = t("Recovery key", "Herstelsleutel")
    val newPassphrase = t("New passphrase", "Nieuwe wachtwoordzin")
    val repeatNewPassphrase = t("Repeat new passphrase", "Herhaal nieuwe wachtwoordzin")
    val resetting = t("Resetting...", "Bezig met herstellen...")
    val resetAction = t("Reset passphrase", "Wachtwoordzin herstellen")
    val addRecoveryTitle = t("Add a recovery key", "Herstelsleutel toevoegen")
    val addRecoveryNote = t(
        "Enter the current passphrase. The recovery key is shown once and replaces any earlier one.",
        "Vul de huidige wachtwoordzin in. De herstelsleutel wordt één keer getoond en vervangt een eerdere.",
    )
    val createRecoveryKey = t("Create recovery key", "Herstelsleutel maken")
    val recoveryShownTitle = t("Write down this recovery key", "Schrijf deze herstelsleutel op")
    val recoveryShownNote = t(
        "This is the only way back in if you forget the passphrase. It is shown once.",
        "Dit is de enige weg terug als je de wachtwoordzin vergeet. Je ziet hem één keer.",
    )
    val recoveryKeep = t(
        "Keep it somewhere other than this device. Anyone with this key can reset the passphrase.",
        "Bewaar hem ergens anders dan op dit toestel. Wie deze sleutel heeft, kan de wachtwoordzin herstellen.",
    )
    val wroteItDown = t("I wrote it down", "Ik heb hem opgeschreven")

    // Unlock.
    val unlockTitle = t("Unlock your archive", "Ontgrendel je archief")
    val unlockNote = t(
        "Enter the passphrase you chose for this device.",
        "Vul de wachtwoordzin in die je voor dit toestel koos.",
    )
    val unlocking = t("Unlocking...", "Bezig met ontgrendelen...")
    val unlock = t("Unlock", "Ontgrendelen")
    val unlockWithDevice = t("Unlock with device", "Ontgrendelen met dit toestel")
    val unlockWithDeviceNote = t(
        "Uses your fingerprint or face. The passphrase still works.",
        "Gebruikt je vingerafdruk of gezicht. De wachtwoordzin blijft werken.",
    )
    val forgotPassphrase = t("Forgot passphrase?", "Wachtwoordzin vergeten?")

    // What the shell reports while it works.
    val enterPassphrase = t("Enter the passphrase.", "Vul de wachtwoordzin in.")
    val choosePassphrase = t("Choose a passphrase.", "Kies een wachtwoordzin.")
    val passphrasesDiffer = t("The passphrases do not match.", "De wachtwoordzinnen zijn niet gelijk.")
    val enterRecoveryKey = t("Enter the recovery key.", "Vul de herstelsleutel in.")
    val chooseNewPassphrase = t("Choose a new passphrase.", "Kies een nieuwe wachtwoordzin.")
    val newPassphrasesDiffer = t(
        "The new passphrases do not match.",
        "De nieuwe wachtwoordzinnen zijn niet gelijk.",
    )
    val enterCurrentPassphrase = t("Enter the current passphrase.", "Vul de huidige wachtwoordzin in.")
    val notRemembered = t(
        "The archive was created, but this device was not remembered: ",
        "Het archief is aangemaakt, maar dit toestel is niet onthouden: ",
    )
    val noRecoveryMade = t(
        "The archive was created, but no recovery key was generated: ",
        "Het archief is aangemaakt, maar er is geen herstelsleutel gemaakt: ",
    )
    val deviceKeyInvalid = t(
        "The device key is no longer valid. Unlock with the passphrase.",
        "De sleutel van dit toestel is niet meer geldig. Ontgrendel met de wachtwoordzin.",
    )
    val deviceForgotten = t("This device no longer opens the archive.", "Dit toestel opent het archief niet meer.")
    val noDeviceKey = t("No device key was stored.", "Er was geen sleutel van dit toestel opgeslagen.")
    val archiveRestored = t("Archive restored.", "Archief hersteld.")

    // Dashboard.
    val unlocked = t("Unlocked", "Ontgrendeld")
    val sellers = t("Sellers", "Verkopers")
    val drafts = t("Drafts", "Concepten")
    val schema = t("Schema", "Schema")
    val customers = t("Customers", "Klanten")
    val documents = t("Documents", "Documenten")
    val importInvoice = t("Import invoice", "Factuur importeren")
    val backup = t("Backup", "Back-up")
    val settings = t("Settings", "Instellingen")
    val lock = t("Lock", "Vergrendelen")
    val forgetDevice = t("Forget this device", "Dit toestel vergeten")

    // Customers.
    val search = t("Search", "Zoeken")
    val addCustomer = t("Add customer", "Klant toevoegen")
    val importCsv = t("Import CSV", "CSV importeren")
    val exportCsv = t("Export CSV", "CSV exporteren")
    val noCustomers = t("No customers yet.", "Nog geen klanten.")
    val chooseCustomerToEdit = t("Choose a customer to edit.", "Kies een klant om te bewerken.")
    val enterName = t("Enter a name.", "Vul een naam in.")
    val enterCity = t("Enter a city.", "Vul een plaats in.")
    val newCustomer = t("New customer", "Nieuwe klant")
    val editCustomer = t("Edit customer", "Klant bewerken")
    val name = t("Name", "Naam")
    val street = t("Street", "Straat")
    val postal = t("Postal code", "Postcode")
    val city = t("City", "Plaats")
    val country = t("Country", "Land")
    val vat = t("VAT identifier", "Btw-nummer")
    val kvk = t("Chamber of commerce", "KvK-nummer")
    val peppol = t("Peppol endpoint", "Peppol-endpoint")
    val contact = t("Contact", "Contactpersoon")
    val email = t("Email", "E-mail")
    val phone = t("Phone", "Telefoon")
    val saveCustomer = t("Save customer", "Klant opslaan")

    fun importedSkipped(imported: Int, skipped: Int): String =
        t("Imported $imported, skipped $skipped", "$imported geïmporteerd, $skipped overgeslagen")

    // Drafts.
    val newDraft = t("New draft", "Nieuw concept")
    val noDrafts = t("No drafts yet.", "Nog geen concepten.")
    val addCustomerFirst = t("Add a customer first.", "Voeg eerst een klant toe.")
    val noCustomer = t("No customer", "Geen klant")
    val draft = t("Draft", "Concept")
    val chooseCustomer = t("Choose a customer", "Kies een klant")
    val issueDate = t("Issue date", "Factuurdatum")
    val dueDate = t("Due date", "Vervaldatum")
    val note = t("Note", "Notitie")
    val buyerReference = t("Buyer reference", "Klantreferentie")
    val orderReference = t("Order reference", "Orderreferentie")
    val lines = t("Lines", "Regels")
    val addLine = t("Add line", "Regel toevoegen")
    val item = t("Item", "Omschrijving")
    val quantity = t("Quantity", "Aantal")
    val unit = t("Unit", "Eenheid")
    val unitPrice = t("Unit price", "Prijs")
    val vatRate = t("VAT %", "Btw %")
    val removeLine = t("Remove line", "Regel verwijderen")
    val findings = t("Findings", "Bevindingen")
    val suppress = t("Suppress", "Onderdrukken")
    val issuing = t("Issuing...", "Bezig met uitgeven...")
    val issueCreditNote = t("Issue credit note", "Creditnota uitgeven")
    val issueInvoice = t("Issue invoice", "Factuur uitgeven")
    val notIssued = t("Not issued", "Niet uitgegeven")
    val refused = t("The archive refused the document.", "Het archief weigerde het document.")
    val refusedWithoutFinding = t(
        "The archive refused the document without a finding.",
        "Het archief weigerde het document zonder bevinding.",
    )
    val reason = t("Reason", "Reden")
    val preview = t("Invoice preview", "Factuurvoorbeeld")
    val previewUnavailable = t("The preview could not be drawn.", "Het voorbeeld kon niet worden getekend.")

    fun netTotal(amount: String) = t("Net $amount", "Netto $amount")

    fun vatTotal(amount: String) = t("VAT $amount", "Btw $amount")

    fun grossTotal(amount: String) = t("Gross $amount", "Bruto $amount")

    fun refusedFields(count: Int): String = t(
        "The archive refused the document. $count of these point at a field this screen does not edit.",
        "Het archief weigerde het document. $count daarvan wijzen naar een veld dat dit scherm niet bewerkt.",
    )

    fun suppressFinding(code: String) = t("Suppress $code", "$code onderdrukken")

    fun linesCount(count: Int) = when (count) {
        1 -> t("1 line", "1 regel")
        else -> t("$count lines", "$count regels")
    }

    // The issued result and its artefacts.
    val issuedTitle = t("Issued", "Verstuurd")
    val filedArtefacts = t("Filed artefacts", "Bewaarde bestanden")
    val warnings = t("Warnings", "Waarschuwingen")
    val share = t("Share", "Delen")
    val save = t("Save", "Opslaan")
    val open = t("Open", "Openen")
    val preparingFile = t("Preparing the file...", "Bestand klaarmaken...")

    fun issuedOn(date: String) = t("Issued $date", "Verstuurd op $date")

    fun pdfNotFiled(error: String) =
        t("The PDF could not be filed: $error", "De PDF kon niet worden bewaard: $error")

    // Import.
    val importTitle = t("Import", "Importeren")
    val importNote = t(
        "A received invoice arrives as UBL or PDF. The archive reads the sender, the number " +
            "and the totals, and refuses to store the same document twice.",
        "Een ontvangen factuur komt binnen als UBL of PDF. Het archief leest de afzender, het " +
            "nummer en de totalen, en bewaart hetzelfde document niet twee keer.",
    )
    val importing = t("Importing...", "Bezig met importeren...")
    val chooseFile = t("Choose a file", "Kies een bestand")
    val received = t("Received", "Ontvangen")
    val file = t("File", "Bestand")
    val number = t("Number", "Nummer")
    val supplier = t("Supplier", "Leverancier")
    val notMatchedToCustomer = t("Not matched to a customer", "Niet aan een klant gekoppeld")
    val alreadyHeld = t(
        "The archive already held this document, so nothing was added.",
        "Het archief had dit document al, dus er is niets toegevoegd.",
    )
    val openDocument = t("Open document", "Document openen")

    // Backup, kit and restore.
    val twoFiles = t(
        "The archive is two files: the database and the key that opens it. A backup holds the data, " +
            "the recovery kit holds the key, and neither is enough on its own.",
        "Het archief is twee bestanden: de database en de sleutel die hem opent. Een back-up bevat " +
            "de gegevens, de herstelset bevat de sleutel, en geen van beide is alleen genoeg.",
    )
    val verifiedCopy = t(
        "A verified, encrypted copy. It is checked here before you choose where it goes, and the " +
            "app's own copy is deleted once it is written.",
        "Een gecontroleerde, versleutelde kopie. Die wordt hier gecontroleerd voordat je kiest waar " +
            "hij heen gaat, en de eigen kopie van de app wordt gewist zodra hij geschreven is.",
    )
    val exportBackup = t("Export backup", "Back-up exporteren")
    val recoveryKit = t("Recovery kit", "Herstelset")
    val recoveryKitNote = t(
        "The key, with the note that explains what it is. Keep it on another medium from the " +
            "backups: anyone who has both this file and the passphrase has the archive.",
        "De sleutel, met de uitleg erbij. Bewaar hem op een ander medium dan de back-ups: wie dit " +
            "bestand én de wachtwoordzin heeft, heeft het archief.",
    )
    val exportKit = t("Export recovery kit", "Herstelset exporteren")
    val restore = t("Restore", "Herstellen")
    val restoreNote = t(
        "Replaces this device's archive with a backup and the key that opens it. Both files are " +
            "needed. The archive here now is kept beside the new one, so a wrong choice is recoverable.",
        "Vervangt het archief op dit toestel door een back-up en de sleutel die hem opent. Beide " +
            "bestanden zijn nodig. Het archief dat er nu staat blijft ernaast bewaard, dus een " +
            "verkeerde keuze is terug te draaien.",
    )
    val bothFilesReady = t(
        "Both files are ready. Enter the passphrase of that backup.",
        "Beide bestanden zijn klaar. Vul de wachtwoordzin van die back-up in.",
    )
    val chooseTwoAgain = t("Choose the two files again", "Kies de twee bestanden opnieuw")
    val chooseTwo = t("Choose the two files", "Kies de twee bestanden")
    val backupPassphrase = t("Passphrase of that backup", "Wachtwoordzin van die back-up")
    val restoring = t("Restoring...", "Bezig met herstellen...")
    val replaceArchive = t("Replace this archive", "Dit archief vervangen")
    val replaceQuestion = t("Replace the archive?", "Het archief vervangen?")
    val replaceNote = t(
        "The archive on this device is replaced by the pair you picked. The files that " +
            "are here now stay beside the new ones.",
        "Het archief op dit toestel wordt vervangen door het paar dat je koos. De bestanden " +
            "die er nu staan blijven ernaast bewaard.",
    )
    val replace = t("Replace", "Vervangen")

    fun freeSpace(needed: String, free: String) = t(
        "Not enough free space: a copy needs about $needed, and $free is free.",
        "Niet genoeg vrije ruimte: een kopie vraagt ongeveer $needed en er is $free vrij.",
    )

    fun savedVerified(name: String, kilobytes: Long) =
        t("Saved $name ($kilobytes KB), verified.", "$name opgeslagen ($kilobytes KB), gecontroleerd.")

    fun kitWritten(where: String) = t(
        "Recovery kit written to $where. Keep it apart from the backups.",
        "Herstelset geschreven naar $where. Bewaar die los van de back-ups.",
    )

    // Settings.
    val appLanguage = t("App language", "App-taal")
    val appLanguageNote = t(
        "The language of the app itself. Documents print in the language above.",
        "De taal van de app zelf. Documenten worden in de taal hierboven gedrukt.",
    )
    val documentLanguage = t("Document language", "Documenttaal")
    val documentLanguageNote = t(
        "What an issued invoice prints in. This is the document's language, not the app's.",
        "De taal waarin een uitgegeven factuur wordt gedrukt. Dit is de taal van het document, " +
            "niet van de app.",
    )
    val english = t("English", "Engels")
    val dutchLanguage = t("Nederlands", "Nederlands")
    val seller = t("Seller", "Verkoper")
    val noVat = t("No VAT identifier yet", "Nog geen btw-nummer")
    val editSeller = t("Edit seller", "Verkoper bewerken")
    val saveSeller = t("Save seller", "Verkoper opslaan")
    val sellerNote = t(
        "Invoices are issued by this party. Conformance checks its address, VAT identifier and Peppol endpoint.",
        "Facturen worden door deze partij uitgegeven. De conformiteitscontrole kijkt naar het adres, " +
            "het btw-nummer en het Peppol-endpoint.",
    )
    val recovery = t("Recovery", "Herstel")
    val recoveryNote = t(
        "A recovery key opens the archive when the passphrase is lost. It is shown once.",
        "Een herstelsleutel opent het archief als de wachtwoordzin kwijt is. Je ziet hem één keer.",
    )
    val addRecoveryKey = t("Add recovery key", "Herstelsleutel toevoegen")
    val auditChain = t("Audit chain", "Auditketen")
    val noEvents = t(
        "No events yet. Every issue, match and status change appends to it.",
        "Nog geen gebeurtenissen. Elke uitgifte, koppeling en statuswijziging komt erbij.",
    )
    val events = t("Events", "Gebeurtenissen")
    val recorded = t("Recorded", "Vastgelegd")

    // The register.
    val issuedTab = t("Issued", "Verstuurd")
    val receivedTab = t("Received", "Ontvangen")
    val allFilter = t("All", "Alle")
    val paid = t("Paid", "Betaald")
    val archived = t("Archived", "Gearchiveerd")
    val approved = t("Approved", "Goedgekeurd")
    val searchDocuments = t(
        "Search numbers, parties, references",
        "Zoek op nummer, partij, referentie",
    )
    val nothingMatches = t("Nothing matches that search.", "Niets gevonden met die zoekopdracht.")
    val chooseDocumentToOpen = t("Choose a document to open.", "Kies een document om te openen.")
    val noReceived = t(
        "No received invoices yet. Import one to file it here.",
        "Nog geen ontvangen facturen. Importeer er een om hem hier te bewaren.",
    )
    val noDocuments = t(
        "No documents yet. Issue a draft and it is filed here.",
        "Nog geen documenten. Geef een concept uit en het komt hier te staan.",
    )
    val notMatched = t("not matched", "niet gekoppeld")
    val noNumber = t("No number", "Geen nummer")
    val document = t("Document", "Document")
    val issuedFact = t("Issued", "Factuurdatum")
    val dueFact = t("Due", "Vervaldatum")
    val buyer = t("Buyer", "Koper")
    val matchSupplier = t("Match supplier", "Leverancier koppelen")
    val changeSupplier = t("Change supplier", "Leverancier wijzigen")
    val notMatchedTitle = t("Not matched", "Niet gekoppeld")
    val approve = t("Approve", "Goedkeuren")
    val markPaid = t("Mark paid", "Als betaald markeren")
    val noneYet = t("None yet.", "Nog geen.")
    val history = t("History", "Geschiedenis")
    val noCustomersFirst = t(
        "No customers yet. Add one in Customers first.",
        "Nog geen klanten. Voeg er eerst een toe bij Klanten.",
    )

    fun matches(count: Int) = when {
        count == 1 -> t("1 match", "1 resultaat")
        else -> t("$count matches", "$count resultaten")
    }

    fun noStatusDocuments(status: String) =
        t("No documents with status $status yet.", "Nog geen documenten met status $status.")

    fun supplierMatched(name: String) = t("Supplier matched to $name.", "Leverancier gekoppeld aan $name.")

    fun nowStatus(status: String) = t("The document is now $status.", "Het document staat nu op $status.")

    fun line(position: Long) = t("Line $position", "Regel $position")

    fun lineDetail(quantity: String, unit: String, price: String, currency: String, rate: String) = t(
        "$quantity $unit x $price $currency · VAT $rate%",
        "$quantity $unit x $price $currency · Btw $rate%",
    )

    fun artefactLine(kind: String, bytes: Long) = t("$kind · $bytes bytes", "$kind · $bytes bytes")

    /** A stored status code as a word. */
    fun status(code: String): String = when (code) {
        "draft" -> t("Draft", "Concept")
        "issued" -> t("Issued", "Verstuurd")
        "received" -> t("Received", "Ontvangen")
        "approved" -> t("Approved", "Goedgekeurd")
        "paid" -> t("Paid", "Betaald")
        "archived" -> t("Archived", "Gearchiveerd")
        else -> code
    }

    /** One line of the status history. */
    fun event(kind: String, at: String, actor: String, detail: String): String {
        val head = "${status(kind)} · $at · $actor"
        return if (detail.isBlank()) head else "$head · $detail"
    }

    // What the platform layer reports.
    val wrongPassphrase = t("Wrong passphrase. Try again.", "Verkeerde wachtwoordzin. Probeer opnieuw.")
    val archiveExists = t(
        "An archive already exists on this device.",
        "Op dit toestel staat al een archief.",
    )
    val damagedKeystore = t(
        "The archive keystore is damaged or from a newer version.",
        "De sleutelopslag van het archief is beschadigd of van een nieuwere versie.",
    )
    val noRecoveryKey = t("This archive has no recovery key.", "Dit archief heeft geen herstelsleutel.")
    val invalidRecoveryKey = t("That recovery key is not valid.", "Die herstelsleutel is niet geldig.")
    val deviceInvalidated = t("The device secret was invalidated.", "Het geheim van dit toestel is ongeldig.")
    val archiveLocked = t("The archive is locked.", "Het archief is vergrendeld.")

    fun storageError(detail: String) = t("Storage error: $detail", "Opslagfout: $detail")

    fun renderingFailed(detail: String) = t("Rendering failed: $detail", "Weergeven mislukt: $detail")

    fun authenticationFailed(detail: String) =
        t("Authentication failed: $detail", "Verificatie mislukt: $detail")

    fun couldNotImport(detail: String) = t("Could not import the file: $detail", "Kon het bestand niet importeren: $detail")

    // Diagnostics.
    val diagnostics = t("Diagnostics", "Diagnostiek")
    val hideDiagnostics = t("Hide diagnostics", "Diagnostiek verbergen")
    val notRun = t("not run", "niet uitgevoerd")
    val runKeystore = t("Run S3 Keystore test", "S3-keystoretest uitvoeren")
    val waitingForAuth = t("waiting for authentication...", "wachten op verificatie...")
    val benchArgon = t("Bench Argon2 (64 MiB)", "Argon2 meten (64 MiB)")
    val runningArgon = t("running 64 MiB Argon2id...", "64 MiB Argon2id loopt...")
    val hashArtefacts = t("Hash UBL + PDF", "UBL + PDF hashen")
    val hashingArtefacts = t("hashing UBL and PDF...", "UBL en PDF hashen...")
    val canary = t("Canary scan", "Canarisscan")
    val scanning = t(
        "scanning database, WAL, and keystore...",
        "database, WAL en sleutelopslag scannen...",
    )

    companion object {
        val English = Strings(false)
        val Dutch = Strings(true)
    }
}

/** Which language the app draws in, and the words for it. */
object UiLanguage {
    var current by mutableStateOf(Lang.English)
}

/**
 * The words for the language the user picked.
 *
 * Reading this in a composable subscribes it to the choice, so changing the
 * language redraws every screen that used it.
 */
val strings: Strings
    get() = if (UiLanguage.current == Lang.Dutch) Strings.Dutch else Strings.English
