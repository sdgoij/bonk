package com.github.sdgoij.bonk

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.VerticalDivider
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.io.IOException
import kotlin.math.abs

/**
 * The shell at the three width bands, compared against committed images.
 *
 * The window is drawn at one pixel per dp, so the goldens do not depend on the
 * device's density; the light palette is pinned because the system choice is not
 * part of what these images assert. The lock screen is the only screen that is
 * pure UI, so the dashboard stands in for content here.
 *
 * Run it on a window at least 1280 px wide, which the tablet AVD is. A missing
 * golden fails the test and writes the image the device drew, which
 * `just mobile-screenshots-update` copies into the assets.
 */
@RunWith(AndroidJUnit4::class)
class ShellScreenshotTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun the_shell_on_a_phone() = capture("shell-phone", width = 360, height = 800, rail = false, twoColumns = false)

    @Test
    fun the_shell_on_a_medium_window() = capture("shell-medium", width = 700, height = 900, rail = true, twoColumns = true)

    @Test
    fun the_shell_on_a_tablet() = capture("shell-tablet", width = 1280, height = 800, rail = true, twoColumns = true)

    private fun capture(name: String, width: Int, height: Int, rail: Boolean, twoColumns: Boolean) {
        compose.setContent {
            MaterialTheme(colorScheme = lightColorScheme()) {
                CompositionLocalProvider(LocalDensity provides Density(1f, 1f)) {
                    Box(Modifier.testTag("shell").size(width.dp, height.dp).background(Color.White)) {
                        Row(Modifier.fillMaxSize()) {
                            if (rail) {
                                NavigationRailPanel(RailDestination.Home) { }
                                VerticalDivider()
                            }
                            Column(
                                modifier = Modifier
                                    .weight(1f)
                                    .fillMaxHeight()
                                    .verticalScroll(rememberScrollState())
                                    .padding(24.dp),
                                horizontalAlignment = Alignment.CenterHorizontally,
                            ) {
                                Spacer(Modifier.height(16.dp))
                                Text("Bonk", style = MaterialTheme.typography.headlineLarge)
                                Spacer(Modifier.height(8.dp))
                                Text(
                                    strings.tagline,
                                    style = MaterialTheme.typography.bodyMedium,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                                Spacer(Modifier.height(24.dp))
                                Card(
                                    Modifier
                                        .widthIn(max = if (twoColumns) 720.dp else 560.dp)
                                        .fillMaxWidth(),
                                ) {
                                    Column(Modifier.padding(24.dp)) {
                                        Dashboard(
                                            summary = ArchiveSummary(sellers = 1, drafts = 4, schemaVersion = 17),
                                            deviceAvailable = false,
                                            notice = null,
                                            wide = twoColumns,
                                            onLock = {},
                                            onForgetDevice = {},
                                            onCustomers = {},
                                            onDrafts = {},
                                            onDocuments = {},
                                            onImport = {},
                                            onBackup = {},
                                            onSettings = {},
                                        )
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        compose.waitForIdle()
        val image = compose.onNodeWithTag("shell").captureToImage().asAndroidBitmap()
        val update = InstrumentationRegistry.getArguments().getString("updateScreenshots") == "1"
        val golden = golden(name)
        if (update || golden == null) {
            write(name, image)
            if (!update) fail("no golden named $name; wrote it to ${directory().absolutePath}")
            return
        }
        assertEquals("$name width", golden.width.toLong(), image.width.toLong())
        assertEquals("$name height", golden.height.toLong(), image.height.toLong())
        val differing = countDifferingPixels(image, golden)
        val ratio = differing.toDouble() / (image.width * image.height)
        assertTrue("$name differs in $differing of ${image.width * image.height} pixels", ratio <= 0.002)
    }

    private fun golden(name: String): Bitmap? = try {
        InstrumentationRegistry.getInstrumentation().context.assets
            .open("screenshots/$name.png")
            .use { BitmapFactory.decodeStream(it) }
    } catch (missing: IOException) {
        null
    }

    private fun directory(): File = InstrumentationRegistry.getInstrumentation()
        .targetContext.getExternalFilesDir("screenshots")
        ?: error("the device has no external files directory")

    private fun write(name: String, image: Bitmap) {
        val directory = directory()
        directory.mkdirs()
        File(directory, "$name.png").outputStream().use {
            image.compress(Bitmap.CompressFormat.PNG, 100, it)
        }
    }

    private fun countDifferingPixels(one: Bitmap, other: Bitmap): Int {
        var differing = 0
        for (y in 0 until one.height) {
            for (x in 0 until one.width) {
                if (channelDistance(one.getPixel(x, y), other.getPixel(x, y)) > 16) differing++
            }
        }
        return differing
    }

    private fun channelDistance(one: Int, other: Int): Int {
        val red = abs((one shr 16 and 0xFF) - (other shr 16 and 0xFF))
        val green = abs((one shr 8 and 0xFF) - (other shr 8 and 0xFF))
        val blue = abs((one and 0xFF) - (other and 0xFF))
        return maxOf(red, green, blue)
    }
}
