package com.github.sdgoij.bonk

import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.util.Log
import android.view.WindowManager
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.core.content.IntentCompat
import androidx.fragment.app.FragmentActivity

class MainActivity : FragmentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // No screenshots and no recents snapshot: the window shows an unlocked archive.
        window.setFlags(WindowManager.LayoutParams.FLAG_SECURE, WindowManager.LayoutParams.FLAG_SECURE)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            setRecentsScreenshotEnabled(false)
        }
        KeystoreHost.attach(this)
        MobileArchiveHost.attach(this)
        // The chosen language is a device preference, read before anything draws.
        loadUiLanguageCode()?.let { UiLanguage.current = Lang.parse(it) }
        enableEdgeToEdge()
        handleIntent(intent)
        setContent { App() }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleIntent(intent)
    }

    override fun onDestroy() {
        KeystoreHost.detach()
        MobileArchiveHost.detach()
        super.onDestroy()
    }

    /**
     * Takes a document another app handed us and holds it until the archive is
     * open. The bytes are read now because a share grant can be short-lived, and
     * the intent is cleared so a configuration change does not import again.
     */
    private fun handleIntent(intent: Intent?) {
        val uri = when (intent?.action) {
            Intent.ACTION_VIEW -> intent.data
            Intent.ACTION_SEND -> IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)
            else -> null
        } ?: return
        runCatching { readDocument(this, uri) }
            .onSuccess { (name, bytes) -> PendingImport.offer(name, bytes) }
            .onFailure { Log.w("Bonk", "the hand-off could not be read", it) }
        setIntent(Intent())
    }
}
