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
import androidx.compose.material3.Surface
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
import androidx.compose.ui.platform.LocalDensity
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
    selectedId: String? = null,
    version: Int = 0,
) {
    var customers by remember { mutableStateOf<List<Customer>>(emptyList()) }
    var query by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    var report by remember { mutableStateOf<CustomerImportReport?>(null) }
    var loading by remember { mutableStateOf(true) }
    var refresh by remember { mutableStateOf(0) }
    val scope = rememberCoroutineScope()
    val csvIo = rememberCsvIo()

    LaunchedEffect(refresh, version) {
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
        ScreenHeader(strings.customers, onBack) { }
        OutlinedTextField(
            value = query,
            onValueChange = {
                query = it
                onInteraction()
            },
            label = { Text(strings.search) },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        Spacer(Modifier.height(8.dp))
        Button(onClick = { onInteraction(); onAdd() }, modifier = Modifier.fillMaxWidth()) {
            Text(strings.addCustomer)
        }
        Spacer(Modifier.height(8.dp))
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton(
                onClick = { onInteraction(); importCsv() },
                modifier = Modifier.weight(1f),
            ) { Text(strings.importCsv) }
            OutlinedButton(
                onClick = { onInteraction(); exportCsv() },
                modifier = Modifier.weight(1f),
            ) { Text(strings.exportCsv) }
        }
        report?.let {
            Spacer(Modifier.height(8.dp))
            Text(
                strings.importedSkipped(it.imported, it.skipped),
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
            Text(strings.noCustomers, modifier = Modifier.padding(16.dp))
        } else {
            visible.forEach { customer ->
                CustomerRow(customer, customer.id == selectedId) { onInteraction(); onEdit(customer) }
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
            error = strings.enterName
            return
        }
        if (form.city.isBlank()) {
            error = strings.enterCity
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
        ScreenHeader(if (customer.id.isEmpty()) strings.newCustomer else strings.editCustomer, onBack) { }
        FormField(strings.name, form.name) { form = form.copy(name = it); onInteraction() }
        FormField(strings.street, form.street) { form = form.copy(street = it); onInteraction() }
        FormField(strings.postal, form.postal) { form = form.copy(postal = it); onInteraction() }
        FormField(strings.city, form.city) { form = form.copy(city = it); onInteraction() }
        FormField(strings.country, form.country) { form = form.copy(country = it.uppercase()); onInteraction() }
        FormField(strings.vat, form.vat) { form = form.copy(vat = it); onInteraction() }
        FormField(strings.kvk, form.kvk) { form = form.copy(kvk = it); onInteraction() }
        FormField(strings.peppol, form.peppol) { form = form.copy(peppol = it); onInteraction() }
        FormField(strings.contact, form.contact) { form = form.copy(contact = it); onInteraction() }
        FormField(strings.email, form.email) { form = form.copy(email = it); onInteraction() }
        FormField(strings.phone, form.phone) { form = form.copy(phone = it); onInteraction() }
        error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        Spacer(Modifier.height(12.dp))
        Button(onClick = ::save, enabled = !busy, modifier = Modifier.fillMaxWidth()) {
            Text(if (busy) strings.saving else strings.saveCustomer)
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
            error = strings.addCustomerFirst
            return
        }
        scope.launch {
            val result = withContext(Dispatchers.IO) { runCatching { newDraft(customers.first().id) } }
            result.onSuccess { onOpen(it, customers) }.onFailure { error = it.message ?: it.toString() }
        }
    }

    Column(Modifier.fillMaxWidth()) {
        ScreenHeader(strings.drafts, onBack) {
            TextButton(onClick = { onInteraction(); onCustomers() }) { Text(strings.customers) }
        }
        Button(onClick = { onInteraction(); create() }, modifier = Modifier.fillMaxWidth()) {
            Text(strings.newDraft)
        }
        Spacer(Modifier.height(8.dp))
        if (loading) {
            CircularProgressIndicator(modifier = Modifier.padding(16.dp))
        } else if (drafts.isEmpty()) {
            Text(strings.noDrafts, modifier = Modifier.padding(16.dp))
        } else {
            drafts.forEach { draft ->
                DraftRow(
                    draft = draft,
                    customerName = customers.firstOrNull { it.id == draft.customerId }?.name ?: strings.noCustomer,
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
                    else -> error = strings.refusedWithoutFinding
                }
            }.onFailure { error = it.message ?: it.toString() }
        }
    }

    Column(Modifier.fillMaxWidth()) {
        ScreenHeader(strings.draft, onBack) { }
        Box {
            Button(
                onClick = { pickerOpen = true; onInteraction() },
                modifier = Modifier.fillMaxWidth().revealOn(focus, DraftFocus.Customer, focus = false),
            ) {
                Text(customers.firstOrNull { it.id == current.customerId }?.name ?: strings.chooseCustomer)
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
            strings.issueDate,
            current.issueDate,
            modifier = Modifier.revealOn(focus, DraftFocus.IssueDate),
        ) { current = current.copy(issueDate = it); onInteraction() }
        FormField(
            strings.dueDate,
            current.dueDate,
            modifier = Modifier.revealOn(focus, DraftFocus.DueDate),
        ) { current = current.copy(dueDate = it); onInteraction() }
        FormField(
            strings.note,
            current.note,
            modifier = Modifier.revealOn(focus, DraftFocus.Note),
        ) { current = current.copy(note = it); onInteraction() }
        FormField(
            strings.buyerReference,
            current.buyerReference,
            modifier = Modifier.revealOn(focus, DraftFocus.BuyerReference),
        ) { current = current.copy(buyerReference = it); onInteraction() }
        FormField(
            strings.orderReference,
            current.orderReference,
            modifier = Modifier.revealOn(focus, DraftFocus.OrderReference),
        ) { current = current.copy(orderReference = it); onInteraction() }

        Spacer(Modifier.height(12.dp))
        Text(strings.lines, style = MaterialTheme.typography.titleMedium)
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
        ) { Text(strings.addLine) }

        totals?.let {
            Spacer(Modifier.height(12.dp))
            Text(
                strings.netTotal("${formatMoney(it.netMicros)} ${current.currency}"),
                style = MaterialTheme.typography.bodyMedium,
            )
            Text(
                strings.vatTotal("${formatMoney(it.taxMicros)} ${current.currency}"),
                style = MaterialTheme.typography.bodyMedium,
            )
            Text(
                strings.grossTotal("${formatMoney(it.grossMicros)} ${current.currency}"),
                style = MaterialTheme.typography.titleMedium,
            )
        }

        if (findings.isNotEmpty()) {
            Spacer(Modifier.height(12.dp))
            Text(strings.findings, style = MaterialTheme.typography.titleMedium)
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
                        ) { Text(strings.suppress) }
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
                    busy -> strings.issuing
                    current.docType == "381" -> strings.issueCreditNote
                    else -> strings.issueInvoice
                },
            )
        }
        error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
    }

    issueFindings?.let { blocked ->
        AlertDialog(
            onDismissRequest = { issueFindings = null },
            title = { Text(strings.notIssued) },
            text = {
                Column(Modifier.verticalScroll(rememberScrollState())) {
                    val unfixable = blocked.count { draftFocus(it.field, current.lines) == null }
                    Text(
                        if (unfixable > 0) strings.refusedFields(unfixable) else strings.refused,
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
                TextButton(onClick = { issueFindings = null }) { Text(strings.close) }
            },
        )
    }

    suppressing?.let { finding ->
        AlertDialog(
            onDismissRequest = { suppressing = null },
            title = { Text(strings.suppressFinding(finding.code)) },
            text = {
                Column {
                    Text(finding.message)
                    Spacer(Modifier.height(8.dp))
                    OutlinedTextField(
                        value = reason,
                        onValueChange = { reason = it },
                        label = { Text(strings.reason) },
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
                ) { Text(strings.suppress) }
            },
            dismissButton = {
                TextButton(onClick = { suppressing = null }) { Text(strings.cancel) }
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
        ScreenHeader(strings.issuedTitle, onDone) { }
        Text(issued.number, style = MaterialTheme.typography.headlineMedium)
        Text(
            strings.issuedOn(issued.issueDate),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(12.dp))
        Text(
            strings.netTotal("${formatMoney(issued.netMicros)} ${issued.currency}"),
            style = MaterialTheme.typography.bodyMedium,
        )
        Text(
            strings.vatTotal("${formatMoney(issued.taxMicros)} ${issued.currency}"),
            style = MaterialTheme.typography.bodyMedium,
        )
        Text(
            strings.grossTotal("${formatMoney(issued.grossMicros)} ${issued.currency}"),
            style = MaterialTheme.typography.titleMedium,
        )

        Spacer(Modifier.height(12.dp))
        Text(strings.filedArtefacts, style = MaterialTheme.typography.titleMedium)
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
                strings.pdfNotFiled(result.pdfError),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.error,
            )
        }

        if (result.findings.isNotEmpty()) {
            Spacer(Modifier.height(12.dp))
            Text(strings.warnings, style = MaterialTheme.typography.titleMedium)
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
            Text(strings.share)
        }
        OutlinedButton(onClick = { run(io::save) }, enabled = !busy, modifier = Modifier.weight(1f)) {
            Text(strings.save)
        }
        OutlinedButton(onClick = { run(io::open) }, enabled = !busy, modifier = Modifier.weight(1f)) {
            Text(strings.open)
        }
    }
    if (busy) {
        Text(
            strings.preparingFile,
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
        ScreenHeader(strings.importTitle, onBack) { }
        Text(
            strings.importNote,
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
            Text(if (busy) strings.importing else strings.chooseFile)
        }

        error?.let {
            Spacer(Modifier.height(8.dp))
            Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
        }

        val received = report
        if (received != null) {
            Spacer(Modifier.height(12.dp))
            Text(strings.received, style = MaterialTheme.typography.titleMedium)
            Fact(strings.file, source)
            Fact(strings.number, received.number)
            Fact(strings.supplier, received.supplier.ifBlank { strings.notMatchedToCustomer })
            if (received.duplicate) {
                Spacer(Modifier.height(4.dp))
                Text(
                    strings.alreadyHeld,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Spacer(Modifier.height(8.dp))
            Button(onClick = { onOpen(received.documentId) }, modifier = Modifier.fillMaxWidth()) {
                Text(strings.openDocument)
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
                        strings.freeSpace(megabytes(needed), megabytes(free))
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
                    onDone = { name -> message = strings.savedVerified(name, kilobytes(export.bytes)) },
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
                    onDone = { where -> message = strings.kitWritten(where) },
                    onError = { error = it },
                )
            }.onFailure { error = it.message ?: it.toString() }
        }
    }

    Column(Modifier.fillMaxWidth()) {
        ScreenHeader(strings.backup, onBack) { }
        Text(
            strings.twoFiles,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        Spacer(Modifier.height(16.dp))
        Text(strings.backup, style = MaterialTheme.typography.titleMedium)
        Text(
            strings.verifiedCopy,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(8.dp))
        Button(onClick = ::exportBackup, enabled = !busy, modifier = Modifier.fillMaxWidth()) {
            Text(if (busy) strings.working else strings.exportBackup)
        }

        Spacer(Modifier.height(20.dp))
        Text(strings.recoveryKit, style = MaterialTheme.typography.titleMedium)
        Text(
            strings.recoveryKitNote,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(8.dp))
        Button(onClick = ::exportKit, enabled = !busy, modifier = Modifier.fillMaxWidth()) {
            Text(strings.exportKit)
        }

        Spacer(Modifier.height(20.dp))
        Text(strings.restore, style = MaterialTheme.typography.titleMedium)
        Text(
            strings.restoreNote,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(8.dp))
        OutlinedButton(
            onClick = {
                onInteraction()
                io.stagePair(
                    onDone = { staged = true; message = strings.bothFilesReady },
                    onError = { error = it },
                )
            },
            enabled = !busy,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text(if (staged) strings.chooseTwoAgain else strings.chooseTwo)
        }
        if (staged) {
            Spacer(Modifier.height(8.dp))
            OutlinedTextField(
                value = passphrase,
                onValueChange = { passphrase = it; onInteraction() },
                label = { Text(strings.backupPassphrase) },
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
                Text(if (busy) strings.restoring else strings.replaceArchive)
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
            title = { Text(strings.replaceQuestion) },
            text = {
                Text(strings.replaceNote)
            },
            confirmButton = {
                TextButton(onClick = ::restore) { Text(strings.replace) }
            },
            dismissButton = {
                TextButton(onClick = { confirming = false }) { Text(strings.cancel) }
            },
        )
    }
}

/** Rounded up, so a message never says zero megabytes for a file that has bytes. */
private fun megabytes(bytes: Long): String = "${(bytes + 1_048_575) / 1_048_576} MB"

private fun kilobytes(bytes: Long): Long = (bytes + 1023) / 1024

/** The archive's own settings: who issues, what documents print in, and the chain. */
@Composable
fun SettingsScreen(
    onBack: () -> Unit,
    onSeller: () -> Unit,
    onAddRecovery: () -> Unit,
    onInteraction: () -> Unit,
) {
    var seller by remember { mutableStateOf<Seller?>(null) }
    var chain by remember { mutableStateOf<ChainHead?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var saved by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    LaunchedEffect(Unit) {
        val loaded = withContext(Dispatchers.IO) { runCatching { loadSeller() to chainHead() } }
        loaded.onSuccess { pair ->
            seller = pair.first
            chain = pair.second
        }.onFailure { error = it.message ?: it.toString() }
    }

    fun chooseLocale(locale: String) {
        val current = seller ?: return
        if (busy || current.defaultLocale == locale) return
        busy = true
        error = null
        saved = false
        onInteraction()
        scope.launch {
            val result = withContext(Dispatchers.IO) {
                runCatching { saveSeller(current.copy(defaultLocale = locale)) }
            }
            busy = false
            result.onSuccess { seller = it; saved = true }
                .onFailure { error = it.message ?: it.toString() }
        }
    }

    fun chooseAppLanguage(language: Lang) {
        if (UiLanguage.current == language) return
        UiLanguage.current = language
        saveUiLanguageCode(language.code)
        saved = false
        onInteraction()
    }

    Column(Modifier.fillMaxWidth()) {
        ScreenHeader(strings.settings, onBack) { }

        Text(strings.appLanguage, style = MaterialTheme.typography.titleMedium)
        Text(
            strings.appLanguageNote,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(4.dp))
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            FilterChip(
                selected = UiLanguage.current == Lang.English,
                onClick = { chooseAppLanguage(Lang.English) },
                label = { Text(strings.english) },
            )
            FilterChip(
                selected = UiLanguage.current == Lang.Dutch,
                onClick = { chooseAppLanguage(Lang.Dutch) },
                label = { Text(strings.dutchLanguage) },
            )
        }

        Spacer(Modifier.height(20.dp))
        Text(strings.seller, style = MaterialTheme.typography.titleMedium)
        val current = seller
        if (current == null) {
            CircularProgressIndicator()
        } else {
            Text(current.name, style = MaterialTheme.typography.bodyMedium)
            Text(
                current.vat.ifBlank { strings.noVat },
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Spacer(Modifier.height(8.dp))
        OutlinedButton(onClick = onSeller, modifier = Modifier.fillMaxWidth()) {
            Text(strings.editSeller)
        }

        Spacer(Modifier.height(20.dp))
        Text(strings.documentLanguage, style = MaterialTheme.typography.titleMedium)
        Text(
            strings.documentLanguageNote,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(4.dp))
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            FilterChip(
                selected = current != null && !current.defaultLocale.startsWith("nl"),
                onClick = { chooseLocale("en") },
                label = { Text(strings.english) },
            )
            FilterChip(
                selected = current != null && current.defaultLocale.startsWith("nl"),
                onClick = { chooseLocale("nl-NL") },
                label = { Text(strings.dutchLanguage) },
            )
        }
        if (saved) {
            Text(strings.saved, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.primary)
        }

        Spacer(Modifier.height(20.dp))
        Text(strings.recovery, style = MaterialTheme.typography.titleMedium)
        Text(
            strings.recoveryNote,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(8.dp))
        OutlinedButton(onClick = onAddRecovery, modifier = Modifier.fillMaxWidth()) {
            Text(strings.addRecoveryKey)
        }

        Spacer(Modifier.height(20.dp))
        Text(strings.auditChain, style = MaterialTheme.typography.titleMedium)
        val head = chain
        if (head == null) {
            Text(
                strings.noEvents,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        } else {
            Fact(strings.events, head.seq.toString())
            Fact(strings.recorded, head.at)
            Spacer(Modifier.height(4.dp))
            Text(head.hash, style = MaterialTheme.typography.labelSmall)
        }

        error?.let {
            Spacer(Modifier.height(12.dp))
            Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
        }
    }
}

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
        ScreenHeader(strings.seller, onBack) { }
        val seller = form
        if (seller == null) {
            if (error == null) CircularProgressIndicator()
            error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            return@Column
        }
        Text(
            strings.sellerNote,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        FormField(strings.name, seller.name) { edit { copy(name = it) } }
        FormField(strings.street, seller.street) { edit { copy(street = it) } }
        FormField(strings.postal, seller.postal) { edit { copy(postal = it) } }
        FormField(strings.city, seller.city) { edit { copy(city = it) } }
        FormField(strings.country, seller.country) { edit { copy(country = it.uppercase()) } }
        FormField(strings.vat, seller.vat) { edit { copy(vat = it) } }
        FormField(strings.kvk, seller.kvk) { edit { copy(kvk = it) } }
        FormField(strings.peppol, seller.peppol) { edit { copy(peppol = it) } }
        FormField(strings.contact, seller.contact) { edit { copy(contact = it) } }
        FormField(strings.email, seller.email) { edit { copy(email = it) } }
        FormField(strings.phone, seller.phone) { edit { copy(phone = it) } }
        error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        if (saved) {
            Spacer(Modifier.height(4.dp))
            Text(strings.saved, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.primary)
        }
        Spacer(Modifier.height(12.dp))
        Button(onClick = ::save, enabled = !busy, modifier = Modifier.fillMaxWidth()) {
            Text(if (busy) strings.saving else strings.saveSeller)
        }
    }
}

/** The two sides of the books, as the register switches between them. */
private fun documentSides(): List<Pair<String, String>> = listOf(
    "outbound" to strings.issuedTab,
    "inbound" to strings.receivedTab,
)

/** The statuses each side can stand in, as the filter offers them. */
private fun documentFilters(): Map<String, List<Pair<String?, String>>> = mapOf(
    "outbound" to listOf(null to strings.allFilter, "paid" to strings.paid, "archived" to strings.archived),
    "inbound" to listOf(
        null to strings.allFilter,
        "received" to strings.receivedTab,
        "approved" to strings.approved,
        "paid" to strings.paid,
    ),
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
    selectedId: String? = null,
    version: Int = 0,
) {
    var documents by remember { mutableStateOf<List<DocumentHit>>(emptyList()) }
    var error by remember { mutableStateOf<String?>(null) }
    var loading by remember { mutableStateOf(true) }
    val listState = rememberLazyListState()
    val filters = rememberScrollState()

    LaunchedEffect(state.query, state.status, state.direction, version) {
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

    val sides = documentSides()
    val statusFilters = documentFilters()

    Column(Modifier.fillMaxSize()) {
        ScreenHeader(strings.documents, onBack) { }
        PrimaryTabRow(selectedTabIndex = sides.indexOfFirst { it.first == state.direction }) {
            sides.forEach { (code, label) ->
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
            label = { Text(strings.searchDocuments) },
            singleLine = true,
            supportingText = if (documents.isNotEmpty() && (state.query.isNotBlank() || state.status != null)) {
                {
                    Text(
                        strings.matches(documents.size),
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
            statusFilters.getValue(state.direction).forEach { (code, label) ->
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
                    state.query.isNotBlank() -> strings.nothingMatches
                    state.direction == "inbound" -> strings.noReceived
                    state.status != null -> strings.noStatusDocuments(strings.status(state.status!!))
                    else -> strings.noDocuments
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
                        DocumentRow(hit, hit.id == selectedId) { onOpen(hit.id) }
                    }
                }
            }
        }
    }
}

/** One stored document: the facts, the lines, the artefacts and the history. */
@Composable
fun DocumentDetailScreen(
    id: String,
    onBack: () -> Unit,
    onChanged: () -> Unit = {},
) {
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
            result.onSuccess { message = strings.supplierMatched(customer.name) }
                .onFailure { error = it.message ?: it.toString() }
            if (result.isSuccess) onChanged()
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
            result.onSuccess { message = strings.nowStatus(it) }
                .onFailure { error = it.message ?: it.toString() }
            if (result.isSuccess) onChanged()
            refresh++
        }
    }

    Column(Modifier.fillMaxWidth()) {
        ScreenHeader(document?.number ?: strings.document, onBack) { }
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
            strings.status(doc.status).uppercase(),
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(4.dp))
        Text(if (inbound) doc.seller else doc.buyer, style = MaterialTheme.typography.titleMedium)
        Fact(strings.issuedFact, doc.issueDate)
        Fact(strings.dueFact, doc.dueDate)
        if (inbound) {
            Fact(strings.supplier, supplier?.name ?: strings.notMatchedTitle)
            Fact(strings.buyer, doc.buyer)
        } else {
            Fact(strings.seller, doc.seller)
        }
        Fact(strings.buyerReference, doc.buyerReference)
        Fact(strings.orderReference, doc.orderReference)
        Fact(strings.note, doc.note)

        if (inbound) {
            Spacer(Modifier.height(8.dp))
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedButton(onClick = { matching = true }, enabled = !busy, modifier = Modifier.weight(1f)) {
                    Text(if (supplier == null) strings.matchSupplier else strings.changeSupplier)
                }
                val next = when (doc.status) {
                    "received" -> "approved" to strings.approve
                    "approved" -> "paid" to strings.markPaid
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
        Text(strings.lines, style = MaterialTheme.typography.titleMedium)
        doc.lines.forEach { line ->
            Column(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
                Text(line.name.ifBlank { strings.line(line.position) }, style = MaterialTheme.typography.bodyMedium)
                Text(
                    strings.lineDetail(
                        line.quantity,
                        line.unit,
                        formatMoney(line.unitPriceMicros),
                        doc.currency,
                        formatRate(line.vatRateBp),
                    ),
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
        Text(
            strings.netTotal("${formatMoney(doc.netMicros)} ${doc.currency}"),
            style = MaterialTheme.typography.bodyMedium,
        )
        Text(
            strings.vatTotal("${formatMoney(doc.taxMicros)} ${doc.currency}"),
            style = MaterialTheme.typography.bodyMedium,
        )
        Text(
            strings.grossTotal("${formatMoney(doc.grossMicros)} ${doc.currency}"),
            style = MaterialTheme.typography.titleMedium,
        )

        Spacer(Modifier.height(12.dp))
        Text(strings.filedArtefacts, style = MaterialTheme.typography.titleMedium)
        if (doc.artefacts.isEmpty()) {
            Text(
                strings.noneYet,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        doc.artefacts.forEach { artefact ->
            Text(
                strings.artefactLine(artefact.kind.uppercase(), artefact.sizeBytes),
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
            Text(strings.history, style = MaterialTheme.typography.titleMedium)
            doc.events.forEach { event ->
                Text(
                    strings.event(event.kind, event.at, event.actor, event.detail),
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        }
    }

    if (matching) {
        AlertDialog(
            onDismissRequest = { matching = false },
            title = { Text(strings.matchSupplier) },
            text = {
                if (customers.isEmpty()) {
                    Text(strings.noCustomersFirst)
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
                TextButton(onClick = { matching = false }) { Text(strings.cancel) }
            },
        )
    }
}

@Composable
private fun Fact(label: String, value: String) {
    if (value.isBlank()) return
    // At a large font the label no longer fits a fixed column, so the pair
    // stacks instead of clipping.
    if (LocalDensity.current.fontScale >= 1.5f) {
        Column(Modifier.fillMaxWidth().padding(vertical = 2.dp)) {
            Text(
                label,
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Text(value, style = MaterialTheme.typography.bodyMedium)
        }
        return
    }
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
private fun DocumentRow(hit: DocumentHit, selected: Boolean, onClick: () -> Unit) {
    val number = hit.number.ifBlank { strings.noNumber }
    val detail = buildList {
        add(hit.party)
        add(hit.issueDate)
        add("${formatMoney(hit.grossMicros)} ${hit.currency}")
        add(strings.status(hit.status))
        if (hit.direction == "inbound" && !hit.matched) add(strings.notMatched)
    }
        .filter { it.isNotBlank() }
        .joinToString(" · ")
    Surface(
        color = if (selected) MaterialTheme.colorScheme.secondaryContainer else MaterialTheme.colorScheme.surface,
        modifier = Modifier.fillMaxWidth(),
    ) {
        TextButton(onClick = onClick, modifier = Modifier.fillMaxWidth()) {
            Column(Modifier.fillMaxWidth()) {
                Text(number, style = MaterialTheme.typography.titleMedium)
                Text(
                    detail,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
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
            label = { Text(strings.item) },
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
                label = { Text(strings.quantity) },
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
                label = { Text(strings.unit) },
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
                label = { Text(strings.unitPrice) },
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
                label = { Text(strings.vatRate) },
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
                modifier = Modifier
                    .weight(1f)
                    .revealOn(focus, DraftFocus.Line(line.id, LineField.TaxRate)),
            )
        }
        TextButton(onClick = onRemove) { Text(strings.removeLine) }
        HorizontalDivider()
    }
}

@Composable
private fun ScreenHeader(title: String, onBack: () -> Unit, action: @Composable () -> Unit) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        TextButton(onClick = onBack) { Text(strings.back) }
        Text(title, style = MaterialTheme.typography.titleLarge, modifier = Modifier.weight(1f))
        action()
    }
}

@Composable
private fun CustomerRow(customer: Customer, selected: Boolean, onClick: () -> Unit) {
    val detail = listOf(customer.city, customer.vat).filter { it.isNotBlank() }.joinToString(" · ")
    Surface(
        color = if (selected) MaterialTheme.colorScheme.secondaryContainer else MaterialTheme.colorScheme.surface,
        modifier = Modifier.fillMaxWidth(),
    ) {
        TextButton(onClick = onClick, modifier = Modifier.fillMaxWidth()) {
            Column(Modifier.fillMaxWidth()) {
                Text(customer.name, style = MaterialTheme.typography.titleMedium)
                if (detail.isNotBlank()) {
                    Text(
                        detail,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
    }
}

@Composable
private fun DraftRow(draft: Draft, customerName: String, gross: String, onClick: () -> Unit) {
    val detail = listOf(draft.issueDate, strings.linesCount(draft.lines.size), gross)
        .filter { it.isNotBlank() }
        .joinToString(" · ")
    Box(Modifier.fillMaxWidth()) {
        TextButton(onClick = onClick, modifier = Modifier.fillMaxWidth()) {
            Column(Modifier.fillMaxWidth()) {
                Text(customerName, style = MaterialTheme.typography.titleMedium)
                Text(
                    detail,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
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
