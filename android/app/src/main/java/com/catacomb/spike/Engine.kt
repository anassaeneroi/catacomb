package com.catacomb.spike

import android.app.Application
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.yausername.ffmpeg.FFmpeg
import com.yausername.youtubedl_android.YoutubeDL
import com.yausername.youtubedl_android.YoutubeDLRequest
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File

/** Lifecycle of the bundled on-device yt-dlp engine. */
sealed interface EngineState {
    data object Initializing : EngineState
    data class Ready(val version: String) : EngineState
    data class Failed(val message: String) : EngineState
}

/**
 * Wraps the bundled youtubedl-android engine (Python + yt-dlp + ffmpeg).
 *
 * `init()` extracts the bundled Python environment on first launch (slow, so it
 * runs off the main thread) and flips [state] to [EngineState.Ready]. The UI
 * observes [state] to enable the Download button and show status.
 */
object Engine {
    var state by mutableStateOf<EngineState>(EngineState.Initializing)
        private set

    private val io = CoroutineScope(Dispatchers.IO)

    fun init(app: Application) {
        io.launch {
            state = try {
                YoutubeDL.getInstance().init(app)
                FFmpeg.getInstance().init(app)
                val v = runCatching { YoutubeDL.getInstance().version(app) }.getOrNull() ?: "unknown"
                android.util.Log.i("CatacombEngine", "yt-dlp ready: $v")
                EngineState.Ready(v)
            } catch (t: Throwable) {
                android.util.Log.e("CatacombEngine", "init failed", t)
                EngineState.Failed(t.message ?: t.javaClass.simpleName)
            }
        }
    }

    val isReady: Boolean get() = state is EngineState.Ready

    /** Result of a download attempt. */
    data class DownloadResult(val ok: Boolean, val message: String)

    /**
     * Run a download. Must be called from a coroutine; the blocking yt-dlp call
     * is dispatched to IO. [onProgress] is invoked with (percent 0..100, eta
     * seconds, latest output line) on the engine's callback thread.
     */
    suspend fun download(
        url: String,
        destDir: File,
        quality: Quality,
        onProgress: (Float, Long, String) -> Unit,
    ): DownloadResult = withContext(Dispatchers.IO) {
        if (state !is EngineState.Ready) {
            return@withContext DownloadResult(false, "Engine not ready")
        }
        if (!destDir.exists()) destDir.mkdirs()
        try {
            val request = YoutubeDLRequest(url)
            request.addOption("-o", "${destDir.absolutePath}/%(title).200s.%(ext)s")
            request.addOption("-f", quality.format)
            if (quality.audioOnly) {
                request.addOption("-x")
                request.addOption("--audio-format", "m4a")
            }
            // Keep runs bounded and quiet-ish for the log view.
            request.addOption("--no-playlist")
            request.addOption("--newline")
            val processId = "catacomb-${System.currentTimeMillis()}"
            YoutubeDL.getInstance().execute(request, processId) { progress, etaSeconds, line ->
                onProgress(progress, etaSeconds, line)
            }
            DownloadResult(true, "Download complete")
        } catch (t: Throwable) {
            DownloadResult(false, t.message ?: t.javaClass.simpleName)
        }
    }
}
