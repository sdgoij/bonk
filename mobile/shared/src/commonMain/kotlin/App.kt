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

private sealed interface Screen {
    data object Checking : Screen
    data object Create : Screen
    data object Unlock : Screen
    data class Unlocked(val summary: ArchiveSummary) : Screen
}

@Composable
fun App() {
    var screen by remember { mutableStateOf<Screen>(Screen.Checking) }
    var passphrase by remember { mutableStateOf("") }
    var confirmation by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    var notice by remember { mutableStateOf<String?>(null) }
    var deviceAvailable by remember { mutableStateOf(false) }
    var rememberOnCreate by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    LaunchedEffect(Unit) {
        val state = withContext(Dispatchers.IO) {
            val exists = runCatching { archiveExists() }.getOrDefault(false)
            val device = exists && runCatching { deviceReady() }.getOrDefault(false)
            exists to device
        }
        deviceAvailable = state.second
        screen = if (state.first) Screen.Unlock else Screen.Create
    }

    // Lock after five minutes unlocked. A configuration change also closes the
    // handle through the activity's onDestroy.
    LaunchedEffect(screen) {
        if (screen is Screen.Unlocked) {
            delay(5 * 60 * 1000L)
            lockArchive()
            passphrase = ""
            confirmation = ""
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
            val result = withContext(Dispatchers.IO) { runCatching { openArchive(passphrase) } }
            result.onSuccess {
                passphrase = ""
                confirmation = ""
                screen = Screen.Unlocked(it)
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
                passphrase = ""
                confirmation = ""
                screen = Screen.Unlocked(result.getOrThrow())
            } else {
                fail(result.exceptionOrNull() ?: IllegalStateException("unknown error"))
            }
            busy = false
        }
    }

    fun unlockWithDevice() {
        busy = true
        error = null
        scope.launch {
            val result = withContext(Dispatchers.IO) { runCatching { openArchiveWithDevice() } }
            result.onSuccess {
                passphrase = ""
                confirmation = ""
                screen = Screen.Unlocked(it)
            }.onFailure(::fail)
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
                                    busy = busy,
                                    error = error,
                                    onPassphrase = {
                                        passphrase = it
                                        error = null
                                    },
                                    onConfirmation = {
                                        confirmation = it
                                        error = null
                                    },
                                    onRemember = { rememberOnCreate = it },
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
                                    },
                                    onSubmit = ::unlock,
                                    onUnlockWithDevice = ::unlockWithDevice,
                                )
                                is Screen.Unlocked -> Unlocked(
                                    summary = current.summary,
                                    deviceAvailable = deviceAvailable,
                                    notice = notice,
                                    onLock = {
                                        lockArchive()
                                        passphrase = ""
                                        error = null
                                        screen = Screen.Unlock
                                    },
                                    onForgetDevice = ::forgetDeviceKey,
                                )
                            }
                        }
                    }

                    if (screen is Screen.Unlocked) {
                        Spacer(Modifier.height(24.dp))
                        Diagnostics(modifier = Modifier.widthIn(max = contentWidth).fillMaxWidth())
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
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Checkbox(checked = remember, onCheckedChange = onRemember, enabled = !busy)
        Text("Remember this device", style = MaterialTheme.typography.bodyMedium)
    }
    Text(
        text = "Unlocks with your fingerprint, face, or device PIN next time.",
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    Spacer(Modifier.height(8.dp))
    Button(
        onClick = onSubmit,
        enabled = !busy,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text(if (busy) "Creating..." else "Create archive")
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
            text = "Uses your fingerprint, face, or device PIN.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun Unlocked(
    summary: ArchiveSummary,
    deviceAvailable: Boolean,
    notice: String?,
    onLock: () -> Unit,
    onForgetDevice: () -> Unit,
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
private fun Diagnostics(modifier: Modifier = Modifier) {
    var expanded by remember { mutableStateOf(false) }
    var keystoreResult by remember { mutableStateOf("not run") }
    var keystoreBusy by remember { mutableStateOf(false) }
    var argonResult by remember { mutableStateOf("not run") }
    var argonBusy by remember { mutableStateOf(false) }
    var artefactResult by remember { mutableStateOf("not run") }
    var artefactBusy by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    Column(modifier = modifier, horizontalAlignment = Alignment.CenterHorizontally) {
        TextButton(onClick = { expanded = !expanded }) {
            Text(if (expanded) "Hide diagnostics" else "Diagnostics")
        }
        if (!expanded) return@Column

        Card {
            Column(modifier = Modifier.padding(20.dp)) {
                ProbeButton(
                    label = "Run S3 Keystore test",
                    busy = keystoreBusy,
                    result = keystoreResult,
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
            }
        }
    }
}

@Composable
private fun ProbeButton(label: String, busy: Boolean, result: String, onRun: () -> Unit) {
    Button(onClick = onRun, enabled = !busy, modifier = Modifier.fillMaxWidth()) {
        Text(label)
    }
    Spacer(Modifier.height(6.dp))
    Text(result, style = MaterialTheme.typography.bodySmall)
}
