# In-UI federation remote editor

**Date:** 2026-07-10
**Status:** Approved design, ready for implementation plan
**Scope:** Desktop GUI + web server (roadmap 3.5 follow-up)

## Problem

Federation peers (other catacomb instances browsed read-only, see
`src/remote.rs`) can only be configured by hand-editing `[[remote]]` tables in
`config.toml` and restarting. There is no in-UI way to add, edit, remove, or
test a peer. Both front-ends already *browse* peers (desktop Remotes screen,
web remote switcher) but neither can *manage* them.

## Goal

Add an in-UI editor to add / edit / remove / test federation peers, in the
Settings surface of both UIs, applied live (no restart).

## Decisions (locked during brainstorming)

- **Location:** a "Federation peers" section in Settings, in both UIs.
- **Apply mode:** live — edits take effect immediately, no restart.
- **Web password handling:** masked / write-only — peer passwords never travel
  to the browser; a blank password field on save keeps the stored secret.
- **Test connection:** included — a per-peer "Test" button.

## Non-goals

- No config schema change: reuse the existing `RemoteSection`.
- No write/sync federation (peers stay read-only browse; this only manages the
  peer list).
- Desktop password masking: the desktop is local and already exposes
  `config.toml`, so it shows peer passwords in the clear (no merge logic there).

## Data model

Unchanged. `config.rs`:

```rust
pub struct RemoteSection {
    pub name: String,
    pub url: String,
    pub password: Option<String>, // plaintext, like cookies
}
// Config.remotes: Vec<RemoteSection>, persisted by Config::save().
```

## Web: live-apply via RwLock

`WebState.remotes` changes type:

```rust
// before: pub remotes: Vec<std::sync::Arc<crate::remote::RemoteClient>>,
pub remotes: std::sync::RwLock<Vec<std::sync::Arc<crate::remote::RemoteClient>>>,
```

Built at startup the same way, wrapped in `RwLock`. The two existing browse
endpoints take a read lock; the editor's save takes a write lock to swap the
whole list. RwLock keeps it std-only (no new dep); reads are infrequent (manual
browse), so contention is a non-issue.

- `get_remotes`: `state.remotes.read().unwrap().iter()…`
- `get_remote_library`: `state.remotes.read().unwrap().get(id).cloned()`

## Web API

Dedicated `/api/remotes/*` endpoints (not folded into `SettingsPayload`, which
is a single struct shared by GET and POST and cannot cleanly express
masked-on-read / password-on-write). All are behind `auth_middleware` like the
rest of `/api/*`.

### `GET /api/remotes` (extended)

Existing response gains `has_password`. Serves both the browse switcher and the
editor:

```json
[{ "id": 0, "name": "woofbox", "url": "http://woofbox:8081", "has_password": true }]
```

No plaintext password is ever emitted.

### `PUT /api/remotes` (new)

Body: the **full replacement** list.

```json
[{ "name": "woofbox", "url": "http://woofbox:8081", "password": null }]
```

Handler (config source of truth is `state.config`, the same lock
`post_settings` uses via `state.config.lock_recover()`):
1. `let mut cfg = state.config.lock_recover();`
2. `let merged = merge_remote_passwords(&body, &cfg.remotes);` (§ Password merge).
3. `cfg.remotes = merged.clone();` then `cfg.save(&state.config_path)` — on
   error return `500` with the message (mirror `post_settings`) and do **not**
   swap the live list.
4. `drop(cfg);` (release the config lock before taking the remotes lock — keep a
   consistent lock order, config-before-remotes, and never hold both).
5. Rebuild live clients:
   `*state.remotes.write().unwrap() = merged.iter().map(|r| std::sync::Arc::new(crate::remote::RemoteClient::new(r))).collect();`
6. Return `200` (`{ "ok": true }`).

Whole-list replace is atomic and sidesteps index-shift races. There is exactly
one copy of `config.remotes` (inside `state.config`); the `RwLock<Vec<Arc<…>>>`
is a derived cache rebuilt from it, never edited independently.

