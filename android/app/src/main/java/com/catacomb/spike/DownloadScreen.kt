package com.catacomb.spike

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Error
import androidx.compose.material3.AssistChip
import androidx.compose.material3.AssistChipDefaults
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch

/** Download stage for the progress card. */
private sealed interface DlStage {
    data object Idle : DlStage
    data object Running : DlStage
    data class Done(val ok: Boolean, val message: String) : DlStage
}

@Composable
fun DownloadScreen() {
    val context = LocalContext.current
    val prefs = remember { Prefs(context) }
    val scope = rememberCoroutineScope()

    var url by remember { mutableStateOf("") }
    var stage by remember { mutableStateOf<DlStage>(DlStage.Idle) }
    var progress by remember { mutableStateOf(0f) }
    var etaSeconds by remember { mutableStateOf(0L) }
    var statusLine by remember { mutableStateOf("") }
    val log = remember { mutableStateOf("") }

    // Live platform detection through the Rust core, as the URL is typed.
    val platform = remember(url) {
        if (url.isBlank()) null
        else runCatching { RustCore.platformFromUrl(url) }.getOrNull()
    }

    val engineState = Engine.state
    val engineReady = engineState is EngineState.Ready
    val running = stage is DlStage.Running

    Column(
        Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        // ── Engine status banner ────────────────────────────────────────
        EngineBanner(engineState)

        // ── URL + platform chip ─────────────────────────────────────────
        SectionCard("Source") {
            OutlinedTextField(
                value = url,
                onValueChange = { url = it },
                label = { Text("Video / playlist URL") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            if (platform != null) {
                Spacer(Modifier.height(10.dp))
                val name = jsonField(platform, "display_name").ifEmpty { "Unknown" }
                val icon = jsonField(platform, "icon")
                val dir = jsonField(platform, "dir_name")
                AssistChip(
                    onClick = {},
                    label = { Text("$icon  $name  ·  $dir/") },
                    colors = AssistChipDefaults.assistChipColors(
                        labelColor = MaterialTheme.colorScheme.primary,
                    ),
                )
            }
        }

        // ── Action ──────────────────────────────────────────────────────
        val quality = Quality.byId(prefs.quality)
        Button(
            onClick = {
                stage = DlStage.Running
                progress = 0f
                etaSeconds = 0L
                statusLine = "Starting…"
                log.value = ""
                scope.launch {
                    val result = Engine.download(
                        url = url.trim(),
                        destDir = downloadDir(context),
                        quality = quality,
                    ) { p, eta, line ->
                        progress = (p / 100f).coerceIn(0f, 1f)
                        etaSeconds = eta
                        if (line.isNotBlank()) {
                            statusLine = line
                            log.value = (log.value + line + "\n").takeLast(6000)
                        }
                    }
                    stage = DlStage.Done(result.ok, result.message)
                    if (result.ok) progress = 1f
                }
            },
            enabled = engineReady && url.isNotBlank() && !running,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text(if (running) "Downloading…" else "Download  ·  ${quality.label}")
        }

        // ── Progress card ───────────────────────────────────────────────
        AnimatedVisibility(visible = stage !is DlStage.Idle) {
            ProgressCard(stage, progress, etaSeconds, statusLine)
        }

        // ── Log ─────────────────────────────────────────────────────────
        if (log.value.isNotEmpty()) {
            SectionCard("Log") {
                Text(
                    log.value,
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

@Composable
private fun EngineBanner(state: EngineState) {
    when (state) {
        is EngineState.Initializing -> SectionCard("Engine") {
            Row(verticalAlignment = Alignment.CenterVertically) {
                CircularProgressIndicator(
                    modifier = Modifier.height(18.dp),
                    strokeWidth = 2.dp,
                )
                Spacer(Modifier.height(0.dp))
                Text(
                    "  Preparing yt-dlp engine (first launch extracts Python)…",
                    style = MaterialTheme.typography.bodyMedium,
                )
            }
        }
        is EngineState.Ready -> SectionCard("Engine") {
            Text(
                "yt-dlp ready · ${state.version}",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.secondary,
            )
        }
        is EngineState.Failed -> SectionCard("Engine") {
            Text(
                "Engine failed to initialise: ${state.message}",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.tertiary,
            )
        }
    }
}

/** The improved determinate progress indicator: animated bar + %, ETA, status. */
@Composable
private fun ProgressCard(stage: DlStage, progress: Float, etaSeconds: Long, statusLine: String) {
    val animated by animateFloatAsState(targetValue = progress, label = "dl-progress")
    val done = stage as? DlStage.Done

    SectionCard(if (done != null) "Result" else "Downloading") {
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                "${(animated * 100).toInt()}%",
                style = MaterialTheme.typography.headlineSmall,
                color = MaterialTheme.colorScheme.primary,
            )
            when {
                done?.ok == true -> Icon(
                    Icons.Filled.CheckCircle, contentDescription = "done",
                    tint = MaterialTheme.colorScheme.secondary,
                )
                done != null -> Icon(
                    Icons.Filled.Error, contentDescription = "error",
                    tint = MaterialTheme.colorScheme.tertiary,
                )
                etaSeconds > 0 -> Text(
                    "ETA ${formatEta(etaSeconds)}",
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        Spacer(Modifier.height(10.dp))
        LinearProgressIndicator(
            progress = { animated },
            modifier = Modifier
                .fillMaxWidth()
                .height(10.dp),
            trackColor = MaterialTheme.colorScheme.surface,
            strokeCap = androidx.compose.ui.graphics.StrokeCap.Round,
        )
        Spacer(Modifier.height(10.dp))
        Text(
            done?.message ?: statusLine.ifEmpty { "Working…" },
            style = MaterialTheme.typography.bodyMedium,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
            color = when {
                done?.ok == true -> MaterialTheme.colorScheme.secondary
                done != null -> MaterialTheme.colorScheme.tertiary
                else -> MaterialTheme.colorScheme.onSurface
            },
        )
    }
}

private fun formatEta(seconds: Long): String {
    if (seconds <= 0) return "—"
    val m = seconds / 60
    val s = seconds % 60
    return if (m > 0) "${m}m ${s}s" else "${s}s"
}

// Suppress unused shape import guard.
@Suppress("unused")
private val cardShape = RoundedCornerShape(16.dp)
