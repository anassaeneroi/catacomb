package com.catacomb.spike

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

/**
 * One selectable theme, ported 1:1 from the desktop `src/theme.rs` palettes so
 * the Android build offers the same look. `bg`/`surface` come from egui's
 * `panel_fill`/`window_fill`; `primary`/`secondary`/`tertiary` are the desktop
 * `accents_for()` accent/success/warning colours.
 */
data class CatTheme(
    val id: String,
    val label: String,
    val dark: Boolean,
    val bg: Color,
    val surface: Color,
    val primary: Color,
    val secondary: Color,
    val tertiary: Color,
)

private fun c(hex: Long) = Color(0xFF000000 or hex)

/** All desktop themes, in the same order as `theme.rs::THEMES`. */
val THEMES: List<CatTheme> = listOf(
    CatTheme("dark", "Dark", true, c(0x14141a), c(0x1a1a22), c(0x7aa2f7), c(0x9ece6a), c(0xbb9af7)),
    CatTheme("light", "Light", false, c(0xf6f4ef), c(0xfefcf7), c(0x2a5db0), c(0x2e7d32), c(0x8e44ad)),
    CatTheme("dracula", "Dracula", true, c(0x282a36), c(0x2f3142), c(0xbd93f9), c(0x50fa7b), c(0xff79c6)),
    CatTheme("trans", "Trans", false, c(0xe8f7fd), c(0xfef0f4), c(0x2288cc), c(0x2e7d32), c(0xf7a8b8)),
    CatTheme("emo-nocturnal", "Emo: Nocturnal", true, c(0x0a0a0a), c(0x0d0d0d), c(0xff0090), c(0x39ff14), c(0x00f5ff)),
    CatTheme("emo-coffin", "Emo: Coffin", true, c(0x0d0009), c(0x110010), c(0xb01a1a), c(0x39ff14), c(0xcc2222)),
    CatTheme("emo-scene-queen", "Emo: Scene Queen", true, c(0x080818), c(0x0a0a1e), c(0x39ff14), c(0xff00ff), c(0x00f5ff)),
    CatTheme("cemetery-moss", "Cemetery Moss", true, c(0x1a1f1a), c(0x1e241e), c(0x7a8a6a), c(0x8faf6a), c(0x9a9a8a)),
    CatTheme("vampire", "Vampire", true, c(0x0d0006), c(0x120008), c(0xc9a227), c(0xd11a2b), c(0x8c0a1e)),
    CatTheme("witching-hour", "Witching Hour", true, c(0x0a0a1f), c(0x0e0e28), c(0x8a6aab), c(0xb0b8d0), c(0x5a5aaa)),
    CatTheme("cyberpunk", "Cyberpunk", true, c(0x0a0a12), c(0x0e0e18), c(0x00fff5), c(0x39ff14), c(0xff003c)),
    CatTheme("synthwave", "Synthwave '84", true, c(0x2b0a3d), c(0x330a48), c(0xff2a6d), c(0x05d9e8), c(0xf649c9)),
    CatTheme("vaporwave", "Vaporwave", true, c(0x1a0033), c(0x200040), c(0x01cdfe), c(0x05ffa1), c(0xff71ce)),
    CatTheme("nord", "Nord", true, c(0x2e3440), c(0x3b4252), c(0x88c0d0), c(0xa3be8c), c(0xebcb8b)),
    CatTheme("gruvbox", "Gruvbox", true, c(0x282828), c(0x32302f), c(0xfe8019), c(0xb8bb26), c(0xd3869b)),
    CatTheme("tokyo-night", "Tokyo Night", true, c(0x1a1b26), c(0x20202f), c(0x7aa2f7), c(0x9ece6a), c(0xbb9af7)),
    CatTheme("paper", "Paper", false, c(0xf4ecd8), c(0xf8f0dc), c(0x8b6f3a), c(0x4a6b3a), c(0xc4a86a)),
    CatTheme("honey", "Honey", false, c(0xfff4e0), c(0xfff9ec), c(0xd98a1e), c(0xb87420), c(0xc97b4a)),
    CatTheme("candlelight", "Candlelight", false, c(0xf2e6d0), c(0xf6ecd9), c(0xa8703a), c(0x6b3f1e), c(0xc79a5a)),
)

fun themeById(id: String): CatTheme = THEMES.firstOrNull { it.id == id } ?: THEMES[0]

/** Pick black/white text for best contrast against [bg]. */
private fun onColor(bg: Color): Color {
    val l = 0.299 * bg.red + 0.587 * bg.green + 0.114 * bg.blue
    return if (l > 0.55) Color(0xFF101014) else Color(0xFFF4F4F8)
}

private fun elevate(base: Color, dark: Boolean): Color {
    // A slightly lifted surface variant for cards/inputs.
    val f = if (dark) 0.06f else -0.04f
    fun ch(v: Float) = (v + f).coerceIn(0f, 1f)
    return Color(ch(base.red), ch(base.green), ch(base.blue), 1f)
}

private val AppType = Typography(
    headlineSmall = TextStyle(fontWeight = FontWeight.Bold, fontSize = 22.sp),
    titleLarge = TextStyle(fontWeight = FontWeight.SemiBold, fontSize = 20.sp),
    titleMedium = TextStyle(fontWeight = FontWeight.SemiBold, fontSize = 16.sp),
    labelLarge = TextStyle(fontWeight = FontWeight.SemiBold, fontSize = 14.sp),
)

/** Wrap [content] in a Material3 theme built from the selected [theme]. */
@Composable
fun CatacombTheme(theme: CatTheme, content: @Composable () -> Unit) {
    val onBg = onColor(theme.bg)
    val onSurface = onColor(theme.surface)
    val surfaceVariant = elevate(theme.surface, theme.dark)
    val scheme = if (theme.dark) {
        darkColorScheme(
            primary = theme.primary,
            onPrimary = onColor(theme.primary),
            secondary = theme.secondary,
            onSecondary = onColor(theme.secondary),
            tertiary = theme.tertiary,
            onTertiary = onColor(theme.tertiary),
            background = theme.bg,
            onBackground = onBg,
            surface = theme.surface,
            onSurface = onSurface,
            surfaceVariant = surfaceVariant,
            onSurfaceVariant = onSurface.copy(alpha = 0.85f),
            outline = onSurface.copy(alpha = 0.35f),
        )
    } else {
        lightColorScheme(
            primary = theme.primary,
            onPrimary = onColor(theme.primary),
            secondary = theme.secondary,
            onSecondary = onColor(theme.secondary),
            tertiary = theme.tertiary,
            onTertiary = onColor(theme.tertiary),
            background = theme.bg,
            onBackground = onBg,
            surface = theme.surface,
            onSurface = onSurface,
            surfaceVariant = surfaceVariant,
            onSurfaceVariant = onSurface.copy(alpha = 0.85f),
            outline = onSurface.copy(alpha = 0.35f),
        )
    }
    MaterialTheme(colorScheme = scheme, typography = AppType, content = content)
}

// Keep the unused-import guard away from isSystemInDarkTheme if a future
// "system" theme is added; referenced here so the import stays intentional.
@Suppress("unused")
private val darkFollowSystem: @Composable () -> Boolean = { isSystemInDarkTheme() }
