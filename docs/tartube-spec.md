# Tartube Specification

A structured analysis of [Tartube v2.5.231](https://github.com/axcore/tartube)
(2026-05-24), our project's primary feature benchmark. Written to give
catacomb a concrete target to work toward — every section ends with a
"What we can do" note about whether to adopt, adapt, or skip.

Tartube codebase: ~146,000 lines of Python 3 across 19 modules under
`tartube/`, plus locale, icons, sounds, screenshots. GTK 3 frontend.
GPL-3-or-later. Single-user desktop application.

## 1. Module map

| File | Lines | Responsibility |
| --- | --- | --- |
| `mainapp.py` | 30,565 | The `TartubeApp` Gtk.Application — owns 1,601 instance variables / 791 boolean toggles, every config field, every operation entry point. |
| `mainwin.py` | 42,074 | `MainWin` Gtk window + ~80 specialised `Gtk.Dialog` subclasses + the Video Catalogue widgets. |
| `config.py` | 36,659 | Preferences and per-object editor windows (per-channel options, FFmpeg options, scheduled actions, custom-download recipes). |
| `downloads.py` | 12,188 | Download orchestration: `DownloadManager` (worker pool), `DownloadList` (queue), `DownloadWorker`, three downloader strategies (`VideoDownloader`, `ClipDownloader`, `StreamDownloader`), JSON fetchers. |
| `ttutils.py` | 5,015 | Cross-cutting helpers: path manipulation, livestream-message parsing, locale, sorting. |
| `media.py` | 4,684 | Hierarchical data model: `Video`, `Channel`, `Playlist`, `Folder`, `Scheduled`, plus `GenericMedia` / `GenericContainer` / `GenericRemoteContainer` mixins. |
| `wizwin.py` | 4,113 | Setup wizard, YouTube subscriptions import wizard, tutorial wizard. |
| `options.py` | 2,284 | `OptionsManager`: a 164-field bag of yt-dlp / youtube-dl flags that can be attached to any Video / Channel / Playlist / Folder, plus an `OptionsParser` that compiles them into a yt-dlp CLI invocation. |
| `tidy.py` | 1,636 | `TidyManager`: verify-on-disk operation that finds phantom DB records, missing files, orphaned thumbnails. |
| `ffmpeg_tartube.py` | 1,303 | `FFmpegManager` + `FFmpegOptionsManager`: post-processing pipeline with named presets. |
| `formats.py` | 1,289 | Canonical lists: SponsorBlock categories/actions, audio/video format codes, language codes, livestream message patterns. |
| `updates.py` | 1,180 | `UpdateManager`: self-update yt-dlp via pip / system package / git inside Tartube. |
| `process.py` | 1,012 | `ProcessManager`: chains FFmpeg invocations after a download completes. |
| `info.py` | 671 | `InfoManager`: yt-dlp `--dump-json` + capability probes, version checks. |
| `refresh.py` | 617 | `RefreshManager`: re-import disk state into the media registry. |
| `dialogue.py` | 522 | Small/common dialog primitives (yes-no, message, OK). |
| `classes.py` | small | Misc shared helpers. |
| `files.py` | 167 | File-existence checks. |
| `xdg_tartube.py` | 183 | XDG base-directory resolution for config/data paths. |

Auxiliary directories:

- `locale/` — `gettext` translations (many languages).
- `icons/`, `sounds/` — assets.
- `nsis/`, `pack/` — Windows installer + per-distro packaging.
- `docs/` — manual pages.

**What we can do:** Our entire codebase is ~9k lines of Rust right now.
Tartube's volume comes from (a) the sheer number of UI dialogs (~80) and
(b) `OptionsManager` cascading through every code path. Both are
mechanical work to match, not architectural blockers.

## 2. Data model

Tartube's central insight is a **container hierarchy** where everything
nests:

```
Folder ─┬─ Folder (recursive)
        ├─ Channel ── Video
        └─ Playlist ── Video
```

`Folder` is the freeform organisation layer (user-created and
system-created — see §2.1). `Channel` and `Playlist` are "remote
containers" with a `source` URL and download semantics. `Video` is a
leaf with on-disk file references plus state flags.

### 2.1 System folders

Tartube ships with a fixed set of system `Folder` objects users can't
delete:

- **All Videos** — every video across the registry.
- **Bookmarks** — videos with `bookmark_flag = True`.
- **Favourite Videos** — videos with `fav_flag = True`, inherited from
  any ancestor.
- **Live Videos** — videos with `live_mode > 0`.
- **Missing Videos** — videos that existed but are gone from the source.
- **New Videos** — downloaded but not yet watched.
- **Recent Videos** — last N downloads.
- **Waiting Videos** — videos manually queued for later attention.
- **Temporary Videos** — one-off Classic-Mode downloads.

System folders are virtual: they don't own their videos, they're
filtered views of the registry by flag. Adding a video to "Bookmarks"
sets `bookmark_flag` on the underlying video; removing the bookmark
removes the video from the folder.

**What we can do:** This pattern is *exactly* the activity feed / smart
filter mechanism we should adopt. Our `SidebarView::Recent` is one of
these, hand-coded. Generalising into a `SmartFolder` enum with a
predicate would let us add Bookmarks / Favourites / Waiting / New
mechanically. Tartube parity-and-then-some.

### 2.2 `media.Video` field inventory

Beyond what we have today, Tartube tracks per video:

| Field | Purpose | Have it? |
| --- | --- | --- |
| `dbid` | Integer primary key | No (we key by yt-dlp ID string) |
| `vid` | Extractor's video ID | Yes (`Video::id`) |
| `name` vs `nickname` | Filename vs display title | No — one `title` field |
| `natname` | Natural-sort key | No — sorted at query time |
| `source` | Original URL | No on `Video`; we keep `source_url` per Channel |
| `file_name`, `file_ext` | Separate parts | No — one `PathBuf` |
| `file_size` | bytes | Yes |
| `upload_time` (unix) | Upload date as seconds | Partial — we have `YYYYMMDD` string |
| `receive_time` (unix) | When Tartube downloaded it | We have `mtime_unix`, equivalent |
| `duration` (int seconds) | Yes (`duration_secs`) |
| `index` | Position in channel/playlist | No |
| `author` | Uploader (distinct from channel) | No |
| `descrip`, `short` | Full + summary description | We have full via sidecar |
| `live_mode` (0/1/2) | Not-live / scheduled / broadcasting | We have `live: bool` at download time only |
| `live_debut_flag` | YouTube "premiere" videos | No |
| `was_live_flag` | Once-was-live, prevents re-marking | No |
| `live_time` | Approximate start time | No |
| `live_msg` | Parsed livestream message | No |
| `archive_flag` | Skip auto-deletion | No |
| `bookmark_flag` | Bookmarked | No |
| `fav_flag` | Favourited (cascades from ancestors) | No |
| `missing_flag` | Removed from source after download | No (we have a maintenance scan) |
| `new_flag` | Downloaded but unwatched | Inverse of our `watched` set |
| `waiting_flag` | Manually queued for attention | No |
| `block_flag` | Censored / age-restricted | No |
| `split_flag` | Clip extracted from a parent video | No |
| `orig_parent_obj` | Where the video originally came from | No |
| `subs_list` | Available subtitle languages | Yes (`subtitles: Vec<Subtitle>`) |
| `stamp_list` | Chapter timestamps (manual or extracted) | Partial — we read chapters from info.json, no manual editing |
| `slice_list` | Per-video SponsorBlock data | No — we use `--sponsorblock-mark` at download time |
| `comment_list` | Comments (yt-dlp `--write-comments`) | No |
| `error_list`, `warning_list` | Last download attempt messages | No persisted; we have rolling Job log only |

### 2.3 `media.Channel` / `media.Playlist` / `media.Folder`

Container fields (shared via `GenericContainer`):

- `dbid`, `name`, `nickname`, `natname`, `parent_obj`, `child_list`.
- `options_obj` — per-container `OptionsManager` override (§3).
- Counters: `vid_count`, `bookmark_count`, `dl_count`, `fav_count`,
  `live_count`, `missing_count`, `new_count`, `waiting_count`.
- `fav_flag` — cascades to all descendants.
- `error_list`, `warning_list`.
- `external_dir` — optional override storage path.
- `master_dbid` / `slave_dbid_list` — alias resolution (a video uploaded
  to two channels with different IDs can be deduplicated by pointing one
  to the other).

Channels & Playlists additionally have:

- `source` (URL).
- `rss` — RSS feed URL (for fast new-video discovery).
- `playlist_id_dict` — set of playlist IDs already imported.
- `dl_no_db_flag` — download to disk but skip the database.
- `dl_sim_flag` — always simulate (check-only, never actually fetch).
- `dl_disable_flag` — skip during operations.

### 2.4 Storage

- **`tartube.db`** — Python `pickle` of the entire media registry, not
  SQLite. Atomic save-on-modify with periodic backups (configurable
  `db_backup_mode = 'always' | 'every_session' | 'daily' | 'never'`).
- **Per-channel folder layout**:
  ```
  <data_dir>/
    <channel_name>/
      <video_name>.mp4
      <video_name>.info.json
      <video_name>.description
      <video_name>.<ext>.<lang>.vtt
      .ytdl-archive          (yt-dlp's downloaded-IDs record)
    ...
  ```
- **External directories** — a channel can be redirected to a separate
  drive via `external_dir`.

**What we can do:** Pickle is a misfeature — schema migrations are
nightmarish and a corrupt pickle locks out the whole library. Our
SQLite-backed approach is strictly better. Don't adopt the storage
model; do adopt the per-channel layout (which matches ours already).

## 3. `OptionsManager` — the headline feature

`options.OptionsManager` is the single biggest gap between us and
Tartube. It holds **164 distinct yt-dlp / youtube-dl flags** and can
be attached to:

- A specific `Video`.
- A `Channel` / `Playlist` / `Folder`.
- The application globally (the "default" manager).

Resolution at download time: `Video.options_obj` → `parent.options_obj`
→ ancestor chain → app global. First non-`None` wins. A user can
create *named* OptionsManager instances and apply the same one to many
channels.

### 3.1 Canonical option groups

From the docstring at `options.py:69`:

- **Behaviour**: ignore_errors, abort_on_error, live_from_start, wait_for_video_min.
- **Network**: proxy, socket_timeout, source_address, force_ipv4/6,
  geo_verification_proxy, geo_bypass*, geo_bypass_country, geo_bypass_ip_block.
- **Playlist selection**: playlist_start, playlist_end, playlist_items,
  max_downloads, playlist_reverse, playlist_random,
  skip_playlist_after_errors, break_on_existing, break_on_reject.
- **Filters**: min_filesize, max_filesize, date, date_before, date_after,
  min_views, max_views, match_filter, age_limit, include_ads,
  match_title_list, reject_title_list.
- **Download**: limit_rate, retries, abort_on_unavailable_fragment,
  native_hls, hls_prefer_ffmpeg, external_downloader, external_arg_string,
  concurrent_fragments, throttled_rate.
- **Filesystem**: restrict_filenames, nomtime, write_description,
  write_info, write_annotations, cookies_path, write_thumbnail,
  force_encoding, output_format, output_template, output_format_list,
  output_path_list, windows_filenames, trim_filenames, no_overwrites,
  force_overwrites.
- **Auth**: username, password, two_factor, net_rc, video_password,
  ap_mso, ap_username, ap_password.
- **Anti-bot**: no_check_certificate, prefer_insecure, user_agent,
  referer, min_sleep_interval, max_sleep_interval.
- **Format**: video_format, all_formats, prefer_free_formats, yt_skip_dash,
  merge_output_format, video_format_list, video_format_mode.
- **Subtitles**: write_subs, write_auto_subs, write_all_subs, subs_format,
  subs_lang, subs_lang_list.
- **Post-processing**: extract_audio, audio_format, audio_quality,
  recode_video, pp_args, keep_video, embed_subs, embed_thumbnail,
  add_metadata, fixup_policy, prefer_avconv, prefer_ffmpeg.
- **Cookies**: no_cookies, cookies_from_browser, no_cookies_from_browser.
- **Output sidecars**: write_link, write_url_link, write_webloc_link,
  write_desktop_link.
- **Tartube-specific**: keep_description / keep_info / keep_thumbnail /
  keep_annotations (move sidecars vs delete), sim_keep_* (same in
  simulate mode), move_description (move out of data dir after
  download), extra_cmd_string (raw passthrough), direct_cmd_flag /
  direct_url_flag (escape hatches), fetch_formats_cmd_string,
  fetch_subtitles_cmd_string, downloader_config (write a yt-dlp config
  file), check_fetch_comments / dl_fetch_comments /
  store_comments_in_db, extractor_args_list.

### 3.2 `OptionsParser`

A second class that walks the canonical option list (a registry of
`OptionHolder` entries) and emits a yt-dlp CLI: `--retries 30
--match-filter "duration > 60" -o "<template>" --cookies <path>` and
so on. Each `OptionHolder` knows its option name, its CLI switch,
default value, and `requirement_list` (other options that must be set).

**What we can do:** This is Phase 1.1 of our ROADMAP. The concrete
design:

1. Add a `download_options` table to SQLite: `(id, name, parent_id,
   json_blob)`.
2. Attach to channels via `channels` table additions: `options_id`.
3. New `DownloadOptions` Rust struct with serde — start with maybe 20
   most-used flags, not all 164. Skip the ones we don't need
   (`force_ipv4` etc.).
4. Resolver: `Video.options` → `Channel.options` → app default.
5. Per-channel "Edit options…" dialog mirrors the structure of
   `OptionsEditWin`.

## 4. Operations model

Tartube has **seven distinct operations**, each its own threaded class.
Only one operation can run at a time except for live-stream monitoring,
which runs concurrently.

| Operation | Class | Purpose |
| --- | --- | --- |
| Download | `DownloadManager` | Actually fetch videos via yt-dlp |
| Update | `UpdateManager` | Self-update yt-dlp (pip / system / git) |
| Refresh | `RefreshManager` | Re-import disk state into the media registry |
| Tidy | `TidyManager` | Verify files-on-disk vs DB, clean phantoms |
| Info | `InfoManager` | yt-dlp `--dump-json` probes, version checks, test runs |
| Process | `ProcessManager` | Post-download FFmpeg pipeline |
| Livestream | `StreamManager` | Background polling for live channels |

### 4.1 `DownloadManager`

Threading model:

- `DownloadManager` is a `threading.Thread`. It owns a worker pool of
  `DownloadWorker` threads (configurable max, default 2).
- The `DownloadList` is a queue of `DownloadItem` objects. Each item
  references a `media.Video` / `Channel` / `Playlist` plus an
  `operation_type` (sim / real / classic_sim / classic_real /
  custom_sim / custom_real).
- A worker pulls the next item, instantiates a `VideoDownloader` /
  `ClipDownloader` / `StreamDownloader`, runs it, reports back.
- Bypass workers handle livestreams without consuming a normal slot.
- Progress is reported via a `data_callback` that updates the Progress
  tab in real time.

### 4.2 `VideoDownloader` vs `ClipDownloader` vs `StreamDownloader`

- **`VideoDownloader`** — the standard case. Builds a yt-dlp CLI from
  the resolved `OptionsManager`, runs it, parses stdout/stderr line by
  line, updates the `media.Video` record on completion.
- **`ClipDownloader`** — uses yt-dlp's `--download-sections` /
  `--postprocessor-args "ffmpeg:-ss …"` to download just one timestamp
  range. Powers the "Edit timestamps" → "Download clip" UI.
- **`StreamDownloader`** — wraps `yt-dlp --live-from-start` with
  Tartube-specific monitoring, restart-on-disconnect, and a max-duration
  cap. Used by `StreamManager`'s livestream auto-record.

### 4.3 `Tidy`, `Refresh`, `Info`

- **Tidy** — like our maintenance scan but more aggressive: finds
  duplicate dbids, orphaned files, channels whose folder vanished,
  videos that exist on disk but not in the DB.
- **Refresh** — opposite direction: walks the disk and imports any
  unrecognised file into the DB.
- **Info** — runs yt-dlp `--dump-json` on a URL to fetch metadata
  without downloading; also `--test` mode that downloads only a
  fragment to verify settings work.

### 4.4 Update operation

Three backends, user-selectable:

- pip (`pip install -U yt-dlp`)
- system package (`apt install --reinstall yt-dlp`, etc.)
- git pull on a local clone

**What we can do:** Our bundled-venv install is conceptually the
"pip" path. We don't expose a "git pull from upstream master" mode but
arguably that's overkill for a single-user tool.

## 5. Custom downloads

`CustomDLManager` adds a second axis of configuration on top of
`OptionsManager`. A custom download is a *recipe*:

- Which sources to use (channels / playlists / folders / specific
  videos).
- Whether to download or just check.
- Whether to delete originals after a successful post-process.
- A specific `OptionsManager`.
- A scheduled time (via `Scheduled`).

Users define multiple named custom-DL recipes and trigger them
manually from the Drag-and-Drop tab or via schedule.

**What we can do:** Our "Music mode" + "Live mode" + "Twitch clips
only" toggles are early hints of this. The principled version is
*named profiles* the user can construct and reuse.

## 6. UI surfaces

`MainWin` is one Gtk window with five primary tabs:

1. **Videos** — the main library browser. Sidebar = container tree;
   main area = "Video Catalogue" (the video card grid). Three render
   modes: Simple (list), Complex (cards with metadata), Grid
   (thumbnail wall). Filters (by name/author/desc/comment) + sort.
2. **Progress** — live download status: per-job rows with status,
   percent, ETA, current file. Like our jobs list.
3. **Classic Mode** — one-off downloads. Paste a URL, pick destination
   folder, pick format, queue. Bypasses the database entirely.
4. **Drag and Drop** — "drop zones" the user creates, each with a
   pre-configured `OptionsManager`. Dropping a URL onto a zone
   downloads with that zone's settings.
5. **Errors / Warnings** — accumulated diagnostic messages.

Around the central tabs:

- Menu bar: Media (add channel / playlist / folder / video, bulk add,
  insert video, import YouTube subscriptions…), Operations (download,
  check, tidy, refresh, info, update), Options (edit general /
  custom-DL / FFmpeg / preferences), Help (about, manual, tutorial).
- Toolbar: hide / squeeze, custom icons, custom DL button.
- Status icon (system tray): tray menu, restore from tray, minimize to
  tray.

### 6.1 Dialogs (~80 of them)

A representative sample:

- **AddChannelDialogue / AddPlaylistDialogue / AddFolderDialogue** —
  add a remote container with name + URL + parent folder.
- **AddBulkDialogue** — paste many URLs, autodetect type per URL.
- **AddStampDialogue** — manually add chapter timestamps.
- **AddVideoDialogue** — add a single video URL to an existing channel.
- **AddDropZoneDialogue** — create a Drag-and-Drop zone.
- **ApplyOptionsDialogue** — pick an `OptionsManager` to apply to one
  or more containers.
- **CalendarDialogue** — date picker for filters.
- **ChangeThemeDialogue** — switch GTK theme.
- **CreateProfileDialogue** — group containers into a "profile" you can
  download all at once.
- **DeleteContainerDialogue / DeleteVideoDialogue / DeleteDropZoneDialogue**
  — confirm-before-delete with the usual checkboxes.
- **DuplicateVideoDialogue** — what to do when the same video ID
  appears in two channels.
- **ExportDialogue / ImportDialogue** — DB export/import as JSON.
- **ExtractorCodeDialogue** — manual yt-dlp extractor selection.
- **FormatsSubsDialogue** — pick subtitle formats for a download.
- **InsertVideoDialogue** — insert a specific video at a position in a
  playlist.
- **MountDriveDialogue** — handle external-drive containers when the
  drive isn't mounted.

Plus per-object edit windows in `config.py`:

- `VideoEditWin` — edit a single video's metadata, flags, stamps,
  slices.
- `ChannelPlaylistEditWin` — channel/playlist editor.
- `FolderEditWin` — folder editor.
- `OptionsEditWin` — a giant tabbed editor for an `OptionsManager`.
- `CustomDLEditWin` — recipe editor.
- `FFmpegOptionsEditWin` — preset editor for the post-process pipeline.
- `ScheduledEditWin` — schedule entry editor.
- `SystemPrefWin` — application-wide preferences (the equivalent of
  our Settings modal but with ~30 tabs).

### 6.2 Wizards

- **SetupWizWin** — first-run wizard: pick data dir, pick downloader,
  optionally install yt-dlp / FFmpeg, optionally enable the database.
- **ImportYTWizWin** — import a YouTube subscriptions OPML/JSON file as
  channels.
- **TutorialWizWin** — embedded tutorial.

**What we can do:** Our `Settings` modal covers maybe 15% of what
`SystemPrefWin` handles. Some of those — proxy config, retry tuning,
sleep intervals — are reasonable to add. Others (Adobe Pass auth, geo
bypass, restrict filenames) are corner cases that bloat the surface
without serving a clear use case. Be selective.

## 7. Configuration / preferences

Tartube's `SystemPrefWin` covers (each is a tab):

- General
- Files (data dir, external dirs, backups)
- Downloads (worker count, retry limits, bandwidth)
- Filters (default catalogue filters)
- Display (themes, drag formats, status icon)
- Operations (sim vs real defaults, error visibility)
- yt-dlp (path to binary, update mode, extra args)
- FFmpeg (post-process presets, codec defaults)
- Livestreams (polling interval, max duration)
- Notifications (system, dialog, sound)
- Scheduling (default times, day-of-week toggles)
- Profiles (named container groups)
- Cookies (path, browser, refresh policy)
- Anti-bot (sleep intervals, user agents, impersonation hints —
  available since yt-dlp added curl_cffi)
- SponsorBlock (categories, actions, manual editing toggle)
- Comments (fetch / store / display)
- Advanced (locks, debug toggles, raw command-line passthrough)

The 791 boolean flags in `mainapp.py` cover all of these plus their
"…also show this…" / "…also do this in classic mode…" variants.

**What we can do:** A handful of these tabs map to features we
literally don't have (FFmpeg presets, comments, profiles, scheduling
calendar). They're roadmap items, not config surface. The actual
*configuration surface* of Catacomb is currently:

