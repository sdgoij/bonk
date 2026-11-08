package com.github.sdgoij.bonk

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
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
            Text("UBL", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Text(result.ublSha256, style = MaterialTheme.typography.labelSmall)
        }
        if (result.pdfSha256.isNotBlank()) {
            Text("PDF", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Text(result.pdfSha256, style = MaterialTheme.typography.labelSmall)
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
