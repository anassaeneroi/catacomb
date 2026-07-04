package com.catacomb.spike

import android.content.Context
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import org.json.JSONObject
import java.io.File

/** App-private external download directory (no runtime permission needed). */
fun downloadDir(context: Context): File =
    context.getExternalFilesDir("downloads") ?: File(context.filesDir, "downloads")

/** Safe extract of a string field from a small JSON object string. */
fun jsonField(json: String, key: String): String =
    runCatching { JSONObject(json).optString(key, "") }.getOrDefault("")

/** A titled surface card used to group content into sections. */
@Composable
fun SectionCard(
    title: String,
    modifier: Modifier = Modifier,
    content: @Composable () -> Unit,
) {
    Card(
        modifier = modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant,
            contentColor = MaterialTheme.colorScheme.onSurface,
        ),
        elevation = CardDefaults.cardElevation(defaultElevation = 2.dp),
    ) {
        Column(Modifier.padding(16.dp)) {
            Text(
                title,
                style = MaterialTheme.typography.titleMedium,
                color = MaterialTheme.colorScheme.primary,
                modifier = Modifier.padding(bottom = 10.dp),
            )
            content()
        }
    }
}

/** Standard screen content padding. */
val screenPadding = PaddingValues(16.dp)
