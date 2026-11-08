package com.github.sdgoij.bonk

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
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
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
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
}

private fun Screen.isArchiveContent(): Boolean = when (this) {
    Screen.Unlocked,
    Screen.AddRecovery,
    Screen.Customers,
    Screen.Drafts,
    is Screen.CustomerEdit,
    is Screen.DraftEditor,
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

    fun touch() {
        interactions++
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
            screen = Screen.Unlock
        }
    }

    fun fail(message: Throwable) {
        error = message.message ?: message.toString()
        busy = false
    }

    fun unlock() {
        if (passphrase.isEmpty()) {
            error = "Enter the passphrase."
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
            error = "Choose a passphrase."
            return
        }
        if (passphrase != confirmation) {
            error = "The passphrases do not match."
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
                        notice = "The archive was created, but this device was not remembered: " +
                            (remembered.exceptionOrNull()?.message ?: "unknown error")
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
                    notice = "The archive was created, but no recovery key was generated: " +
                        (recovery.exceptionOrNull()?.message ?: "unknown error")
                    Screen.Unlocked
                }
            } else {
                fail(result.exceptionOrNull() ?: IllegalStateException("unknown error"))
            }
            busy = false
        }
    }

    fun reset() {
        if (recoveryKey.isBlank()) {
            error = "Enter the recovery key."
            return
        }
        if (passphrase.isEmpty()) {
            error = "Choose a new passphrase."
            return
        }
        if (passphrase != confirmation) {
            error = "The new passphrases do not match."
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
            error = "Enter the current passphrase."
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
                fail(result.exceptionOrNull() ?: IllegalStateException("unknown error"))
                // A Keystore invalidation deletes the wrapped entry. If it is
                // gone, stop offering the route and say so.
                deviceAvailable = withContext(Dispatchers.IO) { runCatching { deviceReady() }.getOrDefault(false) }
                if (!deviceAvailable) {
                    error = "The device key is no longer valid. Unlock with the passphrase."
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
                notice = if (it) "This device no longer opens the archive." else "No device key was stored."
            }.onFailure { notice = it.message ?: it.toString() }
        }
    }

    MaterialTheme {
        Surface(modifier = Modifier.fillMaxSize()) {
            BoxWithConstraints(modifier = Modifier.fillMaxSize()) {
                val contentWidth = if (maxWidth > 720.dp) 560.dp else maxWidth
                val tall = maxHeight > 800.dp
                Column(
                    modifier = Modifier
                        .fillMaxSize()
                        .verticalScroll(rememberScrollState())
                        .padding(24.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Spacer(Modifier.height(if (tall) 72.dp else 16.dp))
                    Text("Bonk", style = MaterialTheme.typography.headlineLarge)
                    Spacer(Modifier.height(8.dp))
                    Text(
                        text = "Local-first e-invoicing",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Spacer(Modifier.height(24.dp))

                    Card(modifier = Modifier.widthIn(max = contentWidth).fillMaxWidth()) {
                        Column(modifier = Modifier.padding(24.dp)) {
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
                                Screen.Unlocked -> Unlocked(
                                    summary = summary,
                                    deviceAvailable = deviceAvailable,
                                    notice = notice,
                                    onLock = {
                                        lockArchive()
                                        passphrase = ""
                                        confirmation = ""
                                        recoveryKey = ""
                                        error = null
                                        screen = Screen.Unlock
                                    },
                                    onForgetDevice = ::forgetDeviceKey,
                                    onAddRecovery = {
                                        passphrase = ""
                                        error = null
                                        screen = Screen.AddRecovery
                                    },
                                    onCustomers = { screen = Screen.Customers },
                                    onDrafts = { screen = Screen.Drafts },
                                )
                                Screen.Customers -> CustomerListScreen(
                                    onBack = { screen = Screen.Unlocked },
                                    onEdit = { screen = Screen.CustomerEdit(it) },
                                    onAdd = { screen = Screen.CustomerEdit(Customer()) },
                                    onInteraction = ::touch,
                                )
                                is Screen.CustomerEdit -> CustomerEditScreen(
                                    customer = current.customer,
                                    onBack = { screen = Screen.Customers },
                                    onSaved = { screen = Screen.Customers },
                                    onInteraction = ::touch,
                                )
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
                                )
                            }
                        }
                    }

                    if (screen is Screen.Unlocked) {
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
    Text("Create your archive", style = MaterialTheme.typography.titleLarge)
    Spacer(Modifier.height(8.dp))
    Text(
        text = "The passphrase protects the archive on this device. There is no way to recover it if it is lost.",
        style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    Spacer(Modifier.height(20.dp))
    PasswordField(value = passphrase, label = "Passphrase", enabled = !busy, onChange = onPassphrase)
    Spacer(Modifier.height(12.dp))
    PasswordField(value = confirmation, label = "Repeat passphrase", enabled = !busy, onChange = onConfirmation)
    Spacer(Modifier.height(8.dp))
    ErrorText(error)
    Spacer(Modifier.height(8.dp))
    if (rememberSupported) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Checkbox(checked = remember, onCheckedChange = onRemember, enabled = !busy)
            Text("Remember this device", style = MaterialTheme.typography.bodyMedium)
        }
        Text(
            text = "Unlocks with your fingerprint or face next time. The passphrase still works.",
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
        Text(if (busy) "Creating..." else "Create archive")
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
    Text("Reset the passphrase", style = MaterialTheme.typography.titleLarge)
    Spacer(Modifier.height(8.dp))
    Text(
        text = "Enter the recovery key you wrote down. It will be spent, and a new one will be shown.",
        style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    Spacer(Modifier.height(20.dp))
    PasswordField(value = recoveryKey, label = "Recovery key", enabled = !busy, onChange = onRecoveryKey)
    Spacer(Modifier.height(12.dp))
    PasswordField(value = passphrase, label = "New passphrase", enabled = !busy, onChange = onPassphrase)
    Spacer(Modifier.height(12.dp))
    PasswordField(
        value = confirmation,
        label = "Repeat new passphrase",
        enabled = !busy,
        onChange = onConfirmation,
    )
    Spacer(Modifier.height(8.dp))
    ErrorText(error)
    Spacer(Modifier.height(8.dp))
    Button(onClick = onSubmit, enabled = !busy, modifier = Modifier.fillMaxWidth()) {
        Text(if (busy) "Resetting..." else "Reset passphrase")
    }
    TextButton(onClick = onBack, enabled = !busy, modifier = Modifier.fillMaxWidth()) {
        Text("Back")
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
    Text("Add a recovery key", style = MaterialTheme.typography.titleLarge)
    Spacer(Modifier.height(8.dp))
    Text(
        text = "Enter the current passphrase. The recovery key is shown once and replaces any earlier one.",
        style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    Spacer(Modifier.height(20.dp))
    PasswordField(value = passphrase, label = "Passphrase", enabled = !busy, onChange = onPassphrase)
    Spacer(Modifier.height(8.dp))
    ErrorText(error)
    Spacer(Modifier.height(8.dp))
    Button(onClick = onSubmit, enabled = !busy, modifier = Modifier.fillMaxWidth()) {
        Text(if (busy) "Creating..." else "Create recovery key")
    }
    TextButton(onClick = onBack, enabled = !busy, modifier = Modifier.fillMaxWidth()) {
        Text("Back")
    }
}

@Composable
private fun RecoveryKeyShown(key: String, notice: String?, onDone: () -> Unit) {
    Text("Write down this recovery key", style = MaterialTheme.typography.titleLarge)
    Spacer(Modifier.height(8.dp))
    Text(
        text = "This is the only way back in if you forget the passphrase. It is shown once.",
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
        text = "Keep it somewhere other than this device. Anyone with this key can reset the passphrase.",
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
        Text("I wrote it down")
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
    Text("Unlock your archive", style = MaterialTheme.typography.titleLarge)
    Spacer(Modifier.height(8.dp))
    Text(
        text = "Enter the passphrase you chose for this device.",
        style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    Spacer(Modifier.height(20.dp))
    PasswordField(value = passphrase, label = "Passphrase", enabled = !busy, onChange = onPassphrase)
    Spacer(Modifier.height(8.dp))
    ErrorText(error)
    Spacer(Modifier.height(8.dp))
    Button(
        onClick = onSubmit,
        enabled = !busy,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text(if (busy) "Unlocking..." else "Unlock")
    }
    if (deviceAvailable) {
        Spacer(Modifier.height(12.dp))
        OutlinedButton(
            onClick = onUnlockWithDevice,
            enabled = !busy,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text("Unlock with device")
        }
        Spacer(Modifier.height(6.dp))
        Text(
            text = "Uses your fingerprint or face. The passphrase still works.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
    Spacer(Modifier.height(4.dp))
    TextButton(onClick = onForgot, enabled = !busy, modifier = Modifier.fillMaxWidth()) {
        Text("Forgot passphrase?")
    }
}

@Composable
private fun Unlocked(
    summary: ArchiveSummary,
    deviceAvailable: Boolean,
    notice: String?,
    onLock: () -> Unit,
    onForgetDevice: () -> Unit,
    onAddRecovery: () -> Unit,
    onCustomers: () -> Unit,
    onDrafts: () -> Unit,
) {
    Text("Unlocked", style = MaterialTheme.typography.titleLarge)
    Spacer(Modifier.height(16.dp))
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceEvenly,
    ) {
        Metric("Sellers", summary.sellers)
        Metric("Drafts", summary.drafts)
        Metric("Schema", summary.schemaVersion)
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
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Button(onClick = onCustomers, modifier = Modifier.weight(1f)) { Text("Customers") }
        Button(onClick = onDrafts, modifier = Modifier.weight(1f)) { Text("Drafts") }
    }
    Spacer(Modifier.height(8.dp))
    Button(
        onClick = onLock,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text("Lock")
    }
    if (deviceAvailable) {
        Spacer(Modifier.height(4.dp))
        TextButton(onClick = onForgetDevice, modifier = Modifier.fillMaxWidth()) {
            Text("Forget this device")
        }
    }
    TextButton(onClick = onAddRecovery, modifier = Modifier.fillMaxWidth()) {
        Text("Add recovery key")
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
    var keystoreResult by remember { mutableStateOf("not run") }
    var keystoreBusy by remember { mutableStateOf(false) }
    var argonResult by remember { mutableStateOf("not run") }
    var argonBusy by remember { mutableStateOf(false) }
    var artefactResult by remember { mutableStateOf("not run") }
    var artefactBusy by remember { mutableStateOf(false) }
    var canaryResult by remember { mutableStateOf("not run") }
    var canaryBusy by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    Column(modifier = modifier, horizontalAlignment = Alignment.CenterHorizontally) {
        TextButton(onClick = {
            expanded = !expanded
            onInteraction()
        }) {
            Text(if (expanded) "Hide diagnostics" else "Diagnostics")
        }
        if (!expanded) return@Column

        Card {
            Column(modifier = Modifier.padding(20.dp)) {
                ProbeButton(
                    label = "Run S3 Keystore test",
                    busy = keystoreBusy,
                    result = keystoreResult,
                    onInteraction = onInteraction,
                    onRun = {
                        keystoreBusy = true
                        keystoreResult = "waiting for authentication..."
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
                    label = "Bench Argon2 (64 MiB)",
                    busy = argonBusy,
                    result = argonResult,
                    onInteraction = onInteraction,
                    onRun = {
                        argonBusy = true
                        argonResult = "running 64 MiB Argon2id..."
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
                    label = "Hash UBL + PDF",
                    busy = artefactBusy,
                    result = artefactResult,
                    onInteraction = onInteraction,
                    onRun = {
                        artefactBusy = true
                        artefactResult = "hashing UBL and PDF..."
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
                    label = "Canary scan",
                    busy = canaryBusy,
                    result = canaryResult,
                    onInteraction = onInteraction,
                    onRun = {
                        canaryBusy = true
                        canaryResult = "scanning database, WAL, and keystore..."
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
