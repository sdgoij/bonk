package com.github.sdgoij.bonk

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.VerticalDivider
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * S2/S3 boundary: the Android actual calls Rust through UniFFI and JNA. A future
 * iOS actual calls the Swift bindings instead, so `commonMain` stays free of
 * platform FFI details.
 */
expect fun greet(name: String): String

/** Runs the Rust-driven Keystore probe; the Android actual shows a prompt. */
expect fun runKeystoreTest(): String

/** Times the keystore's Argon2id parameters (64 MiB, t=3, p=1). */
expect fun runArgon2Benchmark(): String

/** Hashes the fixed sample invoice's UBL and PDF output. */
expect fun runArtefactHashes(): String

/** Locks the archive when the app leaves the foreground. */
@Composable
expect fun LockOnBackground(onLock: () -> Unit)

private sealed interface Screen {
    data object Checking : Screen
    data object Create : Screen
    data object Unlock : Screen
    data object Reset : Screen
    data object AddRecovery : Screen
    data class RecoveryKey(val key: String) : Screen
    data object Unlocked : Screen
    data object Customers : Screen
    data class CustomerEdit(val customer: Customer) : Screen
    data object Drafts : Screen
    data class DraftEditor(val draft: Draft, val customers: List<Customer>) : Screen
    data class Issued(val result: IssueResult) : Screen
    data object Documents : Screen
    data class DocumentView(val id: String) : Screen
    data object Import : Screen
    data object Backup : Screen
    data object Settings : Screen
    data object Seller : Screen
}

/** True while the archive is still closed, so a hand-off has to wait. */
private fun Screen.waitsForUnlock(): Boolean = when (this) {
    Screen.Checking, Screen.Create, Screen.Unlock, Screen.Reset, Screen.AddRecovery, is Screen.RecoveryKey -> true
    else -> false
}

private fun Screen.isArchiveContent(): Boolean = when (this) {
    Screen.Unlocked,
    Screen.AddRecovery,
    Screen.Customers,
    Screen.Drafts,
    is Screen.CustomerEdit,
    is Screen.DraftEditor,
    is Screen.Issued,
    Screen.Documents,
    is Screen.DocumentView,
    Screen.Seller,
    -> true

    else -> false
}

