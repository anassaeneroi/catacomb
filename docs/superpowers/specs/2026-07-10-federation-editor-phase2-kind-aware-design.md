# Kind-aware remote editor (phase 2)

**Date:** 2026-07-10
**Status:** Approved design, ready for implementation plan
**Scope:** Desktop GUI + web server. Sub-project 2 of the PeerTube federation work.

## Context

Phase 2 of three (see `2026-07-10-peertube-client-backend-design.md` §Context).
Phase 1 (backend `PeerTubeClient` + `RemoteKind`/`username` config) is **done**.
This phase adds the in-UI editor to manage remotes of *both* kinds, applied
live.

**Inherits** `2026-07-10-federation-remote-editor-design.md` wholesale for the
unchanged editor mechanics — read it first. Unchanged from that spec:
- `WebState.remotes` becomes a `std::sync::RwLock<…>`; browse endpoints
  read-lock, the editor save write-locks (live-apply).
- Dedicated `/api/remotes/*` endpoints (not folded into `SettingsPayload`):
  extended `GET`, new `PUT` (whole-list replace), new `POST /api/remotes/test`.
- Web passwords are masked / write-only; blank on save keeps the stored secret,
  matched by URL (`merge_remote_passwords`, pure + unit-tested). Desktop shows
  passwords in the clear (local).
- Config source of truth is `state.config` (`state.config.lock_recover()`);
  save via `config.save`; on save error, don't swap the live list; drop the
  config lock before taking the remotes lock.
- After any edit, both UIs refetch the list and clear the current selection.

This document specifies only the **kind-aware deltas**.

## Delta 1 — Multi-kind live storage

New enum in `src/remote.rs`:

```rust
pub enum RemoteClientKind {
    Catacomb(RemoteClient),
    Peertube(crate::peertube::PeerTubeClient),
}

impl RemoteClientKind {
    pub fn from_section(cfg: &crate::config::RemoteSection) -> Self {
        match cfg.kind {
            crate::config::RemoteKind::Catacomb => Self::Catacomb(RemoteClient::new(cfg)),
            crate::config::RemoteKind::Peertube => Self::Peertube(crate::peertube::PeerTubeClient::new(cfg)),
        }
    }
    pub fn name(&self) -> &str {
        match self { Self::Catacomb(c) => &c.name, Self::Peertube(p) => &p.name }
    }
    pub fn kind(&self) -> crate::config::RemoteKind {
        match self {
            Self::Catacomb(_) => crate::config::RemoteKind::Catacomb,
            Self::Peertube(_) => crate::config::RemoteKind::Peertube,
        }
    }
}
```

- `App.remotes: Vec<Arc<RemoteClientKind>>` (desktop; mutable App, no lock).
- `WebState.remotes: RwLock<Vec<Arc<RemoteClientKind>>>` (web; per inherited
  spec). Built at startup with `RemoteClientKind::from_section` per config
  remote.
- Both fronts rebuild the whole `Vec` on editor save.

`RemoteClient.name` must be `pub` (it already is); `PeerTubeClient.name` is `pub`
(phase 1).

## Delta 2 — Browse dispatch (phase-2 stopgap)

The existing browse call-sites match on the enum:

- **Web** `get_remote_library` (positional `:id`): read-lock; match the entry:
  - `Catacomb(c)` → `c.library_json()` (unchanged behavior).
  - `Peertube(_)` → `501 Not Implemented` / body `"PeerTube browsing arrives in
    phase 3"`.
- **Desktop** `start_remote_fetch(idx)`: match the entry:
  - `Catacomb(c)` → spawn the existing `c.library()` fetch (unchanged).
  - `Peertube(_)` → set `remote_status = "PeerTube browsing arrives in a later
    update"`, no fetch.

So a PeerTube remote is fully manageable (add/edit/test/save) in phase 2;
*browsing* it is phase 3. Catacomb browsing is untouched.

## Delta 3 — Editor UI

The inherited "Federation peers" Settings section, each row gaining:
- a **kind** selector — Catacomb / PeerTube (`egui::ComboBox` desktop; `<select>`
  web);
- a **username** field, rendered only when kind = PeerTube;
- (unchanged) name, url, password (masked web / clear desktop), **Test**, Remove.

Desktop edits `self.config.remotes` (each `RemoteSection` now carries
`kind`/`username`); on save rebuild `self.remotes` via `from_section` and reset
`remote_selected`/`remote_library`. Web edits a JS array and saves via `PUT`.

## Delta 4 — Web API

On top of the inherited endpoints:

- `GET /api/remotes` response gains `kind` (string, `"catacomb"`/`"peertube"`),
  keeps `has_password` — `[{ id, name, url, kind, has_password }]`.
- `PUT /api/remotes` entry shape is `{ name, url, kind, username, password }`.
  `merge_remote_passwords` (URL-keyed) unchanged; the rebuilt live list uses
  `RemoteClientKind::from_section` per merged `RemoteSection`.
- `POST /api/remotes/test` body `{ url, kind, username, password }`. Build the
  matching client kind (resolving a blank password from the stored remote by
  URL, per the inherited merge) and check reachability on `spawn_blocking`:
  - catacomb → `RemoteClient::library_json()` ok → `{ ok: true }` (optionally a
    channel count from the parsed library);
  - peertube → `PeerTubeClient::list_channels()` → `{ ok: true, channels: N }`;
  - failure → `{ ok: false, error }`.

## Delta 5 — Test-connection

Branches on kind as in Delta 4. Desktop runs it on a background thread
(mirroring `start_remote_fetch`), delivering `{ ok, detail }` over an mpsc
channel drained in `update()`; the PeerTube arm calls `list_channels()`.

## Error handling

Inherited. Additionally: a PeerTube test against a bad instance/credentials
fails at `list_channels()`/OAuth with a descriptive message; the remote can
still be saved (peer may be down at edit time).

## Testing

- **Unit** (`remote.rs`): `RemoteClientKind::from_section` builds the
  `Catacomb` variant for a default section and `Peertube` for
  `kind = peertube`; `name()`/`kind()` report correctly.
- **Unit** (inherited): `merge_remote_passwords` (URL-keyed keep-on-blank).
- **Integration** (`tests/api.rs`): `PUT /api/remotes` with one catacomb and one
  `kind:"peertube"` entry → `GET /api/remotes` reflects both with correct `kind`
  and `has_password`; a removed entry disappears; a blank-password edit of an
  existing entry keeps `has_password` true. (Live PeerTube reachability needs a
  real instance → manual.)
- **Manual:** in both UIs, add a catacomb peer and a public PeerTube remote
  (e.g. `https://framatube.org/c/<channel>`); Test each; save; confirm catacomb
  browse still works and clicking the PeerTube remote shows the phase-3 stopgap;
  confirm the web `GET` never returns plaintext passwords and blank-save
  preserves them.

## What phase 3 consumes

The `RemoteClientKind::Peertube` arm's stopgap is replaced by the two-level
lazy browse (`list_channels` → `channel_videos`) + inline play (`video_media`) +
the per-video Archive action (`watch_url` → downloader).