### `POST /api/remotes/test` (new)

Body: `{ "url": "...", "password": null }`. Resolve the password (use `password`
if non-empty, else the stored password of the existing remote with the same
URL). Build a transient `RemoteClient`, call `library()` on
`spawn_blocking`, and return:

```json
{ "ok": true, "channels": 42 }        // or
{ "ok": false, "error": "remote error: 401 Unauthorized" }
```

## Password merge (pure, testable)

```rust
/// Resolve write-only passwords: each input keeps its own password if the
/// caller typed one (Some(non-empty)); otherwise it adopts the stored password
/// of the existing remote with the same `url`; otherwise None. URL is the peer
/// key, so a blank field preserves the stored secret without echoing it.
fn merge_remote_passwords(
    inputs: &[RemoteInput],
    existing: &[RemoteSection],
) -> Vec<RemoteSection>;

struct RemoteInput { name: String, url: String, password: Option<String> }
```

Empty-string passwords are treated as `None` (blank field). Trim the URL for
matching to avoid whitespace mismatches; store the trimmed URL.

## Desktop

`App.remotes` stays `Vec<Arc<RemoteClient>>` (mutable `App`, UI thread, no lock).

- **Editor:** a "Federation peers" section in `settings_screen`, editing
  `self.config.remotes` directly — a row per peer (name, url, password text
  fields; a Remove button) and an Add-peer control. Passwords shown in the
  clear (local).
- **Save:** the desktop Settings save path already calls
  `self.config.save(&self.config_path)`. After saving, rebuild
  `self.remotes = self.config.remotes.iter().map(|r| Arc::new(RemoteClient::new(r))).collect()`
  and reset `self.remote_selected = None; self.remote_library = None;`.
- **Test:** a per-row "Test" button spawns a background thread (mirroring
  `start_remote_fetch`) that builds a `RemoteClient` from the row and calls
  `library()`, delivering the result over an mpsc channel drained in `update()`;
  show reachable / channel count / error inline in the section.

## Web UI (`web_ui/index.html`)

A "Federation peers" section in the Settings modal:
- One row per peer: name, url, a password field (blank, with a "password set"
  hint when `has_password`), a **Test** button (→ `POST /api/remotes/test`), a
  **Remove** button.
- An **Add peer** row.
- A **Save peers** action → `PUT /api/remotes` with the full array; on success,
  re-`GET /api/remotes` so the editor and the browse switcher stay in sync.
- The section renders under the existing theme CSS variables; syntax-check the
  inline script with `node --check` after editing (per repo convention).

## Live-apply & index stability

The browse layer stays positional (`/api/remotes/:id`, desktop index). After any
successful edit, both UIs refetch the peer list and clear the current selection,
so a stale index cannot mis-target a shifted/removed peer.

## Error handling

- Save failures (`config.save`) surface the error to the UI and do **not** swap
  the live client list (config and live state stay consistent).
- `PUT`/`test` with a malformed URL: `RemoteClient` construction is total;
  errors surface at `test`/browse time as a `BAD_GATEWAY`-style message, not a
  panic.
- Test against an unreachable/authexn peer returns `{ ok: false, error }` — the
  editor shows it inline; the peer can still be saved (the user may be adding it
  before the peer is up).

## Testing

- **Unit** (`web.rs` or `config.rs`): `merge_remote_passwords` — new URL →
  `None`; blank password + matching URL → keeps the stored secret; typed
  password → replaces; a removed peer → dropped; whitespace-padded URL matches
  its trimmed twin.
- **Integration** (`tests/api.rs`): `PUT /api/remotes` then `GET /api/remotes`
  reflects the new set (names/urls/count) and correct `has_password`; a removed
  peer disappears; a blank-password edit of an existing peer leaves
  `has_password` true. (Live test-connection needs a second server, so it is
  covered by the unit-level resolver plus manual verification.)
- **Manual:** desktop add / edit / remove / test + live browse without restart;
  web the same, confirming passwords never appear in `GET` responses (check
  devtools/network) and blank-save preserves them.
```
