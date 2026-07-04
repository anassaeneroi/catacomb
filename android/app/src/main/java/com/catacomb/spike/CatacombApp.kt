package com.catacomb.spike

import android.app.Application

/** Kicks off the bundled yt-dlp engine init as early as possible. */
class CatacombApp : Application() {
    override fun onCreate() {
        super.onCreate()
        Engine.init(this)
    }
}
