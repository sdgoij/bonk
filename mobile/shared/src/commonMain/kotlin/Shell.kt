package com.github.sdgoij.bonk

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp

/** Where the navigation rail can take an open archive. */
enum class RailDestination {
    Home,
    Customers,
    Drafts,
    Documents,
    Settings,
}

/**
 * The permanent navigation a window from 600 dp gets.
 *
 * Words rather than icons, so the rail reads in the app's own language and needs
 * no icon font; each entry is at least a 48 dp target.
 */
@Composable
fun NavigationRailPanel(destination: RailDestination, onSelect: (RailDestination) -> Unit) {
    Surface(
        color = MaterialTheme.colorScheme.surfaceContainer,
        modifier = Modifier.fillMaxHeight().width(200.dp),
    ) {
        Column(
            modifier = Modifier.fillMaxHeight().padding(12.dp),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Text("Bonk", style = MaterialTheme.typography.titleLarge, modifier = Modifier.padding(12.dp))
            Spacer(Modifier.height(8.dp))
            RailEntry(strings.home, destination == RailDestination.Home) { onSelect(RailDestination.Home) }
            RailEntry(strings.customers, destination == RailDestination.Customers) {
                onSelect(RailDestination.Customers)
            }
            RailEntry(strings.drafts, destination == RailDestination.Drafts) { onSelect(RailDestination.Drafts) }
            RailEntry(strings.documents, destination == RailDestination.Documents) {
                onSelect(RailDestination.Documents)
            }
            RailEntry(strings.settings, destination == RailDestination.Settings) {
                onSelect(RailDestination.Settings)
            }
        }
    }
}

@Composable
private fun RailEntry(label: String, selected: Boolean, onClick: () -> Unit) {
    TextButton(
        onClick = onClick,
        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp),
        colors = if (selected) ButtonDefaults.filledTonalButtonColors() else ButtonDefaults.textButtonColors(),
    ) {
        Text(label, modifier = Modifier.fillMaxWidth(), textAlign = TextAlign.Start)
    }
}
