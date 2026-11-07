package com.github.sdgoij.bonk

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
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

/** Creates, locks, and reopens an encrypted archive through the FFI. */
expect fun runArchiveProbe(): String

@Composable
fun App() {
    var name by remember { mutableStateOf("Android") }
    var greeting by remember { mutableStateOf(greet("Android")) }
    var keystoreResult by remember { mutableStateOf("not run") }
    var keystoreBusy by remember { mutableStateOf(false) }
    var argonResult by remember { mutableStateOf("not run") }
    var argonBusy by remember { mutableStateOf(false) }
    var artefactResult by remember { mutableStateOf("not run") }
    var artefactBusy by remember { mutableStateOf(false) }
    var archiveResult by remember { mutableStateOf("not run") }
    var archiveBusy by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    MaterialTheme {
        Surface(modifier = Modifier.fillMaxSize()) {
            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(24.dp),
                verticalArrangement = Arrangement.Center,
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Text(
                    text = "S2: UniFFI + Compose Multiplatform",
                    style = MaterialTheme.typography.headlineSmall,
                )
                Spacer(Modifier.height(24.dp))
                OutlinedTextField(
                    value = name,
                    onValueChange = { name = it },
                    label = { Text("Name") },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                )
                Spacer(Modifier.height(16.dp))
                Button(onClick = { greeting = greet(name) }) {
                    Text("Call Rust")
                }
                Spacer(Modifier.height(24.dp))
                Text(
                    text = greeting,
                    style = MaterialTheme.typography.bodyLarge,
                )
                Spacer(Modifier.height(24.dp))
                Button(
                    onClick = {
                        keystoreBusy = true
                        keystoreResult = "waiting for authentication..."
                        scope.launch {
                            keystoreResult = withContext(Dispatchers.IO) {
                                runCatching { runKeystoreTest() }
                                    .getOrElse { it.message ?: it.toString() }
                            }
                            keystoreBusy = false
                        }
                    },
                    enabled = !keystoreBusy,
                ) {
                    Text("Run S3 Keystore test")
                }
                Spacer(Modifier.height(12.dp))
                Text(
                    text = keystoreResult,
                    style = MaterialTheme.typography.bodyMedium,
                )
                Spacer(Modifier.height(16.dp))
                Button(
                    onClick = {
                        argonBusy = true
                        argonResult = "running 64 MiB Argon2id..."
                        scope.launch {
                            argonResult = withContext(Dispatchers.IO) {
                                runCatching { runArgon2Benchmark() }
                                    .getOrElse { it.message ?: it.toString() }
                            }
                            argonBusy = false
                        }
                    },
                    enabled = !argonBusy,
                ) {
                    Text("Bench Argon2 (64 MiB)")
                }
                Spacer(Modifier.height(12.dp))
                Text(
                    text = argonResult,
                    style = MaterialTheme.typography.bodyMedium,
                )
                Spacer(Modifier.height(16.dp))
                Button(
                    onClick = {
                        artefactBusy = true
                        artefactResult = "hashing UBL and PDF..."
                        scope.launch {
                            artefactResult = withContext(Dispatchers.IO) {
                                runCatching { runArtefactHashes() }
                                    .getOrElse { it.message ?: it.toString() }
                            }
                            artefactBusy = false
                        }
                    },
                    enabled = !artefactBusy,
                ) {
                    Text("Hash UBL + PDF")
                }
                Spacer(Modifier.height(12.dp))
                Text(
                    text = artefactResult,
                    style = MaterialTheme.typography.bodyMedium,
                )
                Spacer(Modifier.height(16.dp))
                Button(
                    onClick = {
                        archiveBusy = true
                        archiveResult = "creating, locking, reopening..."
                        scope.launch {
                            archiveResult = withContext(Dispatchers.IO) {
                                runCatching { runArchiveProbe() }
                                    .getOrElse { it.message ?: it.toString() }
                            }
                            archiveBusy = false
                        }
                    },
                    enabled = !archiveBusy,
                ) {
                    Text("Archive create/lock/open")
                }
                Spacer(Modifier.height(12.dp))
                Text(
                    text = archiveResult,
                    style = MaterialTheme.typography.bodyMedium,
                )
            }
        }
    }
}
