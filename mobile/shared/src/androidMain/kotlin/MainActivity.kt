package com.github.sdgoij.bonk

import android.os.Build
import android.os.Bundle
import android.view.WindowManager
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
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
        enableEdgeToEdge()
        setContent { App() }
    }

    override fun onDestroy() {
        KeystoreHost.detach()
        MobileArchiveHost.detach()
        super.onDestroy()
    }
}
