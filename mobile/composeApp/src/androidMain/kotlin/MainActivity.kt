package com.github.sdgoij.bonk

import android.os.Bundle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.fragment.app.FragmentActivity

class MainActivity : FragmentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        KeystoreHost.attach(this)
        enableEdgeToEdge()
        setContent { App() }
    }

    override fun onDestroy() {
        KeystoreHost.detach()
        super.onDestroy()
    }
}