- backup dir, max concurrent, bundled-or-system yt-dlp
- player command, browser (for `--cookies-from-browser`)
- UI theme
- web port, bind interface, transcode, source URL
- scheduler enabled + interval hours
- Plex library path
- cookies (paste/file)
- password (Argon2)

Adding ~5–8 fields (proxy, retry overrides, sleep interval, default
options manager) would close most of the practical gap. Adding 50 more
matches Tartube but rapidly costs more than it earns.

## 8. SponsorBlock + Stamp lists

Tartube goes further than yt-dlp's built-in SponsorBlock support:

- `media.Video.slice_list` stores per-video segments with `(category,
  action, start_time, stop_time)`.
- The user can manually add slices via a UI editor.
- The slice list survives re-downloads — Tartube re-applies them.
- Slices can be set to "skip" (cut from output) instead of just "mark"
  (chapter marker).

`stamp_list` is the same idea for chapters: extracted from
description/metadata at download time, editable by the user, used by
`ClipDownloader` to slice the source video into named clips.

**What we can do:** Our SponsorBlock support is "pass `--sponsorblock-mark
all` to yt-dlp and hope". Adopting Tartube's persistence model would
let users:
- Manually mark intros / outros / sponsor reads on any video.
- Generate clips by chapter (download a 30-minute video, save 5 clips
  named after their chapter titles).
- Hide already-cut segments from the player.

Reasonable Phase 1 add-on after per-channel options ships.

## 9. Notable corners

- **Master/slave dbid** — alias resolution for re-uploads. A video
  uploaded to two channels with different IDs can be marked as a
  slave of another, so the dedupe scan treats them as the same.
- **Profiles** — named groups of `Channel` / `Playlist` / `Folder`
  references. The user can "Download Profile 'Daily'" and get every
  member checked in one batch.
- **Newbie dialog** — pop-up that surfaces useful tips for new users.
- **RSS feed discovery** — `media.Channel.rss` lets Tartube use a
  channel's RSS feed for fast new-video discovery instead of doing a
  full yt-dlp playlist scan.
- **Livestream message parsing** — `ttutils.extract_livestream_data()`
  parses YouTube's "Scheduled for…" / "Premieres in 3 days" strings
  into structured fields used for sorting + display.
- **External directories** — a single library can span multiple drives
  by setting `media.Channel.external_dir` per-channel.
- **Block flag** — videos that are age-restricted / geo-blocked /
  censored. Tartube tracks them separately so the UI shows them with
  a different colour and the download skips them.
- **Locale** — full `gettext` i18n with ~10 translations. We have
  none.

## 10. Things Tartube does better than us

Items where we *currently lose* and should adopt:

1. **Per-target download options with cascade resolution** (`OptionsManager`)
   — Phase 1.1 of our roadmap. Largest gap.
2. **Folder hierarchy** — Phase 1.2.
3. **System folders as smart filters** — Bookmarks, Favourites, Waiting,
   New, Live, Missing. Generalises our `SidebarView::Recent`.
4. **Multiple per-video state flags** — bookmark / favourite / archive /
   missing / waiting / block / new. Currently we only track watched.
5. **Master / slave dedup** — alias one upload to another.
6. **Manual chapter / clip editing UI** — `stamp_list` + `ClipDownloader`.
7. **Manual SponsorBlock editing + persistence** — `slice_list`.
8. **Comments capture and viewer** — `--write-comments` + `comment_list`.
9. **Profiles** — named container groups for batch operations.
10. **Tidy + Refresh operations** — full bidirectional sync between
    registry and disk. Our maintenance scan covers only half.
11. **Setup wizard** — first-run handholding for the data dir / yt-dlp
    / FFmpeg.
12. **i18n / `gettext`** — every locale Tartube has, we don't.
13. **Per-distro installer ecosystem** — .deb, .rpm, MSI, .dmg,
    Chocolatey, AUR. We ship a PKGBUILD.
14. **RSS-driven new-video discovery** — quicker than a full yt-dlp
    scan.
15. **`extra_cmd_string` / passthrough** — escape hatch for any yt-dlp
    flag we haven't surfaced. We have nothing.

## 11. Things we do better than Tartube

For the record (to keep momentum honest):

1. **Single static binary** vs Python + GTK + dozens of pip deps.
2. **Web UI accessible from any device** vs desktop-only GTK window.
3. **Modern security stack** — Argon2 sessions, CSP, rate-limit, body
   limit, ETag — vs none.
4. **First-class Plex export** with NFO sidecars and show folders.
5. **Bundled curl_cffi venv install** — Tartube users do this manually
   per platform.
6. **Real multi-platform routing** — separate `tiktok/`, `twitch/`,
   `vimeo/` etc. with `.source-url` sidecars. Tartube treats every
   source identically.
7. **Themes** — 10 of ours vs default GTK plus a single optional dark
   theme.
8. **Performance** — Rust parallel scan, gzip + ETag on
   `/api/library`, adaptive polling, DB connection pool. Tartube's
   pickle load is the bottleneck for large libraries.
9. **SHA-256-verified bundled-binary install**.
10. **Activity feed / Recent additions sidebar entry** — Tartube has
    a Recent Videos folder but no per-week activity histogram.

## 12. Specification for "Tartube parity" milestone

Working definition: a catacomb release is "at Tartube parity" when
the following are all true.

- [x] **Per-target download options** with cascade resolution and a
      per-channel editor. *v1 shipped 2026-05-25: 9 fields per channel
      (quality cap, audio-only, rate limit, min/max filesize, date-after,
      match-filter, subtitle langs, raw extra args), persisted in the
      `channel_options` SQLite table, attached at scan time, applied at
      every dispatch site (scheduled re-checks, right-click checks).
      Editor surfaces in both UIs.*
- [x] **Folder hierarchy** with N-level nesting, drag-to-reparent, and
      per-folder default options. *v1 shipped 2026-05-25: flat folders
      (single level, no nesting) via new `folders` + `channel_assignments`
      SQLite tables. Sidebar groups assigned channels under their folder;
      unfiled channels keep platform grouping. Manage-folders dialog in
      both UIs handles create / rename / delete. Per-channel "Move to
      folder…" context action. N-level nesting + drag-to-reparent +
      per-folder cascading options are deferred to v2.*
- [x] **Smart folders** for Bookmarks / Favourites / Waiting on top of
      Recent + Continue Watching. *v1 shipped 2026-05-25: each smart
      folder appears in the sidebar only when at least one video carries
      the matching flag, so a fresh install isn't polluted with empty
      rows. Driven by the in-memory flag bundle so no SQL per render.*
- [x] **Per-video state flags** persisted in a new `video_flags`
      SQLite table with columns `bookmark`, `favourite`, `waiting`,
      `archive`. Loaded as four `HashSet<String>` at startup, mutated
      via `POST /api/videos/:id/flags/:flag` (web) or the per-card
      action buttons (desktop). `archive` is wired through but UI is
      deferred until we actually have auto-deletion to gate.
- [x] **Comments capture** via the new `fetch_comments` field in
      DownloadOptions. Adds `--write-comments` to the yt-dlp command
      when set. New `GET /api/comments/:id` reads the comments array
      out of info.json and the web player modal's 💬 button renders a
      threaded viewer.
- [x] **Profiles** — saved named groups of channels with a "Download
      this profile" action. *Partial via folders: each folder doubles
      as a profile. `POST /api/folders/:id/check` + the "⬇ Check"
      buttons in both UIs' folder-manager dialogs fire a re-check on
      every member channel, each honouring its own `DownloadOptions`
      (quality / rate / extra args).*
- [ ] **Tidy + Refresh** unified into a single bidirectional sync
      operation in the maintenance window. *Mostly already covered by
      our existing `maintenance::scan` — what's left is rolling phantom
      detection + disk-content-not-in-DB into the same UI. Low effort
      when picked up.*
- [x] **Manual chapter / SponsorBlock editing UI** — edit / persist /
      re-apply on re-download. *Deferred — comments capture covered
      the more useful sibling feature; chapter editing remains a niche
      ask.*
- [ ] **Setup wizard** — first-run flow for empty installs that
      handles data dir, yt-dlp install, optional FFmpeg, optional
      Plex.
- [ ] **Per-distro release artifacts** — .deb, .rpm, MSI, .app/.dmg,
      AUR, Chocolatey or winget.
- [x] **`extra_args` passthrough** field in download options for any
      flag we don't surface natively. *Shipped as part of Phase 1.1
      DownloadOptions.*
- [ ] **i18n scaffolding** — even one alt locale to prove the build.

### Beyond Tartube (additional UX shipped 2026-05-25)

- DB-snapshot download via `GET /api/backup/db` — one-click "save my
  watched / favourites / channel-options / folders state offsite".
- Bulk tagging in selection mode: ★/🔖/⏳ in addition to ✓/○.
- Search filters extend to channel name (not just title + id).
- Keyboard shortcuts in the web UI (/, r, Esc, ?) with a help modal.
- Persisted UI state across reloads — view, search, sort all survive
  via localStorage.
- 🎲 Shuffle button — random unwatched video pick.

Together with the architectural wins listed in §11, that gives us the
"surpass" toehold the roadmap calls for.

When all twelve land, the comparison table at the top of `ROADMAP.md`
goes from 8-ahead-8-behind to 8-ahead-0-behind. After that, the
"surpass" phase consists of leaning on our architectural advantages:
WebSocket progress, Android client, federation, perceptual-hash dedup,
plugins.

## 13. References

- Source: <https://github.com/axcore/tartube>
- Local clone for this analysis: `/tmp/tartube` (axcore/tartube
  v2.5.231).
- Modules read in full: `media.py` (data model), `options.py` (option
  registry), `downloads.py` (operations + workers), `mainapp.py`
  (config surface).
- Modules summarised: every other `.py` file under `tartube/`.
- Cross-referenced with our `ROADMAP.md` Phase 1 items.
