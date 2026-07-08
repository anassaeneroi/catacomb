# Mobile scaling fixes + PWA support — design

Date: 2026-07-05. Scope: `src/web_ui/` + `src/web.rs` + `tests/api.rs`.

## Problem

1. **Mobile scaling regressions.** The "private cinematheque" design layer was
   appended *after* the `@media(max-width:640px)` / `@media(max-width:380px)`
   blocks in `src/web_ui/index.html`. Media queries add no specificity, so the
   design layer's base rules (header padding, `.vid-wrap video` max-height, …)
   override the mobile rules by source order. The design layer also introduced
   whole components with **no** mobile rules at all: the custom player
   (`.pl-*`), command palette (`.cp-*`), stats dashboard (`.sx-*`) and
   maintenance console (`.mx-*`).
2. The viewport meta lacks `viewport-fit=cover`, so every
   `env(safe-area-inset-*)` the CSS relies on evaluates to 0 on iPhones.
3. Text inputs are 13px, which triggers iOS Safari's automatic focus-zoom —
   the classic "the page scales itself when I tap search" bug.
4. No PWA support: no manifest, no service worker, no installability, and the
   bundled `icon.png` is a YouTube-logo lookalike unsuitable for an installed
   app icon.

## Approach chosen

**Mobile fixes (index.html only, no JS-visible id/class changes):**

- Add `viewport-fit=cover` to the viewport meta.
- Move both mobile media-query blocks to the **end** of the stylesheet, after
  the design layer, with a comment stating the ordering invariant. This is the
  structural fix; alternatives (raising specificity, `!important`) were
  rejected as fragile.
- Inside the ≤640px block: bump text inputs/selects to 16px (kills iOS
  focus-zoom), and add rules for the new components — full-bleed command
  palette with `dvh` sizing, clamped stats-hero numerals, safe-area padding.
- New `@media(pointer:coarse)` block for touch ergonomics independent of
  width: taller scrub track, always-visible seek thumb, ≥40px player buttons,
  hide the hover-oriented volume slider (phones use hardware volume).

**PWA (additive, all static assets embedded like the existing HTML):**

- `src/web_ui/manifest.webmanifest` — name/short_name Catacomb, `display:
  standalone`, `start_url: /`, `id: /`, theme/background `#1a1a2e`, icons
  192/512 + maskable 512.
- New Catacomb-branded icon: `src/web_ui/icon.svg` (crimson catacomb arch on
  the app's dark palette) rendered to `icon-192.png`, `icon-512.png`,
  `icon-maskable-512.png`, `apple-touch-icon.png` (180px, opaque). SVG source
  checked in; PNGs regenerated via `rsvg-convert`.
- `src/web_ui/sw.js` — deliberately minimal service worker. Intercepts **only**
  GET navigations to `/` (network-first, falling back to a cached copy when
  offline) and the static icon/manifest paths (cache-first). It never touches
  `/api/*`, `/ws/*`, `/files/*`, `/music-files/*`, or `/feed*` — no interference
  with auth, streaming ranges, or WebSockets. Served `no-store` so upgrades
  propagate on next load, matching the existing "no stale UI" policy.
- `src/web.rs` — `include_str!`/`include_bytes!` consts + routes for
  `/manifest.webmanifest`, `/sw.js`, `/icons/*`, `/apple-touch-icon.png`;
  these paths are allowlisted (GET, static, nothing sensitive) in
  `auth_middleware` so the browser can fetch them pre-login.
- `index.html` head: manifest link, `theme-color` meta (kept in sync with the
  active theme's `--panel` by `applyTheme`), apple-touch-icon link, iOS
  standalone metas; SW registration guarded by `'serviceWorker' in navigator`.

## Testing

- `tests/api.rs`: new test asserting `/manifest.webmanifest`, `/sw.js`, and an
  icon route return 200 with sane content types, and that they are reachable
  without auth when a password is set (follows the existing curl harness).
- `node --check` on the extracted inline script and on `sw.js`.
- Headless Chromium screenshots at phone viewports (375×812, 360×740) before
  and after, against a real `--web` instance.

## Out of scope

Offline caching of library data/media, push notifications, background sync,
replacing `icon.png` used by the desktop `.desktop` entry (flagged for a
follow-up since it's a YouTube trademark lookalike).
