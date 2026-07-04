package com.catacomb.spike

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext

/** Bottom-navigation destinations. */
private enum class Screen(val label: String, val icon: ImageVector) {
    Download("Download", Icons.Filled.Download),
    Files("Files", Icons.Filled.Folder),
    Settings("Settings", Icons.Filled.Settings),
}

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent { CatacombRoot() }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun CatacombRoot() {
    val context = LocalContext.current
    val prefs = remember { Prefs(context) }

    // Hoisted app state — theme + current screen survive recomposition.
    var themeId by remember { mutableStateOf(prefs.themeId) }
    var current by remember { mutableStateOf(Screen.Download) }
    val theme = themeById(themeId)

    CatacombTheme(theme) {
        Scaffold(
            topBar = {
                TopAppBar(
                    title = { Text("Catacomb · ${current.label}") },
                    colors = TopAppBarDefaults.topAppBarColors(
                        containerColor = MaterialTheme.colorScheme.surface,
                        titleContentColor = MaterialTheme.colorScheme.primary,
                    ),
                )
            },
            bottomBar = {
                NavigationBar(containerColor = MaterialTheme.colorScheme.surface) {
                    Screen.entries.forEach { screen ->
                        NavigationBarItem(
                            selected = current == screen,
                            onClick = { current = screen },
                            icon = { Icon(screen.icon, contentDescription = screen.label) },
                            label = { Text(screen.label) },
                        )
                    }
                }
            },
        ) { inner ->
            Box(Modifier.fillMaxSize().padding(inner)) {
                when (current) {
                    Screen.Download -> DownloadScreen()
                    Screen.Files -> FilesScreen()
                    Screen.Settings -> SettingsScreen(
                        currentThemeId = themeId,
                        currentQuality = prefs.quality,
                        onThemeSelected = { id ->
                            themeId = id
                            prefs.themeId = id
                        },
                        onQualitySelected = { q -> prefs.quality = q },
                    )
                }
            }
        }
    }
}
