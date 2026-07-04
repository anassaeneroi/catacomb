package com.catacomb.spike

import android.content.Context

/** Thin SharedPreferences wrapper for the handful of persisted settings. */
class Prefs(context: Context) {
    private val sp = context.getSharedPreferences("catacomb", Context.MODE_PRIVATE)

    var themeId: String
        get() = sp.getString("theme", "dark") ?: "dark"
        set(v) = sp.edit().putString("theme", v).apply()

    var quality: String
        get() = sp.getString("quality", "best") ?: "best"
        set(v) = sp.edit().putString("quality", v).apply()
}

/** Download quality presets → yt-dlp format selectors. */
enum class Quality(val id: String, val label: String, val format: String, val audioOnly: Boolean) {
    BEST("best", "Best available", "bestvideo*+bestaudio/best", false),
    P1080("1080p", "1080p", "bestvideo[height<=1080]+bestaudio/best[height<=1080]", false),
    P720("720p", "720p", "bestvideo[height<=720]+bestaudio/best[height<=720]", false),
    AUDIO("audio", "Audio only (m4a)", "bestaudio/best", true);

    companion object {
        fun byId(id: String) = entries.firstOrNull { it.id == id } ?: BEST
    }
}
