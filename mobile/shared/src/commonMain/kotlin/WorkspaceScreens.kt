package com.github.sdgoij.bonk

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.PrimaryTabRow
import androidx.compose.material3.Tab
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlin.math.abs

@Composable
fun CustomerListScreen(
    onBack: () -> Unit,
    onEdit: (Customer) -> Unit,
    onAdd: () -> Unit,
    onInteraction: () -> Unit,
) {
    var customers by remember { mutableStateOf<List<Customer>>(emptyList()) }
    var query by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    var report by remember { mutableStateOf<CustomerImportReport?>(null) }
    var loading by remember { mutableStateOf(true) }
    var refresh by remember { mutableStateOf(0) }
    val scope = rememberCoroutineScope()
    val csvIo = rememberCsvIo()

    LaunchedEffect(refresh) {
        withContext(Dispatchers.IO) { runCatching { listCustomers() } }
            .onSuccess { customers = it }
            .onFailure { error = it.message ?: it.toString() }
        loading = false
    }

    fun exportCsv() {
        scope.launch {
            val csv = withContext(Dispatchers.IO) { runCatching { customersCsv() } }
            csv.onSuccess { csvIo.export(it, "customers.csv") }
                .onFailure { error = it.message ?: it.toString() }
        }
    }

    fun importCsv() {
        csvIo.pick { text ->
            scope.launch {
                val result = withContext(Dispatchers.IO) { runCatching { importCustomersCsv(text) } }
                result.onSuccess {
                    report = it
                    error = null
                    refresh++
                }.onFailure { error = it.message ?: it.toString() }
            }
        }
    }

    val visible = if (query.isBlank()) {
        customers
    } else {
        customers.filter {
            it.name.contains(query, ignoreCase = true) ||
                it.city.contains(query, ignoreCase = true) ||
                it.vat.contains(query, ignoreCase = true)
        }
    }

    Column(Modifier.fillMaxWidth()) {
        ScreenHeader("Customers", onBack) { }
        OutlinedTextField(
            value = query,
            onValueChange = {
                query = it
                onInteraction()
            },
            label = { Text("Search") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        Spacer(Modifier.height(8.dp))
        Button(onClick = { onInteraction(); onAdd() }, modifier = Modifier.fillMaxWidth()) {
            Text("Add customer")
        }
        Spacer(Modifier.height(8.dp))
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton(
                onClick = { onInteraction(); importCsv() },
                modifier = Modifier.weight(1f),
            ) { Text("Import CSV") }
            OutlinedButton(
                onClick = { onInteraction(); exportCsv() },
                modifier = Modifier.weight(1f),
            ) { Text("Export CSV") }
        }
        report?.let {
            Spacer(Modifier.height(8.dp))
            Text(
                "Imported ${it.imported}, skipped ${it.skipped}",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            it.errors.forEach { message ->
                Text(message, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
            }
        }
        Spacer(Modifier.height(8.dp))
        if (loading) {
            CircularProgressIndicator(modifier = Modifier.padding(16.dp))
        } else if (visible.isEmpty()) {
            Text("No customers yet.", modifier = Modifier.padding(16.dp))
        } else {
            visible.forEach { customer ->
                CustomerRow(customer) { onInteraction(); onEdit(customer) }
                HorizontalDivider()
            }
        }
        error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
    }
}

@Composable
fun CustomerEditScreen(
    customer: Customer,
    onBack: () -> Unit,
    onSaved: () -> Unit,
    onInteraction: () -> Unit,
) {
    var form by remember { mutableStateOf(customer) }
    var error by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    fun save() {
        if (form.name.isBlank()) {
            error = "Enter a name."
            return
        }
        if (form.city.isBlank()) {
            error = "Enter a city."
            return
        }
        busy = true
        error = null
        scope.launch {
            val result = withContext(Dispatchers.IO) { runCatching { saveCustomer(form) } }
            result.onSuccess { onSaved() }.onFailure { error = it.message ?: it.toString() }
            busy = false
        }
    }

    Column(Modifier.fillMaxWidth()) {
        ScreenHeader(if (customer.id.isEmpty()) "New customer" else "Edit customer", onBack) { }
        FormField("Name", form.name) { form = form.copy(name = it); onInteraction() }
        FormField("Street", form.street) { form = form.copy(street = it); onInteraction() }
        FormField("Postal code", form.postal) { form = form.copy(postal = it); onInteraction() }
        FormField("City", form.city) { form = form.copy(city = it); onInteraction() }
        FormField("Country", form.country) { form = form.copy(country = it.uppercase()); onInteraction() }
        FormField("VAT identifier", form.vat) { form = form.copy(vat = it); onInteraction() }
        FormField("Chamber of commerce", form.kvk) { form = form.copy(kvk = it); onInteraction() }
        FormField("Peppol endpoint", form.peppol) { form = form.copy(peppol = it); onInteraction() }
        FormField("Contact", form.contact) { form = form.copy(contact = it); onInteraction() }
        FormField("Email", form.email) { form = form.copy(email = it); onInteraction() }
        FormField("Phone", form.phone) { form = form.copy(phone = it); onInteraction() }
        error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        Spacer(Modifier.height(12.dp))
        Button(onClick = ::save, enabled = !busy, modifier = Modifier.fillMaxWidth()) {
            Text(if (busy) "Saving..." else "Save customer")
        }
    }
}

@Composable
fun DraftListScreen(
    onBack: () -> Unit,
    onOpen: (Draft, List<Customer>) -> Unit,
    onCustomers: () -> Unit,
    onInteraction: () -> Unit,
) {
    var drafts by remember { mutableStateOf<List<Draft>>(emptyList()) }
    var customers by remember { mutableStateOf<List<Customer>>(emptyList()) }
    var totals by remember { mutableStateOf<Map<String, DraftTotals>>(emptyMap()) }
    var error by remember { mutableStateOf<String?>(null) }
    var loading by remember { mutableStateOf(true) }
    val scope = rememberCoroutineScope()

    LaunchedEffect(Unit) {
        val loaded = withContext(Dispatchers.IO) {
            runCatching {
                val loadedCustomers = listCustomers()
                val loadedDrafts = listDrafts()
                val loadedTotals = loadedDrafts.associate { it.id to draftTotals(it) }
                Triple(loadedDrafts, loadedCustomers, loadedTotals)
            }
        }
        loaded.onSuccess { (d, c, t) -> drafts = d; customers = c; totals = t }
            .onFailure { error = it.message ?: it.toString() }
        loading = false
    }

    fun create() {
        if (customers.isEmpty()) {
            error = "Add a customer first."
            return
        }
        scope.launch {
            val result = withContext(Dispatchers.IO) { runCatching { newDraft(customers.first().id) } }
            result.onSuccess { onOpen(it, customers) }.onFailure { error = it.message ?: it.toString() }
        }
    }

    Column(Modifier.fillMaxWidth()) {
        ScreenHeader("Drafts", onBack) {
            TextButton(onClick = { onInteraction(); onCustomers() }) { Text("Customers") }
        }
        Button(onClick = { onInteraction(); create() }, modifier = Modifier.fillMaxWidth()) {
            Text("New draft")
        }
        Spacer(Modifier.height(8.dp))
        if (loading) {
            CircularProgressIndicator(modifier = Modifier.padding(16.dp))
        } else if (drafts.isEmpty()) {
            Text("No drafts yet.", modifier = Modifier.padding(16.dp))
        } else {
            drafts.forEach { draft ->
                DraftRow(
                    draft = draft,
                    customerName = customers.firstOrNull { it.id == draft.customerId }?.name ?: "No customer",
                    gross = totals[draft.id]?.let { "${formatMoney(it.grossMicros)} ${draft.currency}" } ?: "",
                ) { onInteraction(); onOpen(draft, customers) }
                HorizontalDivider()
            }
        }
        error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
    }
}

@Composable
fun DraftEditorScreen(
    draft: Draft,
    customers: List<Customer>,
    onBack: () -> Unit,
    onInteraction: () -> Unit,
    onIssued: (IssueResult) -> Unit,
) {
    var current by remember { mutableStateOf(draft) }
    var totals by remember { mutableStateOf<DraftTotals?>(null) }
    var findings by remember { mutableStateOf<List<Finding>>(emptyList()) }
    var error by remember { mutableStateOf<String?>(null) }
    var pickerOpen by remember { mutableStateOf(false) }
    var suppressing by remember { mutableStateOf<Finding?>(null) }
    var reason by remember { mutableStateOf("") }
    var focus by remember { mutableStateOf<DraftFocusRequest?>(null) }
    var busy by remember { mutableStateOf(false) }
    var issueFindings by remember { mutableStateOf<List<Finding>?>(null) }
    val scope = rememberCoroutineScope()

    // Tapping a finding reveals its control. A customer finding can only be
    // answered by choosing another customer, so it opens the picker instead of
    // focusing the button that opens it.
    fun moveTo(field: String) {
        val target = draftFocus(field, current.lines) ?: return
        if (target == DraftFocus.Customer) {
            pickerOpen = true
            return
        }
        focus = DraftFocusRequest(target, (focus?.token ?: 0) + 1)
    }

    // Save and refresh the preview 500 ms after the last edit.
    LaunchedEffect(current) {
        delay(500)
        val preview = withContext(Dispatchers.IO) {
            runCatching {
                saveDraft(current)
                draftTotals(current) to validateDraft(current)
            }
        }
        preview.onSuccess { (t, f) -> totals = t; findings = f }
            .onFailure { error = it.message ?: it.toString() }
    }

    // The issue attempt saves the draft itself, so an edit that is still inside
    // the debounce is what gets issued. The archive decides, and every finding
    // it returns is shown, including the ones this screen cannot answer.
    fun issue() {
        error = null
        issueFindings = null
        busy = true
        val attempt = current
        scope.launch {
            val result = withContext(Dispatchers.IO) { runCatching { issueDraft(attempt) } }
            busy = false
            result.onSuccess { outcome ->
                val issued = outcome.issued
                when {
                    issued != null -> onIssued(outcome)
                    outcome.findings.isNotEmpty() -> issueFindings = outcome.findings
                    else -> error = "The archive refused the document without a finding."
                }
            }.onFailure { error = it.message ?: it.toString() }
        }
    }

    Column(Modifier.fillMaxWidth()) {
        ScreenHeader("Draft", onBack) { }
        Box {
            Button(
                onClick = { pickerOpen = true; onInteraction() },
                modifier = Modifier.fillMaxWidth().revealOn(focus, DraftFocus.Customer, focus = false),
            ) {
                Text(customers.firstOrNull { it.id == current.customerId }?.name ?: "Choose a customer")
            }
            DropdownMenu(expanded = pickerOpen, onDismissRequest = { pickerOpen = false }) {
                customers.forEach { customer ->
                    DropdownMenuItem(
                        text = { Text(customer.name) },
                        onClick = {
                            current = current.copy(customerId = customer.id)
                            pickerOpen = false
                            onInteraction()
                        },
                    )
                }
            }
        }
        Spacer(Modifier.height(8.dp))
        FormField(
            "Issue date",
            current.issueDate,
            modifier = Modifier.revealOn(focus, DraftFocus.IssueDate),
        ) { current = current.copy(issueDate = it); onInteraction() }
        FormField(
            "Due date",
            current.dueDate,
            modifier = Modifier.revealOn(focus, DraftFocus.DueDate),
        ) { current = current.copy(dueDate = it); onInteraction() }
        FormField(
            "Note",
            current.note,
            modifier = Modifier.revealOn(focus, DraftFocus.Note),
        ) { current = current.copy(note = it); onInteraction() }
        FormField(
            "Buyer reference",
            current.buyerReference,
            modifier = Modifier.revealOn(focus, DraftFocus.BuyerReference),
        ) { current = current.copy(buyerReference = it); onInteraction() }
        FormField(
            "Order reference",
            current.orderReference,
            modifier = Modifier.revealOn(focus, DraftFocus.OrderReference),
        ) { current = current.copy(orderReference = it); onInteraction() }

        Spacer(Modifier.height(12.dp))
        Text("Lines", style = MaterialTheme.typography.titleMedium)
        current.lines.forEach { line ->
            DraftLineEditor(
                line = line,
                focus = focus,
                onChange = { updated ->
                    current = current.copy(lines = current.lines.map { if (it.id == line.id) updated else it })
                    onInteraction()
                },
                onRemove = {
                    current = current.copy(
                        lines = current.lines.filter { it.id != line.id }
                            .mapIndexed { index, remaining -> remaining.copy(position = index + 1) },
                    )
                    onInteraction()
                },
            )
        }
        TextButton(
            modifier = Modifier.revealOn(focus, DraftFocus.Lines),
            onClick = {
                current = current.copy(
                    lines = current.lines + DraftLine(
                        id = newId(),
                        position = current.lines.size + 1,
                        name = "",
                        quantityMicros = 1_000_000,
                        unitPriceMicros = 0,
                    ),
                )
                onInteraction()
            },
        ) { Text("Add line") }

        totals?.let {
            Spacer(Modifier.height(12.dp))
            Text("Net ${formatMoney(it.netMicros)} ${current.currency}", style = MaterialTheme.typography.bodyMedium)
            Text("VAT ${formatMoney(it.taxMicros)} ${current.currency}", style = MaterialTheme.typography.bodyMedium)
            Text(
                "Gross ${formatMoney(it.grossMicros)} ${current.currency}",
                style = MaterialTheme.typography.titleMedium,
            )
        }

        if (findings.isNotEmpty()) {
            Spacer(Modifier.height(12.dp))
            Text("Findings", style = MaterialTheme.typography.titleMedium)
            findings.forEach { finding ->
                FindingEntry(
                    finding = finding,
                    focusable = draftFocus(finding.field, current.lines) != null,
                    onFocus = { moveTo(finding.field) },
                ) {
                    if (!finding.blocking) {
                        TextButton(
                            onClick = {
                                suppressing = finding
                                reason = ""
                                onInteraction()
                            },
                        ) { Text("Suppress") }
                    }
                }
            }
        }

        Spacer(Modifier.height(16.dp))
        Button(
            onClick = ::issue,
            enabled = !busy,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text(
                when {
                    busy -> "Issuing..."
                    current.docType == "381" -> "Issue credit note"
                    else -> "Issue invoice"
                },
            )
        }
        error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
    }

    issueFindings?.let { blocked ->
        AlertDialog(
            onDismissRequest = { issueFindings = null },
            title = { Text("Not issued") },
            text = {
                Column(Modifier.verticalScroll(rememberScrollState())) {
                    val unfixable = blocked.count { draftFocus(it.field, current.lines) == null }
                    Text(
                        if (unfixable > 0) {
                            "The archive refused the document. $unfixable of these point at a field this screen does not edit."
                        } else {
                            "The archive refused the document."
                        },
                        style = MaterialTheme.typography.bodyMedium,
                    )
                    Spacer(Modifier.height(8.dp))
                    blocked.forEach { finding ->
                        FindingEntry(
                            finding = finding,
                            focusable = draftFocus(finding.field, current.lines) != null,
                            onFocus = {
                                issueFindings = null
                                moveTo(finding.field)
                            },
                        )
                    }
                }
            },
            confirmButton = {
                TextButton(onClick = { issueFindings = null }) { Text("Close") }
            },
        )
    }

    suppressing?.let { finding ->
        AlertDialog(
            onDismissRequest = { suppressing = null },
            title = { Text("Suppress ${finding.code}") },
            text = {
                Column {
                    Text(finding.message)
                    Spacer(Modifier.height(8.dp))
                    OutlinedTextField(
                        value = reason,
                        onValueChange = { reason = it },
                        label = { Text("Reason") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        val why = reason
                        scope.launch {
                            val result = withContext(Dispatchers.IO) {
                                runCatching {
                                    suppressFinding(current.id, finding.code, finding.field, why)
                                    validateDraft(current)
                                }
                            }
                            result.onSuccess {
                                findings = it
                                suppressing = null
                                error = null
                            }.onFailure { error = it.message ?: it.toString() }
                        }
                    },
                    enabled = reason.isNotBlank(),
                ) { Text("Suppress") }
            },
            dismissButton = {
                TextButton(onClick = { suppressing = null }) { Text("Cancel") }
            },
        )
    }
}

@Composable
private fun FindingEntry(
    finding: Finding,
    focusable: Boolean,
    onFocus: () -> Unit,
    action: @Composable () -> Unit = {},
) {
    val reference = listOf(finding.code, finding.bt).filter { it.isNotBlank() }.joinToString(" ")
    val tone = if (finding.blocking) {
        MaterialTheme.colorScheme.error
    } else {
        MaterialTheme.colorScheme.onSurfaceVariant
    }
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f).padding(vertical = 4.dp)) {
            Text(
                "${finding.severity}: $reference",
                style = MaterialTheme.typography.labelMedium,
                color = tone,
            )
            Text(finding.message, style = MaterialTheme.typography.bodySmall)
            if (finding.field.isNotBlank()) {
                if (focusable) {
                    AssistChip(
                        onClick = onFocus,
                        label = { Text(finding.field, style = MaterialTheme.typography.labelSmall) },
                    )
                } else {
                    Text(finding.field, style = MaterialTheme.typography.labelSmall, color = tone)
                }
            }
        }
        action()
    }
}

/** What the archive holds after a successful issue. */
@Composable
fun IssueResultScreen(result: IssueResult, onDone: () -> Unit) {
    val issued = result.issued ?: return
    var message by remember { mutableStateOf<String?>(null) }
    Column(Modifier.fillMaxWidth()) {
        ScreenHeader("Issued", onDone) { }
        Text(issued.number, style = MaterialTheme.typography.headlineMedium)
        Text(
            "Issued ${issued.issueDate}",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(12.dp))
        Text("Net ${formatMoney(issued.netMicros)} ${issued.currency}", style = MaterialTheme.typography.bodyMedium)
        Text("VAT ${formatMoney(issued.taxMicros)} ${issued.currency}", style = MaterialTheme.typography.bodyMedium)
        Text(
            "Gross ${formatMoney(issued.grossMicros)} ${issued.currency}",
            style = MaterialTheme.typography.titleMedium,
        )

        Spacer(Modifier.height(12.dp))
        Text("Filed artefacts", style = MaterialTheme.typography.titleMedium)
        if (result.ublSha256.isNotBlank()) {
            ArtefactBlock("UBL", issued.id, issued.number, "ubl", result.ublSha256) { message = it }
        }
        if (result.pdfSha256.isNotBlank()) {
            ArtefactBlock("PDF", issued.id, issued.number, "pdf", result.pdfSha256) { message = it }
        }
        message?.let { text ->
            Spacer(Modifier.height(4.dp))
            Text(text, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
        }
        if (result.pdfError.isNotBlank()) {
            Spacer(Modifier.height(8.dp))
            Text(
                "The PDF could not be filed: ${result.pdfError}",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.error,
            )
        }

        if (result.findings.isNotEmpty()) {
            Spacer(Modifier.height(12.dp))
            Text("Warnings", style = MaterialTheme.typography.titleMedium)
            result.findings.forEach { finding ->
                Text(
                    "${finding.code} ${finding.message}",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

/** One artefact's hash and the platform actions that hand it out. */
@Composable
private fun ArtefactBlock(
    label: String,
    documentId: String,
    number: String,
    kind: String,
    hash: String,
    onMessage: (String?) -> Unit,
) {
    Text(label, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
    Text(hash, style = MaterialTheme.typography.labelSmall)
    ArtefactActions(documentId, number, kind, onMessage)
    Spacer(Modifier.height(8.dp))
}

/**
 * Fetches the artefact when a button is pressed, so the screen never keeps the
 * bytes in composition state.
 */
@Composable
private fun ArtefactActions(
    documentId: String,
    number: String,
    kind: String,
    onMessage: (String?) -> Unit,
) {
    val io = rememberArtefactIo()
    val scope = rememberCoroutineScope()
    var busy by remember { mutableStateOf(false) }

    fun run(action: (ByteArray, String, String, (String) -> Unit) -> Unit) {
        if (busy) return
        busy = true
        onMessage(null)
        scope.launch {
            val bytes = withContext(Dispatchers.IO) { runCatching { documentArtefact(documentId, kind) } }
            busy = false
            bytes
                .onSuccess { action(it, artefactFileName(number, kind), artefactMime(kind), onMessage) }
                .onFailure { onMessage(it.message ?: it.toString()) }
        }
    }

    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        OutlinedButton(onClick = { run(io::share) }, enabled = !busy, modifier = Modifier.weight(1f)) {
            Text("Share")
        }
        OutlinedButton(onClick = { run(io::save) }, enabled = !busy, modifier = Modifier.weight(1f)) {
            Text("Save")
        }
        OutlinedButton(onClick = { run(io::open) }, enabled = !busy, modifier = Modifier.weight(1f)) {
            Text("Open")
        }
    }
    if (busy) {
        Text(
            "Preparing the file...",
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/** The MIME type the receiving app keys on. */
private fun artefactMime(kind: String): String = when (kind.lowercase()) {
    "pdf" -> "application/pdf"
    else -> "application/xml"
}

/** A name a person recognises and a file system accepts. */
private fun artefactFileName(number: String, kind: String): String {
    val extension = when (kind.lowercase()) {
        "pdf" -> "pdf"
        else -> "xml"
    }
    val stem = number.ifBlank { "document" }
        .map { character -> if (character.isLetterOrDigit() || character in "-._") character else '-' }
        .joinToString("")
    return "$stem.$extension"
}

/** Receiving one invoice: a file the user picks, or one another app handed over. */
@Composable
fun ImportScreen(
    onBack: () -> Unit,
    onOpen: (String) -> Unit,
    onInteraction: () -> Unit,
) {
    val picker = rememberDocumentPicker()
    val scope = rememberCoroutineScope()
    var report by remember { mutableStateOf<ImportReport?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var source by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }

    // A share or view intent waits until the archive is open; the screen takes
    // it when it arrives, including while this screen is already showing.
    val incoming = PendingImport.request

    fun receive(name: String, bytes: ByteArray) {
        if (busy) return
        busy = true
        error = null
        report = null
        source = name
        scope.launch {
            val result = withContext(Dispatchers.IO) { runCatching { importDocument(name, bytes) } }
            busy = false
            result.onSuccess { report = it }
                .onFailure { error = it.message ?: it.toString() }
        }
    }

    LaunchedEffect(incoming) {
        if (incoming != null) {
            PendingImport.take()
            receive(incoming.name, incoming.bytes)
        }
    }

    Column(Modifier.fillMaxWidth()) {
        ScreenHeader("Import", onBack) { }
        Text(
            "A received invoice arrives as UBL or PDF. The archive reads the sender, the number " +
                "and the totals, and refuses to store the same document twice.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(12.dp))
        Button(
            onClick = {
                onInteraction()
                picker.pick(::receive) { error = it }
            },
            enabled = !busy,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text(if (busy) "Importing..." else "Choose a file")
        }

        error?.let {
            Spacer(Modifier.height(8.dp))
            Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
        }

        val received = report
        if (received != null) {
            Spacer(Modifier.height(12.dp))
            Text("Received", style = MaterialTheme.typography.titleMedium)
            Fact("File", source)
            Fact("Number", received.number)
            Fact("Supplier", received.supplier.ifBlank { "Not matched to a customer" })
            if (received.duplicate) {
                Spacer(Modifier.height(4.dp))
                Text(
                    "The archive already held this document, so nothing was added.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Spacer(Modifier.height(8.dp))
            Button(onClick = { onOpen(received.documentId) }, modifier = Modifier.fillMaxWidth()) {
                Text("Open document")
            }
        }
    }
}

/** Backup, the recovery kit, and a restore from the two files they produce. */
@Composable
fun BackupScreen(
    onBack: () -> Unit,
    onRestored: (ArchiveSummary) -> Unit,
    onLocked: (String) -> Unit,
    onInteraction: () -> Unit,
) {
    val io = rememberBackupIo()
    val scope = rememberCoroutineScope()
    var busy by remember { mutableStateOf(false) }
    var message by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var staged by remember { mutableStateOf(false) }
    var passphrase by remember { mutableStateOf("") }
    var confirming by remember { mutableStateOf(false) }

    fun restore() {
        if (busy) return
        busy = true
        error = null
        message = null
        confirming = false
        scope.launch {
            val result = withContext(Dispatchers.IO) { runCatching { restoreArchive(passphrase) } }
            busy = false
            result.onSuccess { onRestored(it) }
                .onFailure { onLocked(it.message ?: it.toString()) }
        }
    }

    fun exportBackup() {
        if (busy) return
        busy = true
        error = null
        message = null
        onInteraction()
        scope.launch {
            val result = withContext(Dispatchers.IO) {
                runCatching {
                    // The copy is about the size of the archive, and SQLite writes
                    // through a temporary of its own while copying.
                    val needed = databaseBytes() * 2
                    val free = freeSpaceBytes()
                    check(free >= needed) {
                        "Not enough free space: a copy needs about ${megabytes(needed)}, and " +
                            "${megabytes(free)} is free."
                    }
                    writeBackup()
                }
            }
            busy = false
            result.onSuccess { export ->
                io.export(
                    path = export.path,
                    name = export.path.substringAfterLast('/'),
                    mime = "application/octet-stream",
                    onDone = { name -> message = "Saved $name (${kilobytes(export.bytes)} KB), verified." },
                    onError = { error = it },
                )
            }.onFailure { error = it.message ?: it.toString() }
        }
    }

    fun exportKit() {
        if (busy) return
        busy = true
        error = null
        message = null
        onInteraction()
        scope.launch {
            val result = withContext(Dispatchers.IO) { runCatching { recoveryKey() to recoveryNote() } }
            busy = false
            result.onSuccess { (key, note) ->
                io.exportKit(
                    key = key,
                    note = note,
                    onDone = { where ->
                        message = "Recovery kit written to $where. Keep it apart from the backups."
                    },
                    onError = { error = it },
                )
            }.onFailure { error = it.message ?: it.toString() }
        }
    }

    Column(Modifier.fillMaxWidth()) {
        ScreenHeader("Backup", onBack) { }
        Text(
            "The archive is two files: the database and the key that opens it. A backup holds the data, " +
                "the recovery kit holds the key, and neither is enough on its own.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        Spacer(Modifier.height(16.dp))
        Text("Backup", style = MaterialTheme.typography.titleMedium)
        Text(
            "A verified, encrypted copy. It is checked here before you choose where it goes, and the " +
                "app's own copy is deleted once it is written.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(8.dp))
        Button(onClick = ::exportBackup, enabled = !busy, modifier = Modifier.fillMaxWidth()) {
            Text(if (busy) "Working..." else "Export backup")
        }

        Spacer(Modifier.height(20.dp))
        Text("Recovery kit", style = MaterialTheme.typography.titleMedium)
        Text(
            "The key, with the note that explains what it is. Keep it on another medium from the " +
                "backups: anyone who has both this file and the passphrase has the archive.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(8.dp))
        Button(onClick = ::exportKit, enabled = !busy, modifier = Modifier.fillMaxWidth()) {
            Text("Export recovery kit")
        }

        Spacer(Modifier.height(20.dp))
        Text("Restore", style = MaterialTheme.typography.titleMedium)
        Text(
            "Replaces this device's archive with a backup and the key that opens it. Both files are " +
                "needed. The archive here now is kept beside the new one, so a wrong choice is recoverable.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(8.dp))
        OutlinedButton(
            onClick = {
                onInteraction()
                io.stagePair(
                    onDone = { staged = true; message = "Both files are ready. Enter the passphrase of that backup." },
                    onError = { error = it },
                )
            },
            enabled = !busy,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text(if (staged) "Choose the two files again" else "Choose the two files")
        }
        if (staged) {
            Spacer(Modifier.height(8.dp))
            OutlinedTextField(
                value = passphrase,
                onValueChange = { passphrase = it; onInteraction() },
                label = { Text("Passphrase of that backup") },
                singleLine = true,
                visualTransformation = PasswordVisualTransformation(),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                modifier = Modifier.fillMaxWidth(),
            )
            Spacer(Modifier.height(8.dp))
            Button(
                onClick = { confirming = true },
                enabled = !busy && passphrase.isNotBlank(),
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text(if (busy) "Restoring..." else "Replace this archive")
            }
        }

        message?.let {
            Spacer(Modifier.height(12.dp))
            Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.primary)
        }
        error?.let {
            Spacer(Modifier.height(12.dp))
            Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
        }
    }

    if (confirming) {
        AlertDialog(
            onDismissRequest = { confirming = false },
            title = { Text("Replace the archive?") },
            text = {
                Text(
                    "The archive on this device is replaced by the pair you picked. The files that " +
                        "are here now stay beside the new ones.",
                )
            },
            confirmButton = {
                TextButton(onClick = ::restore) { Text("Replace") }
            },
            dismissButton = {
                TextButton(onClick = { confirming = false }) { Text("Cancel") }
            },
        )
    }
}

/** Rounded up, so a message never says zero megabytes for a file that has bytes. */
private fun megabytes(bytes: Long): String = "${(bytes + 1_048_575) / 1_048_576} MB"

private fun kilobytes(bytes: Long): Long = (bytes + 1023) / 1024

/** The party invoices are issued by. */
@Composable
fun SellerEditScreen(onBack: () -> Unit, onInteraction: () -> Unit) {
    var form by remember { mutableStateOf<Seller?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var saved by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    LaunchedEffect(Unit) {
        val loaded = withContext(Dispatchers.IO) { runCatching { loadSeller() } }
        loaded.onSuccess { form = it }.onFailure { error = it.message ?: it.toString() }
    }

    // Each edit clears the confirmation, so it never stands for text that is not stored.
    fun edit(update: Seller.() -> Seller) {
        form = form?.update()
        saved = false
        onInteraction()
    }

    fun save() {
        val current = form ?: return
        busy = true
        error = null
        scope.launch {
            val result = withContext(Dispatchers.IO) { runCatching { saveSeller(current) } }
            busy = false
            result.onSuccess { form = it; saved = true }
                .onFailure { error = it.message ?: it.toString() }
        }
    }

    Column(Modifier.fillMaxWidth()) {
        ScreenHeader("Seller", onBack) { }
        val seller = form
        if (seller == null) {
            if (error == null) CircularProgressIndicator()
            error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            return@Column
        }
        Text(
            "Invoices are issued by this party. Conformance checks its address, VAT identifier and Peppol endpoint.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        FormField("Name", seller.name) { edit { copy(name = it) } }
        FormField("Street", seller.street) { edit { copy(street = it) } }
        FormField("Postal code", seller.postal) { edit { copy(postal = it) } }
        FormField("City", seller.city) { edit { copy(city = it) } }
        FormField("Country", seller.country) { edit { copy(country = it.uppercase()) } }
        FormField("VAT identifier", seller.vat) { edit { copy(vat = it) } }
        FormField("Chamber of commerce", seller.kvk) { edit { copy(kvk = it) } }
        FormField("Peppol endpoint", seller.peppol) { edit { copy(peppol = it) } }
        FormField("Contact", seller.contact) { edit { copy(contact = it) } }
        FormField("Email", seller.email) { edit { copy(email = it) } }
        FormField("Phone", seller.phone) { edit { copy(phone = it) } }
        error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        if (saved) {
            Spacer(Modifier.height(4.dp))
            Text("Saved.", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.primary)
        }
        Spacer(Modifier.height(12.dp))
        Button(onClick = ::save, enabled = !busy, modifier = Modifier.fillMaxWidth()) {
            Text(if (busy) "Saving..." else "Save seller")
        }
    }
}

/** The two sides of the books, as the register switches between them. */
private val DOCUMENT_SIDES = listOf(
    "outbound" to "Issued",
    "inbound" to "Received",
)

/** The statuses each side can stand in, as the filter offers them. */
private val DOCUMENT_FILTERS = mapOf(
    "outbound" to listOf(null to "All", "paid" to "Paid", "archived" to "Archived"),
    "inbound" to listOf(null to "All", "received" to "Received", "approved" to "Approved", "paid" to "Paid"),
)

/** What the register was showing, kept across a trip into a document. */
class DocumentRegisterState {
    var direction by mutableStateOf("outbound")
    var query by mutableStateOf("")
    var status by mutableStateOf<String?>(null)
}

/** Documents on one side of the books, newest first, with a search and filters. */
@Composable
fun DocumentListScreen(
    state: DocumentRegisterState,
    onBack: () -> Unit,
    onOpen: (String) -> Unit,
    onInteraction: () -> Unit,
) {
    var documents by remember { mutableStateOf<List<DocumentHit>>(emptyList()) }
    var error by remember { mutableStateOf<String?>(null) }
    var loading by remember { mutableStateOf(true) }
    val listState = rememberLazyListState()
    val filters = rememberScrollState()

    LaunchedEffect(state.query, state.status, state.direction) {
        if (state.query.isNotBlank()) delay(300)
        loading = true
        val result = withContext(Dispatchers.IO) {
            runCatching { listDocuments(state.query, state.status, state.direction) }
        }
        result.onSuccess { documents = it; error = null }
            .onFailure { error = it.message ?: it.toString() }
        loading = false
    }

    // A new result set starts at its best match, not where the last one was left.
    LaunchedEffect(documents) {
        if (documents.isNotEmpty()) listState.scrollToItem(0)
    }

    Column(Modifier.fillMaxSize()) {
        ScreenHeader("Documents", onBack) { }
        PrimaryTabRow(selectedTabIndex = DOCUMENT_SIDES.indexOfFirst { it.first == state.direction }) {
            DOCUMENT_SIDES.forEach { (code, label) ->
                Tab(
                    selected = state.direction == code,
                    onClick = {
                        if (state.direction != code) {
                            // The statuses differ per side, so a filter from one
                            // does not carry into the other.
                            state.direction = code
                            state.status = null
                            onInteraction()
                        }
                    },
                    text = { Text(label) },
                )
            }
        }
        OutlinedTextField(
            value = state.query,
            onValueChange = { state.query = it; onInteraction() },
            label = { Text("Search numbers, parties, references") },
            singleLine = true,
            supportingText = if (documents.isNotEmpty() && (state.query.isNotBlank() || state.status != null)) {
                {
                    Text(
                        if (documents.size == 1) "1 match" else "${documents.size} matches",
                        style = MaterialTheme.typography.labelMedium,
                    )
                }
            } else {
                null
            },
            modifier = Modifier.fillMaxWidth(),
        )
        Row(
            modifier = Modifier.fillMaxWidth().horizontalScroll(filters),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            DOCUMENT_FILTERS.getValue(state.direction).forEach { (code, label) ->
                FilterChip(
                    selected = state.status == code,
                    onClick = {
                        state.status = code
                        onInteraction()
                    },
                    label = { Text(label) },
                )
            }
        }
        Spacer(Modifier.height(8.dp))
        when {
            error != null -> Text(error!!, color = MaterialTheme.colorScheme.error)
            loading && documents.isEmpty() -> CircularProgressIndicator()
            documents.isEmpty() -> Text(
                when {
                    state.query.isNotBlank() -> "Nothing matches that search."
                    state.direction == "inbound" -> "No received invoices yet. Import one to file it here."
                    state.status != null -> "No ${state.status} documents yet."
                    else -> "No documents yet. Issue a draft and it is filed here."
                },
                style = MaterialTheme.typography.bodyMedium,
            )
            else -> {
                // A search answers with everything that matches; the list draws
                // only the rows on screen, so a wide match stays cheap.
                LazyColumn(
                    state = listState,
                    modifier = Modifier.weight(1f).fillMaxWidth(),
                    verticalArrangement = Arrangement.spacedBy(2.dp),
                ) {
                    items(documents, key = { it.id }) { hit ->
                        DocumentRow(hit) { onOpen(hit.id) }
                    }
                }
            }
        }
    }
}

/** One stored document: the facts, the lines, the artefacts and the history. */
@Composable
fun DocumentDetailScreen(id: String, onBack: () -> Unit) {
    var document by remember { mutableStateOf<DocumentDetail?>(null) }
    var customers by remember { mutableStateOf<List<Customer>>(emptyList()) }
    var error by remember { mutableStateOf<String?>(null) }
    var message by remember { mutableStateOf<String?>(null) }
    var artefactError by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    var matching by remember { mutableStateOf(false) }
    var refresh by remember { mutableStateOf(0) }
    val scope = rememberCoroutineScope()

    LaunchedEffect(id, refresh) {
        val result = withContext(Dispatchers.IO) { runCatching { loadDocument(id) to listCustomers() } }
        result.onSuccess { loaded ->
            document = loaded.first
            customers = loaded.second
            error = null
        }.onFailure { error = it.message ?: it.toString() }
    }

    fun match(customer: Customer) {
        val doc = document ?: return
        if (busy) return
        busy = true
        error = null
        scope.launch {
            val result = withContext(Dispatchers.IO) { runCatching { matchSupplier(doc.id, customer.id) } }
            busy = false
            result.onSuccess { message = "Supplier matched to ${customer.name}." }
                .onFailure { error = it.message ?: it.toString() }
            refresh++
        }
    }

    fun advance(target: String) {
        val doc = document ?: return
        if (busy) return
        busy = true
        error = null
        scope.launch {
            val result = withContext(Dispatchers.IO) { runCatching { advanceDocument(doc.id, target) } }
            busy = false
            result.onSuccess { message = "The document is now $it." }
                .onFailure { error = it.message ?: it.toString() }
            refresh++
        }
    }

    Column(Modifier.fillMaxWidth()) {
        ScreenHeader(document?.number ?: "Document", onBack) { }
        val doc = document
        if (doc == null) {
            error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            return@Column
        }

        // On what we issued the header is the customer; on what we received it is
        // the supplier who sent it, and our own name is the buyer.
        val inbound = doc.direction == "inbound"
        val supplier = customers.firstOrNull { it.id == doc.supplierId }

        Text(
            doc.status.uppercase(),
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(4.dp))
        Text(if (inbound) doc.seller else doc.buyer, style = MaterialTheme.typography.titleMedium)
        Fact("Issued", doc.issueDate)
        Fact("Due", doc.dueDate)
        if (inbound) {
            Fact("Supplier", supplier?.name ?: "Not matched")
            Fact("Buyer", doc.buyer)
        } else {
            Fact("Seller", doc.seller)
        }
        Fact("Buyer reference", doc.buyerReference)
        Fact("Order reference", doc.orderReference)
        Fact("Note", doc.note)

        if (inbound) {
            Spacer(Modifier.height(8.dp))
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedButton(onClick = { matching = true }, enabled = !busy, modifier = Modifier.weight(1f)) {
                    Text(if (supplier == null) "Match supplier" else "Change supplier")
                }
                val next = when (doc.status) {
                    "received" -> "approved" to "Approve"
                    "approved" -> "paid" to "Mark paid"
                    else -> null
                }
                if (next != null) {
                    Button(onClick = { advance(next.first) }, enabled = !busy, modifier = Modifier.weight(1f)) {
                        Text(next.second)
                    }
                }
            }
        }

        message?.let {
            Spacer(Modifier.height(4.dp))
            Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.primary)
        }
        error?.let {
            Spacer(Modifier.height(4.dp))
            Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
        }

        Spacer(Modifier.height(12.dp))
        Text("Lines", style = MaterialTheme.typography.titleMedium)
        doc.lines.forEach { line ->
            Column(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
                Text(line.name.ifBlank { "Line ${line.position}" }, style = MaterialTheme.typography.bodyMedium)
                Text(
                    "${line.quantity} ${line.unit} x ${formatMoney(line.unitPriceMicros)} ${doc.currency}" +
                        " · VAT ${formatRate(line.vatRateBp)}%",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Text(
                    "${formatMoney(line.lineNetMicros)} ${doc.currency}",
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        }

        Spacer(Modifier.height(12.dp))
        Text("Net ${formatMoney(doc.netMicros)} ${doc.currency}", style = MaterialTheme.typography.bodyMedium)
        Text("VAT ${formatMoney(doc.taxMicros)} ${doc.currency}", style = MaterialTheme.typography.bodyMedium)
        Text("Gross ${formatMoney(doc.grossMicros)} ${doc.currency}", style = MaterialTheme.typography.titleMedium)

        Spacer(Modifier.height(12.dp))
        Text("Filed artefacts", style = MaterialTheme.typography.titleMedium)
        if (doc.artefacts.isEmpty()) {
            Text(
                "None yet.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        doc.artefacts.forEach { artefact ->
            Text(
                "${artefact.kind.uppercase()} · ${artefact.sizeBytes} bytes",
                style = MaterialTheme.typography.bodySmall,
            )
            ArtefactBlock(artefact.kind.uppercase(), doc.id, doc.number, artefact.kind, artefact.bytesHash) {
                artefactError = it
            }
        }
        artefactError?.let { text ->
            Spacer(Modifier.height(4.dp))
            Text(text, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
        }

        if (doc.events.isNotEmpty()) {
            Spacer(Modifier.height(12.dp))
            Text("History", style = MaterialTheme.typography.titleMedium)
            doc.events.forEach { event ->
                Text(
                    "${event.kind} · ${event.at} · ${event.actor}" +
                        if (event.detail.isBlank()) "" else " · ${event.detail}",
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        }
    }

    if (matching) {
        AlertDialog(
            onDismissRequest = { matching = false },
            title = { Text("Match supplier") },
            text = {
                if (customers.isEmpty()) {
                    Text("No customers yet. Add one in Customers first.")
                } else {
                    LazyColumn(Modifier.heightIn(max = 360.dp)) {
                        items(customers, key = { it.id }) { customer ->
                            TextButton(
                                onClick = {
                                    matching = false
                                    match(customer)
                                },
                                modifier = Modifier.fillMaxWidth(),
                            ) {
                                Text(customer.name)
                            }
                        }
                    }
                }
            },
            confirmButton = {
                TextButton(onClick = { matching = false }) { Text("Cancel") }
            },
        )
    }
}

@Composable
private fun Fact(label: String, value: String) {
    if (value.isBlank()) return
    Row(Modifier.fillMaxWidth().padding(vertical = 2.dp)) {
        Text(
            label,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.width(120.dp),
        )
        Text(value, style = MaterialTheme.typography.bodySmall, modifier = Modifier.weight(1f))
    }
}

@Composable
private fun DocumentRow(hit: DocumentHit, onClick: () -> Unit) {
    TextButton(onClick = onClick, modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.fillMaxWidth()) {
            Text(hit.number.ifBlank { "No number" }, style = MaterialTheme.typography.titleMedium)
            val detail = buildList {
                add(hit.party)
                add(hit.issueDate)
                add("${formatMoney(hit.grossMicros)} ${hit.currency}")
                add(hit.status)
                if (hit.direction == "inbound" && !hit.matched) add("not matched")
            }
                .filter { it.isNotBlank() }
                .joinToString(" · ")
            Text(
                detail,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun DraftLineEditor(
    line: DraftLine,
    focus: DraftFocusRequest?,
    onChange: (DraftLine) -> Unit,
    onRemove: () -> Unit,
) {
    var name by remember(line.id) { mutableStateOf(line.name) }
    var quantity by remember(line.id) { mutableStateOf(formatInput(line.quantityMicros)) }
    var price by remember(line.id) {
        mutableStateOf(if (line.unitPriceMicros == 0L) "" else formatInput(line.unitPriceMicros))
    }
    var rate by remember(line.id) { mutableStateOf(formatRate(line.taxRateBp)) }
    var unit by remember(line.id) { mutableStateOf(line.unit) }

    Column(Modifier.fillMaxWidth().padding(vertical = 6.dp)) {
        OutlinedTextField(
            value = name,
            onValueChange = {
                name = it
                onChange(line.copy(name = it))
            },
            label = { Text("Item") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth().revealOn(focus, DraftFocus.Line(line.id, LineField.Name)),
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedTextField(
                value = quantity,
                onValueChange = {
                    quantity = it
                    parseMicros(it)?.let { micros -> onChange(line.copy(quantityMicros = micros)) }
                },
                label = { Text("Quantity") },
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
                modifier = Modifier
                    .weight(1f)
                    .revealOn(focus, DraftFocus.Line(line.id, LineField.Quantity)),
            )
            OutlinedTextField(
                value = unit,
                onValueChange = {
                    unit = it.uppercase()
                    onChange(line.copy(unit = it.uppercase()))
                },
                label = { Text("Unit") },
                singleLine = true,
                modifier = Modifier
                    .weight(1f)
                    .revealOn(focus, DraftFocus.Line(line.id, LineField.Unit)),
            )
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedTextField(
                value = price,
                onValueChange = {
                    price = it
                    parseMicros(it)?.let { micros -> onChange(line.copy(unitPriceMicros = micros)) }
                },
                label = { Text("Unit price") },
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
                modifier = Modifier
                    .weight(1f)
                    .revealOn(focus, DraftFocus.Line(line.id, LineField.UnitPrice)),
            )
            OutlinedTextField(
                value = rate,
                onValueChange = {
                    rate = it
                    parseBasisPoints(it)?.let { bp -> onChange(line.copy(taxRateBp = bp)) }
                },
                label = { Text("VAT %") },
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
                modifier = Modifier
                    .weight(1f)
                    .revealOn(focus, DraftFocus.Line(line.id, LineField.TaxRate)),
            )
        }
        TextButton(onClick = onRemove) { Text("Remove line") }
        HorizontalDivider()
    }
}

@Composable
private fun ScreenHeader(title: String, onBack: () -> Unit, action: @Composable () -> Unit) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        TextButton(onClick = onBack) { Text("Back") }
        Text(title, style = MaterialTheme.typography.titleLarge, modifier = Modifier.weight(1f))
        action()
    }
}

@Composable
private fun CustomerRow(customer: Customer, onClick: () -> Unit) {
    TextButton(onClick = onClick, modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.fillMaxWidth()) {
            Text(customer.name, style = MaterialTheme.typography.titleMedium)
            val detail = listOf(customer.city, customer.vat).filter { it.isNotBlank() }.joinToString(" · ")
            if (detail.isNotBlank()) {
                Text(detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}

@Composable
private fun DraftRow(draft: Draft, customerName: String, gross: String, onClick: () -> Unit) {
    TextButton(onClick = onClick, modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.fillMaxWidth()) {
            Text(customerName, style = MaterialTheme.typography.titleMedium)
            val detail = listOf(draft.issueDate, "${draft.lines.size} line(s)", gross)
                .filter { it.isNotBlank() }
                .joinToString(" · ")
            Text(detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

@Composable
private fun FormField(
    label: String,
    value: String,
    modifier: Modifier = Modifier,
    onChange: (String) -> Unit,
) {
    OutlinedTextField(
        value = value,
        onValueChange = onChange,
        label = { Text(label) },
        singleLine = true,
        modifier = modifier.fillMaxWidth().padding(vertical = 4.dp),
    )
}

private fun formatMoney(micros: Long): String {
    val negative = micros < 0
    val magnitude = abs(micros)
    val whole = magnitude / 1_000_000
    val cents = (magnitude % 1_000_000) / 10_000
    return (if (negative) "-" else "") + whole.toString() + "." + cents.toString().padStart(2, '0')
}

private fun parseMicros(text: String): Long? {
    val trimmed = text.trim().replace(',', '.')
    if (trimmed.isEmpty() || trimmed == "." || trimmed == "-") return null
    val negative = trimmed.startsWith('-')
    val body = if (negative) trimmed.substring(1) else trimmed
    val parts = body.split('.')
    if (parts.size > 2) return null
    val whole = parts[0].ifEmpty { "0" }
    if (whole.any { !it.isDigit() }) return null
    val fraction = (parts.getOrNull(1) ?: "").padEnd(6, '0').take(6)
    if (fraction.any { !it.isDigit() }) return null
    val value = whole.toLongOrNull()?.times(1_000_000)?.plus(fraction.toLongOrNull() ?: 0) ?: return null
    return if (negative) -value else value
}

private fun parseBasisPoints(text: String): Int? {
    val micros = parseMicros(text) ?: return null
    return (micros / 10_000).toInt()
}

private fun formatInput(micros: Long): String {
    val whole = micros / 1_000_000
    val fraction = (micros % 1_000_000).toString().padStart(6, '0').trimEnd('0')
    return if (fraction.isEmpty()) whole.toString() else "$whole.$fraction"
}

private fun formatRate(basisPoints: Int): String {
    val whole = basisPoints / 100
    val fraction = basisPoints % 100
    return if (fraction == 0) whole.toString() else "$whole.${fraction.toString().padStart(2, '0')}"
}