@Composable
fun App() {
    var screen by remember { mutableStateOf<Screen>(Screen.Checking) }
    var summary by remember { mutableStateOf(ArchiveSummary(0, 0, 0)) }
    var passphrase by remember { mutableStateOf("") }
    var confirmation by remember { mutableStateOf("") }
    var recoveryKey by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    var notice by remember { mutableStateOf<String?>(null) }
    var deviceAvailable by remember { mutableStateOf(false) }
    var rememberOnCreate by remember { mutableStateOf(false) }
    var rememberSupported by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var interactions by remember { mutableStateOf(0) }
    val scope = rememberCoroutineScope()
    // Null on anything that does not fold, which keeps every other layout as it was.
    val fold = rememberFoldGap()

    fun touch() {
        interactions++
    }

    // A wide window keeps the list beside the thing it opens, so the selection
    // needs a home outside the screen stack. The counters refresh the list when
    // its neighbour changes something the list shows.
    var selectedCustomer by remember { mutableStateOf<Customer?>(null) }
    var selectedDocument by remember { mutableStateOf<String?>(null) }
    var customersVersion by remember { mutableStateOf(0) }
    var documentsVersion by remember { mutableStateOf(0) }

    fun forgetSelection() {
        selectedCustomer = null
        selectedDocument = null
    }

    LaunchedEffect(Unit) {
        rememberSupported = runCatching { deviceUnlockAvailable() }.getOrDefault(false)
        val state = withContext(Dispatchers.IO) {
            val exists = runCatching { archiveExists() }.getOrDefault(false)
            val device = exists && runCatching { deviceReady() }.getOrDefault(false)
            exists to device
        }
        deviceAvailable = state.second
        screen = if (state.first) Screen.Unlock else Screen.Create
    }

    // Lock five minutes after the last interaction while the archive is open. A
    // configuration change also closes the handle through the activity's
    // onDestroy, and LockOnBackground covers leaving the app. A platform file
    // picker suspends both, so an export or import keeps the archive open.
    LaunchedEffect(screen, interactions) {
        if (screen.isArchiveContent()) {
            delay(5 * 60 * 1000L)
            if (!PickerGuard.active) {
                lockArchive()
                passphrase = ""
                confirmation = ""
                recoveryKey = ""
                forgetSelection()
                screen = Screen.Unlock
            }
        }
    }

    LockOnBackground {
        if (!PickerGuard.active && (screen.isArchiveContent() || screen is Screen.RecoveryKey)) {
            lockArchive()
            passphrase = ""
            confirmation = ""
            recoveryKey = ""
            error = null
            forgetSelection()
            screen = Screen.Unlock
        }
    }

    fun fail(message: Throwable) {
        error = message.message ?: message.toString()
        busy = false
    }

    fun unlock() {
        if (passphrase.isEmpty()) {
            error = strings.enterPassphrase
            return
        }
        busy = true
        error = null
        scope.launch {
            val result = withContext(Dispatchers.IO) {
                runCatching {
                    val summary = openArchive(passphrase)
                    // A revoked device key is recorded after the archive opens; a
                    // failure here must not turn a good unlock into an error.
                    runCatching {
                        if (takeDeviceRevoked()) recordDeviceRevocation()
                    }
                    summary
                }
            }
            result.onSuccess {
                passphrase = ""
                confirmation = ""
                summary = it
                screen = Screen.Unlocked
            }.onFailure(::fail)
            busy = false
        }
    }

    fun create() {
        if (passphrase.isEmpty()) {
            error = strings.choosePassphrase
            return
        }
        if (passphrase != confirmation) {
            error = strings.passphrasesDiffer
            return
        }
        val chosen = passphrase
        busy = true
        error = null
        notice = null
        scope.launch {
            val result = withContext(Dispatchers.IO) { runCatching { createArchive(chosen) } }
            if (result.isSuccess) {
                if (rememberOnCreate) {
                    val remembered = withContext(Dispatchers.IO) { runCatching { rememberDevice(chosen) } }
                    if (remembered.isSuccess) {
                        deviceAvailable = true
                    } else {
                        notice = strings.notRemembered +
                            (remembered.exceptionOrNull()?.message ?: strings.unknownError)
                    }
                }
                val created = result.getOrThrow()
                summary = created
                val recovery = withContext(Dispatchers.IO) { runCatching { addRecoveryKey(chosen) } }
                passphrase = ""
                confirmation = ""
                screen = if (recovery.isSuccess) {
                    Screen.RecoveryKey(recovery.getOrThrow())
                } else {
                    notice = strings.noRecoveryMade +
                        (recovery.exceptionOrNull()?.message ?: strings.unknownError)
                    Screen.Unlocked
                }
            } else {
                fail(result.exceptionOrNull() ?: IllegalStateException(strings.unknownError))
            }
            busy = false
        }
    }

    fun reset() {
        if (recoveryKey.isBlank()) {
            error = strings.enterRecoveryKey
            return
        }
        if (passphrase.isEmpty()) {
            error = strings.chooseNewPassphrase
            return
        }
        if (passphrase != confirmation) {
            error = strings.newPassphrasesDiffer
            return
        }
        val key = recoveryKey
        val phrase = passphrase
        busy = true
        error = null
        scope.launch {
            val result = withContext(Dispatchers.IO) { runCatching { resetPassphrase(key, phrase) } }
            result.onSuccess {
                recoveryKey = ""
                passphrase = ""
                confirmation = ""
                summary = it.summary
                screen = Screen.RecoveryKey(it.replacementKey)
            }.onFailure(::fail)
            busy = false
        }
    }

    fun addRecovery() {
        if (passphrase.isEmpty()) {
            error = strings.enterCurrentPassphrase
            return
        }
        if (screen !is Screen.AddRecovery) return
        val phrase = passphrase
        busy = true
        error = null
        scope.launch {
            val result = withContext(Dispatchers.IO) { runCatching { addRecoveryKey(phrase) } }
            result.onSuccess {
                passphrase = ""
                screen = Screen.RecoveryKey(it)
            }.onFailure(::fail)
            busy = false
        }
    }

    fun unlockWithDevice() {
        busy = true
        error = null
        scope.launch {
            val result = withContext(Dispatchers.IO) { runCatching { openArchiveWithDevice() } }
            if (result.isSuccess) {
                passphrase = ""
                confirmation = ""
                summary = result.getOrThrow()
                screen = Screen.Unlocked
            } else {
                fail(result.exceptionOrNull() ?: IllegalStateException(strings.unknownError))
                // A Keystore invalidation deletes the wrapped entry. If it is
                // gone, stop offering the route and say so.
                deviceAvailable = withContext(Dispatchers.IO) { runCatching { deviceReady() }.getOrDefault(false) }
                if (!deviceAvailable) {
                    error = strings.deviceKeyInvalid
                    runCatching { markDeviceRevoked() }
                }
            }
            busy = false
        }
    }

    fun forgetDeviceKey() {
        notice = null
        scope.launch {
            val result = withContext(Dispatchers.IO) { runCatching { forgetDevice() } }
            result.onSuccess {
                deviceAvailable = false
                notice = if (it) strings.deviceForgotten else strings.noDeviceKey
            }.onFailure { notice = it.message ?: it.toString() }
        }
    }

    // A document another app handed over waits for the archive to open.
    val pendingImport = PendingImport.request
    LaunchedEffect(screen, pendingImport) {
        if (pendingImport != null && !screen.waitsForUnlock()) {
            screen = Screen.Import
        }
    }

    // The register keeps its side and filters while a document is open.
    val register = remember { DocumentRegisterState() }

    BonkTheme {
        Surface(modifier = Modifier.fillMaxSize()) {
            BoxWithConstraints(modifier = Modifier.fillMaxSize()) {
                val width = WindowWidth.of(maxWidth)
                val windowWidthDp = maxWidth.value
                val wide = width.isAtLeastMedium
                // A list that keeps its detail beside it, and the register's lazy
                // rows, need a bounded height; the other screens scroll as a column.
                val panes = wide && (
                    screen is Screen.Customers || screen is Screen.CustomerEdit ||
                        screen is Screen.Documents || screen is Screen.DocumentView
                    )
                val contentWidth = when (width) {
                    WindowWidth.Compact -> if (maxWidth > 720.dp) 560.dp else maxWidth
                    WindowWidth.Medium -> 720.dp
                    WindowWidth.Expanded -> 960.dp
                }
                val tall = maxHeight > 800.dp
                val virtualised = screen is Screen.Documents || panes ||
                    (width.isExpanded && screen is Screen.DraftEditor)
                val scroll = rememberScrollState()
                CompositionLocalProvider(LocalWindowWidth provides width) {
                    Row(Modifier.fillMaxSize()) {
                        if (wide && screen.usesShell()) {
                            NavigationRailPanel(screen.destination()) { destination ->
                                forgetSelection()
                                screen = destination.toScreen()
                                touch()
                            }
                            VerticalDivider()
                        }
                        Column(
                            modifier = Modifier
                                .weight(1f)
                                .fillMaxHeight()
                                .imePadding()
                                .then(if (virtualised) Modifier else Modifier.verticalScroll(scroll))
                                .padding(24.dp),
                            horizontalAlignment = Alignment.CenterHorizontally,
                        ) {
                            Spacer(Modifier.height(if (tall) 72.dp else 16.dp))
                            Text("Bonk", style = MaterialTheme.typography.headlineLarge)
                            Spacer(Modifier.height(8.dp))
                            Text(
                                text = strings.tagline,
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                            Spacer(Modifier.height(24.dp))

                            val card = Modifier.widthIn(max = contentWidth).fillMaxWidth()
                            Card(modifier = if (virtualised) card.weight(1f) else card) {
                                Column(
                                    modifier = if (virtualised) {
                                        Modifier.fillMaxSize().padding(24.dp)
                                    } else {
                                        Modifier.padding(24.dp)
                                    },
                                ) {
                                    when (val current = screen) {
                                        Screen.Checking -> {
                                            Row(
                                                modifier = Modifier.fillMaxWidth(),
                                                horizontalArrangement = Arrangement.Center,
                                            ) {
                                                CircularProgressIndicator()
                                            }
                                        }
                                        Screen.Create -> CreateForm(
                                            passphrase = passphrase,
                                            confirmation = confirmation,
                                            remember = rememberOnCreate,
                                            rememberSupported = rememberSupported,
                                            busy = busy,
                                            error = error,
                                            onPassphrase = {
                                                passphrase = it
                                                error = null
                                                touch()
                                            },
                                            onConfirmation = {
                                                confirmation = it
                                                error = null
                                                touch()
                                            },
                                            onRemember = {
                                                rememberOnCreate = it
                                                touch()
                                            },
                                            onSubmit = ::create,
                                        )
                                        Screen.Unlock -> UnlockForm(
                                            passphrase = passphrase,
                                            deviceAvailable = deviceAvailable,
                                            busy = busy,
                                            error = error,
                                            onPassphrase = {
                                                passphrase = it
                                                error = null
                                                touch()
                                            },
                                            onSubmit = ::unlock,
                                            onUnlockWithDevice = ::unlockWithDevice,
                                            onForgot = {
                                                recoveryKey = ""
                                                passphrase = ""
                                                confirmation = ""
                                                error = null
                                                screen = Screen.Reset
                                            },
                                        )
                                        Screen.Reset -> ResetForm(
                                            recoveryKey = recoveryKey,
                                            passphrase = passphrase,
                                            confirmation = confirmation,
                                            busy = busy,
                                            error = error,
                                            onRecoveryKey = {
                                                recoveryKey = it
                                                error = null
                                                touch()
                                            },
                                            onPassphrase = {
                                                passphrase = it
                                                error = null
                                                touch()
                                            },
                                            onConfirmation = {
                                                confirmation = it
                                                error = null
                                                touch()
                                            },
                                            onSubmit = ::reset,
                                            onBack = {
                                                recoveryKey = ""
                                                passphrase = ""
                                                confirmation = ""
                                                error = null
                                                screen = Screen.Unlock
                                            },
                                        )
                                        is Screen.AddRecovery -> AddRecoveryForm(
                                            passphrase = passphrase,
                                            busy = busy,
                                            error = error,
                                            onPassphrase = {
                                                passphrase = it
                                                error = null
                                                touch()
                                            },
                                            onSubmit = ::addRecovery,
                                            onBack = {
                                                passphrase = ""
                                                error = null
                                                screen = Screen.Unlocked
                                            },
                                        )
                                        is Screen.RecoveryKey -> RecoveryKeyShown(
                                            key = current.key,
                                            notice = notice,
                                            onDone = { screen = Screen.Unlocked },
                                        )
                                        Screen.Unlocked -> Dashboard(
                                            summary = summary,
                                            deviceAvailable = deviceAvailable,
                                            notice = notice,
                                            wide = wide,
                                            onLock = {
                                                lockArchive()
                                                passphrase = ""
                                                confirmation = ""
                                                recoveryKey = ""
                                                error = null
                                                forgetSelection()
                                                screen = Screen.Unlock
                                            },
                                            onForgetDevice = ::forgetDeviceKey,
                                            onCustomers = { screen = Screen.Customers },
                                            onDrafts = { screen = Screen.Drafts },
                                            onDocuments = { screen = Screen.Documents },
                                            onImport = { screen = Screen.Import },
                                            onBackup = { screen = Screen.Backup },
                                            onSettings = { screen = Screen.Settings },
                                        )
                                        Screen.Customers -> if (panes) {
                                            CustomerPanes(
                                                selected = selectedCustomer,
                                                version = customersVersion,
                                                windowWidthDp = windowWidthDp,
                                                fold = fold,
                                                onBack = { screen = Screen.Unlocked },
                                                onSelect = { selectedCustomer = it },
                                                onSaved = { customersVersion++ },
                                                onInteraction = ::touch,
                                            )
                                        } else {
                                            CustomerListScreen(
                                                onBack = { screen = Screen.Unlocked },
                                                onEdit = { screen = Screen.CustomerEdit(it) },
                                                onAdd = { screen = Screen.CustomerEdit(Customer()) },
                                                onInteraction = ::touch,
                                            )
                                        }
                                        is Screen.CustomerEdit -> if (panes) {
                                            CustomerPanes(
                                                selected = selectedCustomer ?: current.customer,
                                                version = customersVersion,
                                                windowWidthDp = windowWidthDp,
                                                fold = fold,
                                                onBack = { screen = Screen.Unlocked },
                                                onSelect = { selectedCustomer = it },
                                                onSaved = { customersVersion++ },
                                                onInteraction = ::touch,
                                            )
                                        } else {
                                            CustomerEditScreen(
                                                customer = current.customer,
                                                onBack = { screen = Screen.Customers },
                                                onSaved = { screen = Screen.Customers },
                                                onInteraction = ::touch,
                                            )
                                        }
                                        Screen.Drafts -> DraftListScreen(
                                            onBack = { screen = Screen.Unlocked },
                                            onOpen = { draft, customers ->
                                                screen = Screen.DraftEditor(draft, customers)
                                            },
                                            onCustomers = { screen = Screen.Customers },
                                            onInteraction = ::touch,
                                        )
                                        is Screen.DraftEditor -> DraftEditorScreen(
                                            draft = current.draft,
                                            customers = current.customers,
                                            onBack = { screen = Screen.Drafts },
                                            onInteraction = ::touch,
                                            onIssued = { screen = Screen.Issued(it) },
                                        )
                                        is Screen.Issued -> IssueResultScreen(current.result) {
                                            val id = current.result.issued?.id
                                            screen = if (id.isNullOrBlank()) Screen.Documents else Screen.DocumentView(id)
                                        }
                                        Screen.Documents -> if (panes) {
                                            DocumentPanes(
                                                state = register,
                                                selectedId = selectedDocument,
                                                version = documentsVersion,
                                                windowWidthDp = windowWidthDp,
                                                fold = fold,
                                                onBack = { screen = Screen.Unlocked },
                                                onSelect = { selectedDocument = it },
                                                onChanged = { documentsVersion++ },
                                                onInteraction = ::touch,
                                            )
                                        } else {
                                            DocumentListScreen(
                                                state = register,
                                                onBack = { screen = Screen.Unlocked },
                                                onOpen = { screen = Screen.DocumentView(it) },
                                                onInteraction = ::touch,
                                            )
                                        }
                                        is Screen.DocumentView -> if (panes) {
                                            DocumentPanes(
                                                state = register,
                                                selectedId = selectedDocument ?: current.id,
                                                version = documentsVersion,
                                                windowWidthDp = windowWidthDp,
                                                fold = fold,
                                                onBack = { screen = Screen.Unlocked },
                                                onSelect = { selectedDocument = it },
                                                onChanged = { documentsVersion++ },
                                                onInteraction = ::touch,
                                            )
                                        } else {
                                            DocumentDetailScreen(
                                                id = current.id,
                                                onBack = { screen = Screen.Documents },
                                            )
                                        }
                                        Screen.Import -> ImportScreen(
                                            onBack = { screen = Screen.Unlocked },
                                            onOpen = { screen = Screen.DocumentView(it) },
                                            onInteraction = ::touch,
                                        )
                                        Screen.Backup -> BackupScreen(
                                            onBack = { screen = Screen.Unlocked },
                                            onRestored = {
                                                summary = it
                                                notice = strings.archiveRestored
                                                screen = Screen.Unlocked
                                            },
                                            onLocked = {
                                                error = it
                                                screen = Screen.Unlock
                                            },
                                            onInteraction = ::touch,
                                        )
                                        Screen.Seller -> SellerEditScreen(
                                            onBack = { screen = Screen.Settings },
                                            onInteraction = ::touch,
                                        )
                                        Screen.Settings -> SettingsScreen(
                                            onBack = { screen = Screen.Unlocked },
                                            onSeller = { screen = Screen.Seller },
                                            onAddRecovery = { screen = Screen.AddRecovery },
                                            onInteraction = ::touch,
                                        )
                                    }
                                }
                            }

                            if (screen is Screen.Settings) {
                                Spacer(Modifier.height(24.dp))
                                Diagnostics(
                                    modifier = Modifier.widthIn(max = contentWidth).fillMaxWidth(),
                                    onInteraction = ::touch,
                                )
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun CreateForm(
    passphrase: String,
    confirmation: String,
    remember: Boolean,
    rememberSupported: Boolean,
    busy: Boolean,
    error: String?,
    onPassphrase: (String) -> Unit,
    onConfirmation: (String) -> Unit,
    onRemember: (Boolean) -> Unit,
    onSubmit: () -> Unit,
) {
    Text(strings.createTitle, style = MaterialTheme.typography.titleLarge)
    Spacer(Modifier.height(8.dp))
    Text(
        text = strings.createNote,
        style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    Spacer(Modifier.height(20.dp))
    PasswordField(value = passphrase, label = strings.passphrase, enabled = !busy, onChange = onPassphrase)
    Spacer(Modifier.height(12.dp))
    PasswordField(
        value = confirmation,
        label = strings.repeatPassphrase,
        enabled = !busy,
        onChange = onConfirmation,
    )
    Spacer(Modifier.height(8.dp))
    ErrorText(error)
    Spacer(Modifier.height(8.dp))
    if (rememberSupported) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Checkbox(checked = remember, onCheckedChange = onRemember, enabled = !busy)
            Text(strings.rememberDevice, style = MaterialTheme.typography.bodyMedium)
        }
        Text(
            text = strings.rememberNote,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(8.dp))
    }
    Button(
        onClick = onSubmit,
        enabled = !busy,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text(if (busy) strings.creating else strings.createArchive)
    }
}

@Composable
private fun ResetForm(
    recoveryKey: String,
    passphrase: String,
    confirmation: String,
    busy: Boolean,
    error: String?,
    onRecoveryKey: (String) -> Unit,
    onPassphrase: (String) -> Unit,
    onConfirmation: (String) -> Unit,
    onSubmit: () -> Unit,
    onBack: () -> Unit,
) {
    Text(strings.resetTitle, style = MaterialTheme.typography.titleLarge)
    Spacer(Modifier.height(8.dp))
    Text(
        text = strings.resetNote,
        style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    Spacer(Modifier.height(20.dp))
    PasswordField(value = recoveryKey, label = strings.recoveryKey, enabled = !busy, onChange = onRecoveryKey)
    Spacer(Modifier.height(12.dp))
    PasswordField(value = passphrase, label = strings.newPassphrase, enabled = !busy, onChange = onPassphrase)
    Spacer(Modifier.height(12.dp))
    PasswordField(
        value = confirmation,
        label = strings.repeatNewPassphrase,
        enabled = !busy,
        onChange = onConfirmation,
    )
    Spacer(Modifier.height(8.dp))
    ErrorText(error)
    Spacer(Modifier.height(8.dp))
    Button(onClick = onSubmit, enabled = !busy, modifier = Modifier.fillMaxWidth()) {
        Text(if (busy) strings.resetting else strings.resetAction)
    }
    TextButton(onClick = onBack, enabled = !busy, modifier = Modifier.fillMaxWidth()) {
        Text(strings.back)
    }
}

@Composable
private fun AddRecoveryForm(
    passphrase: String,
    busy: Boolean,
    error: String?,
    onPassphrase: (String) -> Unit,
    onSubmit: () -> Unit,
    onBack: () -> Unit,
) {
    Text(strings.addRecoveryTitle, style = MaterialTheme.typography.titleLarge)
    Spacer(Modifier.height(8.dp))
    Text(
        text = strings.addRecoveryNote,
        style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    Spacer(Modifier.height(20.dp))
    PasswordField(value = passphrase, label = strings.passphrase, enabled = !busy, onChange = onPassphrase)
    Spacer(Modifier.height(8.dp))
    ErrorText(error)
    Spacer(Modifier.height(8.dp))
    Button(onClick = onSubmit, enabled = !busy, modifier = Modifier.fillMaxWidth()) {
        Text(if (busy) strings.creating else strings.createRecoveryKey)
    }
    TextButton(onClick = onBack, enabled = !busy, modifier = Modifier.fillMaxWidth()) {
        Text(strings.back)
    }
}

@Composable
private fun RecoveryKeyShown(key: String, notice: String?, onDone: () -> Unit) {
    Text(strings.recoveryShownTitle, style = MaterialTheme.typography.titleLarge)
    Spacer(Modifier.height(8.dp))
    Text(
        text = strings.recoveryShownNote,
        style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.error,
    )
    Spacer(Modifier.height(16.dp))
    Surface(
        color = MaterialTheme.colorScheme.surfaceVariant,
        shape = MaterialTheme.shapes.medium,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text(
            text = key,
            modifier = Modifier.padding(16.dp),
            fontFamily = FontFamily.Monospace,
            style = MaterialTheme.typography.bodyLarge,
        )
    }
    Spacer(Modifier.height(12.dp))
    Text(
        text = strings.recoveryKeep,
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    if (notice != null) {
        Spacer(Modifier.height(8.dp))
        Text(
            text = notice,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
    Spacer(Modifier.height(16.dp))
    Button(onClick = onDone, modifier = Modifier.fillMaxWidth()) {
        Text(strings.wroteItDown)
    }
}

@Composable
private fun UnlockForm(
    passphrase: String,
    deviceAvailable: Boolean,
    busy: Boolean,
    error: String?,
    onPassphrase: (String) -> Unit,
    onSubmit: () -> Unit,
    onUnlockWithDevice: () -> Unit,
    onForgot: () -> Unit,
) {
    Text(strings.unlockTitle, style = MaterialTheme.typography.titleLarge)
    Spacer(Modifier.height(8.dp))
    Text(
        text = strings.unlockNote,
        style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    Spacer(Modifier.height(20.dp))
    PasswordField(value = passphrase, label = strings.passphrase, enabled = !busy, onChange = onPassphrase)
    Spacer(Modifier.height(8.dp))
    ErrorText(error)
    Spacer(Modifier.height(8.dp))
    Button(
        onClick = onSubmit,
        enabled = !busy,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text(if (busy) strings.unlocking else strings.unlock)
    }
    if (deviceAvailable) {
        Spacer(Modifier.height(12.dp))
        OutlinedButton(
            onClick = onUnlockWithDevice,
            enabled = !busy,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text(strings.unlockWithDevice)
        }
        Spacer(Modifier.height(6.dp))
        Text(
            text = strings.unlockWithDeviceNote,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
    Spacer(Modifier.height(4.dp))
    TextButton(onClick = onForgot, enabled = !busy, modifier = Modifier.fillMaxWidth()) {
        Text(strings.forgotPassphrase)
    }
}

@Composable
fun Dashboard(
    summary: ArchiveSummary,
    deviceAvailable: Boolean,
    notice: String?,
    wide: Boolean,
    onLock: () -> Unit,
    onForgetDevice: () -> Unit,
    onCustomers: () -> Unit,
    onDrafts: () -> Unit,
    onDocuments: () -> Unit,
    onImport: () -> Unit,
    onBackup: () -> Unit,
    onSettings: () -> Unit,
) {
    Text(strings.unlocked, style = MaterialTheme.typography.titleLarge)
    Spacer(Modifier.height(16.dp))
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceEvenly,
    ) {
        Metric(strings.sellers, summary.sellers)
        Metric(strings.drafts, summary.drafts)
        Metric(strings.schema, summary.schemaVersion)
    }
    if (notice != null) {
        Spacer(Modifier.height(12.dp))
        Text(
            text = notice,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
    Spacer(Modifier.height(20.dp))
    if (wide) {
        // Tablets and landscape phones get two columns; the rail above already
        // names the places, so the cards only need to stay reachable.
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(onClick = onCustomers, modifier = Modifier.fillMaxWidth()) { Text(strings.customers) }
                Button(onClick = onDocuments, modifier = Modifier.fillMaxWidth()) { Text(strings.documents) }
                Button(onClick = onBackup, modifier = Modifier.fillMaxWidth()) { Text(strings.backup) }
            }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(onClick = onDrafts, modifier = Modifier.fillMaxWidth()) { Text(strings.drafts) }
                Button(onClick = onImport, modifier = Modifier.fillMaxWidth()) { Text(strings.importInvoice) }
                Button(onClick = onSettings, modifier = Modifier.fillMaxWidth()) { Text(strings.settings) }
            }
        }
    } else {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Button(onClick = onCustomers, modifier = Modifier.weight(1f)) { Text(strings.customers) }
            Button(onClick = onDrafts, modifier = Modifier.weight(1f)) { Text(strings.drafts) }
        }
        Spacer(Modifier.height(8.dp))
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Button(onClick = onDocuments, modifier = Modifier.weight(1f)) { Text(strings.documents) }
            Button(onClick = onImport, modifier = Modifier.weight(1f)) { Text(strings.importInvoice) }
        }
        Spacer(Modifier.height(8.dp))
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Button(onClick = onBackup, modifier = Modifier.weight(1f)) { Text(strings.backup) }
            Button(onClick = onSettings, modifier = Modifier.weight(1f)) { Text(strings.settings) }
        }
    }
    Spacer(Modifier.height(8.dp))
    Button(
        onClick = onLock,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text(strings.lock)
    }
    if (deviceAvailable) {
        Spacer(Modifier.height(4.dp))
        TextButton(onClick = onForgetDevice, modifier = Modifier.fillMaxWidth()) {
            Text(strings.forgetDevice)
        }
    }
}

@Composable
private fun Metric(label: String, value: Long) {
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        Text(value.toString(), style = MaterialTheme.typography.headlineSmall)
        Text(label, style = MaterialTheme.typography.labelMedium)
    }
}

@Composable
private fun PasswordField(value: String, label: String, enabled: Boolean, onChange: (String) -> Unit) {
    OutlinedTextField(
        value = value,
        onValueChange = onChange,
        label = { Text(label) },
        modifier = Modifier.fillMaxWidth(),
        singleLine = true,
        enabled = enabled,
        visualTransformation = PasswordVisualTransformation(),
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
    )
}

@Composable
private fun ErrorText(error: String?) {
    if (error != null) {
        Text(
            text = error,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.error,
        )
    }
}

@Composable
private fun Diagnostics(modifier: Modifier = Modifier, onInteraction: () -> Unit = {}) {
    var expanded by remember { mutableStateOf(false) }
    var keystoreResult by remember { mutableStateOf(strings.notRun) }
    var keystoreBusy by remember { mutableStateOf(false) }
    var argonResult by remember { mutableStateOf(strings.notRun) }
    var argonBusy by remember { mutableStateOf(false) }
    var artefactResult by remember { mutableStateOf(strings.notRun) }
    var artefactBusy by remember { mutableStateOf(false) }
    var canaryResult by remember { mutableStateOf(strings.notRun) }
    var canaryBusy by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    Column(modifier = modifier, horizontalAlignment = Alignment.CenterHorizontally) {
        TextButton(onClick = {
            expanded = !expanded
            onInteraction()
        }) {
            Text(if (expanded) strings.hideDiagnostics else strings.diagnostics)
        }
        if (!expanded) return@Column

        Card {
            Column(modifier = Modifier.padding(20.dp)) {
                ProbeButton(
                    label = strings.runKeystore,
                    busy = keystoreBusy,
                    result = keystoreResult,
                    onInteraction = onInteraction,
                    onRun = {
                        keystoreBusy = true
                        keystoreResult = strings.waitingForAuth
                        scope.launch {
                            keystoreResult = withContext(Dispatchers.IO) {
                                runCatching { runKeystoreTest() }.getOrElse { it.message ?: it.toString() }
                            }
                            keystoreBusy = false
                        }
                    },
                )
                Spacer(Modifier.height(12.dp))
                ProbeButton(
                    label = strings.benchArgon,
                    busy = argonBusy,
                    result = argonResult,
                    onInteraction = onInteraction,
                    onRun = {
                        argonBusy = true
                        argonResult = strings.runningArgon
                        scope.launch {
                            argonResult = withContext(Dispatchers.IO) {
                                runCatching { runArgon2Benchmark() }.getOrElse { it.message ?: it.toString() }
                            }
                            argonBusy = false
                        }
                    },
                )
                Spacer(Modifier.height(12.dp))
                ProbeButton(
                    label = strings.hashArtefacts,
                    busy = artefactBusy,
                    result = artefactResult,
                    onInteraction = onInteraction,
                    onRun = {
                        artefactBusy = true
                        artefactResult = strings.hashingArtefacts
                        scope.launch {
                            artefactResult = withContext(Dispatchers.IO) {
                                runCatching { runArtefactHashes() }.getOrElse { it.message ?: it.toString() }
                            }
                            artefactBusy = false
                        }
                    },
                )
                Spacer(Modifier.height(12.dp))
                ProbeButton(
                    label = strings.canary,
                    busy = canaryBusy,
                    result = canaryResult,
                    onInteraction = onInteraction,
                    onRun = {
                        canaryBusy = true
                        canaryResult = strings.scanning
                        scope.launch {
                            canaryResult = withContext(Dispatchers.IO) {
                                runCatching { runCanaryScan() }.getOrElse { it.message ?: it.toString() }
                            }
                            canaryBusy = false
                        }
                    },
                )
            }
        }
    }
}

@Composable
private fun ProbeButton(
    label: String,
    busy: Boolean,
    result: String,
    onInteraction: () -> Unit,
    onRun: () -> Unit,
) {
    Button(
        onClick = {
            onInteraction()
            onRun()
        },
        enabled = !busy,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text(label)
    }
    Spacer(Modifier.height(6.dp))
    Text(result, style = MaterialTheme.typography.bodySmall)
}

/** Where the rail can take an open archive. */
private fun RailDestination.toScreen(): Screen = when (this) {
    RailDestination.Home -> Screen.Unlocked
    RailDestination.Customers -> Screen.Customers
    RailDestination.Drafts -> Screen.Drafts
    RailDestination.Documents -> Screen.Documents
    RailDestination.Settings -> Screen.Settings
}

private fun Screen.destination(): RailDestination = when (this) {
    Screen.Customers, is Screen.CustomerEdit -> RailDestination.Customers
    Screen.Drafts, is Screen.DraftEditor -> RailDestination.Drafts
    Screen.Documents, is Screen.DocumentView -> RailDestination.Documents
    Screen.Settings, Screen.Seller, Screen.AddRecovery -> RailDestination.Settings
    else -> RailDestination.Home
}

/** Screens that sit beside the rail; the lock screens keep the whole window. */
private fun Screen.usesShell(): Boolean =
    isArchiveContent() || this is Screen.Import || this is Screen.Backup || this is Screen.Settings

/** Customers on the left, the customer being edited or added on the right. */
@Composable
private fun CustomerPanes(
    selected: Customer?,
    version: Int,
    windowWidthDp: Float,
    fold: FoldGap?,
    onBack: () -> Unit,
    onSelect: (Customer?) -> Unit,
    onSaved: () -> Unit,
    onInteraction: () -> Unit,
) {
    val split = paneSplit(windowWidthDp, fold)
    Row(Modifier.fillMaxSize()) {
        Box(Modifier.weight(split.listWeight).fillMaxHeight().verticalScroll(rememberScrollState())) {
            CustomerListScreen(
                onBack = onBack,
                onEdit = onSelect,
                onAdd = { onSelect(Customer()) },
                onInteraction = onInteraction,
                selectedId = selected?.id,
                version = version,
            )
        }
        if (split.gapDp > 0f) Spacer(Modifier.width(split.gapDp.dp)) else VerticalDivider()
        Box(Modifier.weight(split.detailWeight).fillMaxHeight().verticalScroll(rememberScrollState())) {
            val current = selected
            if (current == null) {
                EmptyPane(strings.chooseCustomerToEdit)
            } else {
                // A different customer is a different form, not a refresh of this one.
                key(current.id) {
                    CustomerEditScreen(
                        customer = current,
                        onBack = { onSelect(null) },
                        onSaved = {
                            onSelect(null)
                            onSaved()
                        },
                        onInteraction = onInteraction,
                    )
                }
            }
        }
    }
}

/** The register on the left, the document it opens on the right. */
@Composable
private fun DocumentPanes(
    state: DocumentRegisterState,
    selectedId: String?,
    version: Int,
    windowWidthDp: Float,
    fold: FoldGap?,
    onBack: () -> Unit,
    onSelect: (String?) -> Unit,
    onChanged: () -> Unit,
    onInteraction: () -> Unit,
) {
    val split = paneSplit(windowWidthDp, fold)
    Row(Modifier.fillMaxSize()) {
        Box(Modifier.weight(split.listWeight).fillMaxHeight()) {
            DocumentListScreen(
                state = state,
                onBack = onBack,
                onOpen = onSelect,
                onInteraction = onInteraction,
                selectedId = selectedId,
                version = version,
            )
        }
        if (split.gapDp > 0f) Spacer(Modifier.width(split.gapDp.dp)) else VerticalDivider()
        Box(Modifier.weight(split.detailWeight).fillMaxHeight().verticalScroll(rememberScrollState())) {
            if (selectedId == null) {
                EmptyPane(strings.chooseDocumentToOpen)
            } else {
                DocumentDetailScreen(
                    id = selectedId,
                    onBack = { onSelect(null) },
                    onChanged = onChanged,
                )
            }
        }
    }
}

@Composable
private fun EmptyPane(text: String) {
    Box(Modifier.fillMaxSize().padding(24.dp), contentAlignment = Alignment.Center) {
        Text(
            text,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
    }
}
