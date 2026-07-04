package com.catacomb.spike

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp

/**
 * Settings section: theme picker (all desktop themes), default quality, engine
 * info, and a live demo of the shared Rust core.
 */
@Composable
fun SettingsScreen(
    currentThemeId: String,
    currentQuality: String,
    onThemeSelected: (String) -> Unit,
    onQualitySelected: (String) -> Unit,
) {
    var quality = currentQuality
    Column(
        Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        // ── Theme picker ────────────────────────────────────────────────
        SectionCard("Appearance") {
            Text(
                "Theme",
                style = MaterialTheme.typography.labelLarge,
                modifier = Modifier.padding(bottom = 10.dp),
            )
            ThemeSwatchGrid(currentThemeId, onThemeSelected)
        }

        // ── Default quality ─────────────────────────────────────────────
        SectionCard("Downloads") {
            Text(
                "Default quality",
                style = MaterialTheme.typography.labelLarge,
                modifier = Modifier.padding(bottom = 10.dp),
            )
            LazyRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                items(Quality.entries.size) { idx ->
                    val q = Quality.entries[idx]
                    FilterChip(
                        selected = quality == q.id,
                        onClick = {
                            quality = q.id
                            onQualitySelected(q.id)
                        },
                        label = { Text(q.label) },
                        leadingIcon = if (quality == q.id) {
                            { Icon(Icons.Filled.Check, contentDescription = null, Modifier.size(18.dp)) }
                        } else null,
                    )
                }
            }
        }

        // ── Engine ──────────────────────────────────────────────────────
        SectionCard("Engine") {
            val s = Engine.state
            val line = when (s) {
                is EngineState.Ready -> "yt-dlp ${s.version} · bundled Python + ffmpeg"
                is EngineState.Initializing -> "Initialising…"
                is EngineState.Failed -> "Failed: ${s.message}"
            }
            Text(line, style = MaterialTheme.typography.bodyMedium)
        }

        // ── Rust core demo ──────────────────────────────────────────────
        SectionCard("Rust core (JNI)") {
            val probe = runCatching {
                val p = RustCore.platformFromUrl("https://youtu.be/dQw4w9WgXcQ")
                val cls = RustCore.classifyError("ERROR: HTTP Error 429: Too Many Requests")
                val cues = RustCore.vttParse("WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nhi\n")
                "platformFromUrl → $p\n\nclassifyError → $cls\n\nvttParse → $cues"
            }.getOrElse { "Rust core error: ${it.message}" }
            Text(
                probe,
                style = MaterialTheme.typography.bodySmall,
                fontFamily = FontFamily.Monospace,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }

        // ── About ───────────────────────────────────────────────────────
        SectionCard("About") {
            Text(
                "Catacomb Android — Stage-1 prototype.\n" +
                    "Bundled on-device yt-dlp engine + shared Rust core over JNI.\n" +
                    "AGPL-3.0.",
                style = MaterialTheme.typography.bodyMedium,
            )
        }
    }
}

/** A wrapping grid of theme swatches; tapping one applies it live. */
@Composable
private fun ThemeSwatchGrid(currentThemeId: String, onThemeSelected: (String) -> Unit) {
    // Simple manual wrap: 3 columns.
    val cols = 3
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        THEMES.chunked(cols).forEach { rowThemes ->
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                rowThemes.forEach { t ->
                    ThemeSwatch(
                        theme = t,
                        selected = t.id == currentThemeId,
                        onClick = { onThemeSelected(t.id) },
                        modifier = Modifier.weight(1f),
                    )
                }
                // pad the last row so weights stay even
                repeat(cols - rowThemes.size) { Spacer(Modifier.weight(1f)) }
            }
        }
    }
}

@Composable
private fun ThemeSwatch(
    theme: CatTheme,
    selected: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val borderColor =
        if (selected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outline
    Column(
        modifier
            .clip(RoundedCornerShape(12.dp))
            .border(if (selected) 2.dp else 1.dp, borderColor, RoundedCornerShape(12.dp))
            .background(theme.bg)
            .clickable(onClick = onClick)
            .padding(10.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        // Accent dots preview the theme's primary/secondary/tertiary.
        Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            Dot(theme.primary)
            Dot(theme.secondary)
            Dot(theme.tertiary)
            if (selected) {
                Spacer(Modifier.width(2.dp))
                Icon(
                    Icons.Filled.Check,
                    contentDescription = "selected",
                    tint = theme.primary,
                    modifier = Modifier.size(14.dp),
                )
            }
        }
        Text(
            theme.label,
            style = MaterialTheme.typography.bodySmall,
            color = if (theme.dark) Color(0xFFF4F4F8) else Color(0xFF101014),
            maxLines = 1,
        )
    }
}

@Composable
private fun Dot(color: Color) {
    Box(Modifier.size(14.dp).clip(CircleShape).background(color))
}
