# Theming & appearance

catacomb's desktop UI is built on [egui](https://github.com/emilk/egui) and
ships with **nineteen themes** spanning four moods, plus a three-mode video
list layout you can toggle on the fly.

## Themes

Pick a theme from **Settings → Theme**. It persists across restarts.

| Category | Themes | Vibe |
|---|---|---|
| **Catacomb** (default) | Dark, Light | The out-of-box experience — fully tuned, not stock egui. |
| **Goth** | Dracula, Vampire, Witching Hour, Cemetery Moss, Emo: Nocturnal, Emo: Coffin, Emo: Scene Queen | Macabre, regal, or neon-goth. Where the app's name lives. |
| **Neon / retro** | Cyberpunk, Synthwave '84, Vaporwave | Magenta-on-black, sunset gradients, pastel aesthetic. |
| **Dev palettes** | Nord, Gruvbox, Tokyo Night | The beloved, legible, community-tested schemes. |
| **Cozy (light)** | Paper, Honey, Candlelight | Warm, low-glare counterpoints to the dark themes. |
| **Trans** | Trans | Trans flag colours — light blue, pink, white. |

Every theme drives the full surface: panel fills, widget states (hovered,
active, open), selection rings, hyperlink and accent colours, and the
warning/error tones. The selection, "now playing", and bulk-selection rings
all derive from the active theme's accents — so in *Witching Hour* a selected
video glows arcane-violet, in *Honey* it glows amber.

## Video list layouts

The video list has three render modes, switchable live from a 3-segment
toggle in the list header:

- **List ☰** — dense horizontal rows (thumbnail left, title + metadata
  right). Best for large libraries; the default.
- **Card ▢** — the same horizontal layout, but each row is a rounded card
  with a hover lift. A more modern, contained feel without losing density.
- **Grid ◫** — YouTube/Plex-style vertical cards (thumbnail on top, title
  and metadata below), with the column count adapting to the window width.
  Best for browsing by eye.

The toggle is **global by default, overridable per view**. Set your preferred
mode once in **Settings** and it applies everywhere — but if you want, say,
Grid for *All Videos* and List for a specific channel, flip the toggle while
in that view and the override is remembered separately. A view with no
override falls back to the global default.

Card density (thumbnail size and spacing) is separately adjustable via the
existing density slider in Settings, and applies to all three modes.

## Placeholders

Videos without a downloaded thumbnail show a consistent placeholder — a
theme-tinted gradient with a subtle glyph — rather than a bare grey box.
