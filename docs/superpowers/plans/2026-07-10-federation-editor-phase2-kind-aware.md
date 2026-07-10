# Kind-Aware Remote Editor — Implementation Plan (Phase 2)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An in-UI editor (both UIs) to add/edit/remove/test federation remotes of both kinds (catacomb + PeerTube), applied live, with PeerTube remotes manageable now and browsable in phase 3.

**Architecture:** A `RemoteClientKind` enum wraps `RemoteClient`/`PeerTubeClient`; the live remotes lists hold it (web behind a `RwLock` for live-apply). Dedicated `/api/remotes/*` endpoints (extended `GET`, new `PUT` whole-list-replace, new `POST /test`) manage them, with URL-keyed write-only password merge on the web. Both editors gain a kind selector + conditional username field. Existing catacomb browse is untouched; PeerTube browse is a phase-3 stopgap.

**Tech Stack:** Rust, axum, eframe/egui, reqwest (blocking), serde/serde_json; the embedded `web_ui/index.html` SPA.

## Global Constraints

- Inherits `docs/superpowers/specs/2026-07-10-federation-remote-editor-design.md` for unchanged editor mechanics; deltas in `docs/superpowers/specs/2026-07-10-federation-editor-phase2-kind-aware-design.md`.
- Web config source of truth is `state.config` (`Mutex<Config>`, accessed via `.lock_recover()`); save via `cfg.save(&state.config_path)`; on save error don't swap the live list; drop the config lock before taking the remotes lock.
- Web passwords masked/write-only: `GET` never returns plaintext; blank on save keeps the stored secret, matched by URL. Desktop shows passwords in the clear.
- After any edit, both UIs refetch and clear the current remote selection.
- The web SPA is one embedded file; **a `cargo build` does not catch JS syntax errors** — after editing `web_ui/index.html`, run `awk '/<script>/{f=1;next}/<\/script>/{f=0}f' src/web_ui/index.html > /tmp/spa.js && node --check /tmp/spa.js`.
- New UI must use the existing theme CSS variables; no CDN assets (offline-first).
- Commits SSH-signed: `export SSH_AUTH_SOCK=/tmp/luna-ssh-agent.sock`. End messages with `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.

---

### Task 1: `RemoteClientKind` enum + storage migration + browse dispatch

Atomic: introduce the enum and switch both live lists to it (web behind `RwLock`), fixing every read site and adding the PeerTube browse stopgap. Compiles with catacomb browse unchanged.

**Files:**
- Modify: `src/remote.rs` — add `RemoteClientKind`.
- Modify: `src/web.rs` — `WebState.remotes` type, construction, `get_remotes`, `get_remote_library`.
- Modify: `src/app.rs` — `App.remotes` type, construction, `start_remote_fetch`, `remotes_screen` name access.

**Interfaces:**
- Consumes: `crate::remote::RemoteClient`, `crate::peertube::PeerTubeClient`, `crate::config::{RemoteSection, RemoteKind}`.
- Produces: `pub enum RemoteClientKind { Catacomb(RemoteClient), Peertube(PeerTubeClient) }` with `from_section(&RemoteSection) -> Self`, `name(&self) -> &str`, `kind(&self) -> RemoteKind`.

- [ ] **Step 1: Write the failing unit test**

Append to `src/remote.rs`'s `#[cfg(test)] mod tests`:

```rust
#[test]
fn client_kind_from_section_dispatches() {
    use crate::config::{RemoteKind, RemoteSection};
    let cat = RemoteSection {
        name: "c".into(), url: "http://p:8081".into(),
        kind: RemoteKind::Catacomb, username: None, password: None,
    };
    let pt = RemoteSection {
        name: "p".into(), url: "https://framatube.org".into(),
        kind: RemoteKind::Peertube, username: None, password: None,
    };
    let a = RemoteClientKind::from_section(&cat);
    let b = RemoteClientKind::from_section(&pt);
    assert_eq!(a.kind(), RemoteKind::Catacomb);
    assert_eq!(a.name(), "c");
    assert_eq!(b.kind(), RemoteKind::Peertube);
    assert_eq!(b.name(), "p");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --release client_kind_from_section 2>&1 | grep -E "cannot find|test result"`
Expected: FAIL — `RemoteClientKind` not found.

- [ ] **Step 3: Add the enum**

Append to `src/remote.rs` (module scope, after `RemoteClient`'s `impl`):

```rust
/// A live federation client of either kind. Held by both front-ends' remotes
/// lists so catacomb peers and PeerTube targets coexist.
pub enum RemoteClientKind {
    Catacomb(RemoteClient),
    Peertube(crate::peertube::PeerTubeClient),
}

impl RemoteClientKind {
    pub fn from_section(cfg: &crate::config::RemoteSection) -> Self {
        match cfg.kind {
            crate::config::RemoteKind::Catacomb => Self::Catacomb(RemoteClient::new(cfg)),
            crate::config::RemoteKind::Peertube => {
                Self::Peertube(crate::peertube::PeerTubeClient::new(cfg))
            }
        }
    }
    pub fn name(&self) -> &str {
        match self {
            Self::Catacomb(c) => &c.name,
            Self::Peertube(p) => &p.name,
        }
    }
    pub fn kind(&self) -> crate::config::RemoteKind {
        match self {
            Self::Catacomb(_) => crate::config::RemoteKind::Catacomb,
            Self::Peertube(_) => crate::config::RemoteKind::Peertube,
        }
    }
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test --release client_kind_from_section 2>&1 | grep "test result"`
Expected: `test result: ok. 1 passed`. (Crate won't fully build yet if the test build touches web/app — if so, proceed to Steps 5–7 and re-run at Step 8.)

- [ ] **Step 5: Migrate `WebState.remotes` to `RwLock<Vec<Arc<RemoteClientKind>>>`**

In `src/web.rs`:

Field (line ~154):
```rust
    pub remotes: std::sync::RwLock<Vec<std::sync::Arc<crate::remote::RemoteClientKind>>>,
```

Construction (line ~3214):
```rust
    let remotes: Vec<std::sync::Arc<crate::remote::RemoteClientKind>> = config
        .remotes
        .iter()
        .map(|r| std::sync::Arc::new(crate::remote::RemoteClientKind::from_section(r)))
        .collect();
```
And in the `WebState { … }` literal change `remotes,` to `remotes: std::sync::RwLock::new(remotes),`.

`get_remotes` (line ~2250):
```rust
async fn get_remotes(State(state): State<Arc<WebState>>) -> impl IntoResponse {
    let list: Vec<_> = state
        .remotes
        .read()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let (url, has_password) = match r.as_ref() {
                crate::remote::RemoteClientKind::Catacomb(c) => (c.base_url().to_string(), c.has_password()),
                crate::remote::RemoteClientKind::Peertube(p) => (p.base_url().to_string(), p.has_password()),
            };
            let kind = match r.kind() {
                crate::config::RemoteKind::Catacomb => "catacomb",
                crate::config::RemoteKind::Peertube => "peertube",
            };
            serde_json::json!({ "id": i, "name": r.name(), "url": url, "kind": kind, "has_password": has_password })
        })
        .collect();
    Json(list)
}
```

`get_remote_library` (line ~2264) — dispatch + stopgap:
```rust
async fn get_remote_library(
    State(state): State<Arc<WebState>>,
    Path(id): Path<usize>,
) -> Response {
    let remote = state.remotes.read().unwrap().get(id).cloned();
    let Some(remote) = remote else {
        return (StatusCode::NOT_FOUND, "no such remote").into_response();
    };
    match remote.as_ref() {
        crate::remote::RemoteClientKind::Catacomb(_) => {
            match tokio::task::spawn_blocking(move || match remote.as_ref() {
                crate::remote::RemoteClientKind::Catacomb(c) => c.library_json(),
                _ => unreachable!(),
            })
            .await
            {
                Ok(Ok(v)) => Json(v).into_response(),
                Ok(Err(e)) => (StatusCode::BAD_GATEWAY, format!("remote error: {e}")).into_response(),
                Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "remote task failed").into_response(),
            }
        }
        crate::remote::RemoteClientKind::Peertube(_) => {
            (StatusCode::NOT_IMPLEMENTED, "PeerTube browsing arrives in a later update").into_response()
        }
    }
}
```

This needs `RemoteClient::has_password()` and `PeerTubeClient::has_password()` — add each (Step 6).

- [ ] **Step 6: Add `has_password`/`base_url` accessors**

`src/remote.rs`, in `impl RemoteClient` (public accessor near `base_url`):
```rust
    pub fn has_password(&self) -> bool {
        self.password.is_some()
    }
```
`src/peertube.rs`, in `impl PeerTubeClient`:
```rust
    pub fn base_url(&self) -> &str {
        &self.api_base
    }
    pub fn has_password(&self) -> bool {
        self.password.is_some()
    }
```
(Remove the `#[allow(dead_code)]` on `PeerTubeClient`/`impl` from phase 1 now that it's constructed — or leave it; either compiles. Prefer removing the struct-level allow so genuine future dead code still warns.)

- [ ] **Step 7: Migrate `App.remotes` + desktop use-sites**

In `src/app.rs`:

Field (line ~264):
```rust
    remotes: Vec<std::sync::Arc<crate::remote::RemoteClientKind>>,
```

Construction (line ~549):
```rust
        let remotes: Vec<std::sync::Arc<crate::remote::RemoteClientKind>> = config.remotes.iter()
            .map(|r| std::sync::Arc::new(crate::remote::RemoteClientKind::from_section(r)))
            .collect();
```

`remotes_screen` name access (line ~2570): `r.name` → `r.name()`.

`start_remote_fetch` (line ~2523) — dispatch + stopgap:
```rust
    fn start_remote_fetch(&mut self, idx: usize) {
        let Some(client) = self.remotes.get(idx).cloned() else { return };
        self.remote_selected = Some(idx);
        self.remote_library = None;
        match client.as_ref() {
            crate::remote::RemoteClientKind::Catacomb(_) => {
                self.remote_status = format!("Connecting to {}…", client.name());
                let (tx, rx) = std::sync::mpsc::channel();
                self.remote_rx = Some(rx);
                let repaint_ctx = self.egui_ctx.clone();
                std::thread::spawn(move || {
                    let res = match client.as_ref() {
                        crate::remote::RemoteClientKind::Catacomb(c) => c.library(),
                        _ => unreachable!(),
                    };
                    let _ = tx.send(res);
                    repaint_ctx.request_repaint();
                });
            }
            crate::remote::RemoteClientKind::Peertube(_) => {
                self.remote_status =
                    "PeerTube browsing arrives in a later update".to_string();
            }
        }
    }
```

- [ ] **Step 8: Build both front-ends + full tests**

Run: `cargo build --release 2>&1 | grep -E "^error" | head`
Expected: no output.
Run: `cargo test --release 2>&1 | grep "test result"`
Expected: all `ok`, 0 failed (+1 unit vs phase 1).

- [ ] **Step 9: Commit**

```bash
export SSH_AUTH_SOCK=/tmp/luna-ssh-agent.sock
git add src/remote.rs src/peertube.rs src/web.rs src/app.rs
git commit -m "feat(remote): RemoteClientKind enum + kind-aware live remotes list

Both front-ends hold Vec<Arc<RemoteClientKind>> (web behind RwLock). Catacomb
browse unchanged; PeerTube remotes report a phase-3 stopgap when browsed.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: Web editor backend — merge + PUT + test endpoints

**Files:**
- Modify: `src/web.rs` — `merge_remote_passwords`; `put_remotes`; `test_remote`; routes.
- Modify: `tests/api.rs` — integration coverage.

**Interfaces:**
- Consumes: Task 1 `RemoteClientKind`, `RemoteClient`/`PeerTubeClient` reachability calls.
- Produces: `PUT /api/remotes`, `POST /api/remotes/test`; pure `merge_remote_passwords(inputs: &[RemoteInput], existing: &[RemoteSection]) -> Vec<RemoteSection>`.

- [ ] **Step 1: Write the failing merge unit test**

Add to `src/web.rs` in a `#[cfg(test)] mod tests` block (create with `use super::*;` if none):

```rust
#[test]
fn merge_keeps_blank_password_by_url() {
    use crate::config::{RemoteKind, RemoteSection};
    let existing = vec![RemoteSection {
        name: "old".into(), url: "http://p:8081".into(),
        kind: RemoteKind::Catacomb, username: None, password: Some("SECRET".into()),
    }];
    let inputs = vec![
        RemoteInput { name: "renamed".into(), url: "http://p:8081".into(),
            kind: RemoteKind::Catacomb, username: None, password: None }, // blank → keep
        RemoteInput { name: "new".into(), url: "http://q:8081".into(),
            kind: RemoteKind::Catacomb, username: None, password: Some("TYPED".into()) },
    ];
    let out = merge_remote_passwords(&inputs, &existing);
    assert_eq!(out[0].password.as_deref(), Some("SECRET")); // preserved by URL
    assert_eq!(out[0].name, "renamed");
    assert_eq!(out[1].password.as_deref(), Some("TYPED"));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --release merge_keeps_blank 2>&1 | grep -E "cannot find|test result"`
Expected: FAIL — `RemoteInput`/`merge_remote_passwords` not found.

- [ ] **Step 3: Implement the input type + merge + endpoints**

Add to `src/web.rs` (module scope):

```rust
#[derive(serde::Deserialize)]
pub struct RemoteInput {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub kind: crate::config::RemoteKind,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
}

/// Resolve write-only passwords: keep the typed password if non-empty, else
/// adopt the stored password of the existing remote with the same URL, else
/// None. Trims URLs so whitespace can't defeat the match.
pub fn merge_remote_passwords(
    inputs: &[RemoteInput],
    existing: &[crate::config::RemoteSection],
) -> Vec<crate::config::RemoteSection> {
    inputs
        .iter()
        .map(|i| {
            let url = i.url.trim().to_string();
            let password = match i.password.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
                Some(p) => Some(p.to_string()),
                None => existing
                    .iter()
                    .find(|e| e.url.trim() == url)
                    .and_then(|e| e.password.clone()),
            };
            let username = i.username.as_deref().map(str::trim).filter(|u| !u.is_empty()).map(String::from);
            crate::config::RemoteSection {
                name: i.name.trim().to_string(),
                url,
                kind: i.kind.clone(),
                username,
                password,
            }
        })
        .collect()
}

/// `PUT /api/remotes` — replace the whole peer list (live-apply).
async fn put_remotes(
    State(state): State<Arc<WebState>>,
    Json(body): Json<Vec<RemoteInput>>,
) -> impl IntoResponse {
    let merged = {
        let mut cfg = state.config.lock_recover();
        let merged = merge_remote_passwords(&body, &cfg.remotes);
        cfg.remotes = merged.clone();
        if let Err(e) = cfg.save(&state.config_path) {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("save failed: {e}")).into_response();
        }
        merged
    }; // config lock dropped here
    let rebuilt: Vec<std::sync::Arc<crate::remote::RemoteClientKind>> = merged
        .iter()
        .map(|r| std::sync::Arc::new(crate::remote::RemoteClientKind::from_section(r)))
        .collect();
    *state.remotes.write().unwrap() = rebuilt;
    Json(serde_json::json!({ "ok": true })).into_response()
}

/// `POST /api/remotes/test` — reachability check for a (possibly unsaved) peer.
async fn test_remote(
    State(state): State<Arc<WebState>>,
    Json(body): Json<RemoteInput>,
) -> impl IntoResponse {
    // Resolve a blank password from the stored remote with the same URL.
    let section = {
        let cfg = state.config.lock_recover();
        merge_remote_passwords(std::slice::from_ref(&body), &cfg.remotes)
            .into_iter()
            .next()
            .unwrap()
    };
    let result = tokio::task::spawn_blocking(move || {
        match crate::remote::RemoteClientKind::from_section(&section) {
            crate::remote::RemoteClientKind::Catacomb(c) => c.library_json().map(|_| None::<usize>),
            crate::remote::RemoteClientKind::Peertube(p) => p.list_channels().map(|ch| Some(ch.len())),
        }
    })
    .await;
    match result {
        Ok(Ok(channels)) => Json(serde_json::json!({ "ok": true, "channels": channels })).into_response(),
        Ok(Err(e)) => Json(serde_json::json!({ "ok": false, "error": e })).into_response(),
        Err(_) => Json(serde_json::json!({ "ok": false, "error": "test task failed" })).into_response(),
    }
}
```

Add the routes (near the existing `/api/remotes` route, line ~3364):
```rust
        .route("/api/remotes", get(get_remotes).put(put_remotes))
        .route("/api/remotes/test", post(test_remote))
```
(Keep the existing `.route("/api/remotes/:id/library", get(get_remote_library))`.)

- [ ] **Step 4: Run the merge test to verify it passes**

Run: `cargo test --release merge_keeps_blank 2>&1 | grep "test result"`
Expected: `test result: ok. 1 passed`.

- [ ] **Step 5: Add integration coverage**

In `tests/api.rs`, add a test that (following the file's existing spawn-server helper) `PUT /api/remotes` with a catacomb + a peertube entry, then `GET /api/remotes` and asserts both appear with the right `kind` and `has_password`. Model it on the nearest existing settings/endpoint test in that file (reuse its server-spawn + curl helpers; skip if curl absent, as the file already does). Assert the JSON contains `"kind":"peertube"` and `"kind":"catacomb"` and that no plaintext password is present in the GET body.

- [ ] **Step 6: Build + full tests**

Run: `cargo build --release 2>&1 | grep -E "^error" | head` → no output.
Run: `cargo test --release 2>&1 | grep "test result"` → all `ok`.

- [ ] **Step 7: Commit**

```bash
export SSH_AUTH_SOCK=/tmp/luna-ssh-agent.sock
git add src/web.rs tests/api.rs
git commit -m "feat(web): PUT /api/remotes + test endpoint, kind-aware, live-apply

URL-keyed write-only password merge; whole-list replace rebuilds the live
RwLock client list; POST /api/remotes/test checks reachability per kind.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: Web UI editor section

**Files:**
- Modify: `src/web_ui/index.html` — a "Federation peers" section in the Settings modal + its JS.

**Interfaces:**
- Consumes: `GET/PUT /api/remotes`, `POST /api/remotes/test` (Tasks 1–2).

- [ ] **Step 1: Add the section markup**

Inside the Settings modal (find the settings form container in `src/web_ui/index.html`), add a section using existing theme vars:

```html
<div class="settings-section" id="remotesSection">
  <h3>Federation peers</h3>
  <p class="muted">Browse other Catacomb instances, or PeerTube channels, from this one.</p>
  <div id="remotesRows"></div>
  <button type="button" class="btn" id="addRemoteBtn">+ Add peer</button>
  <button type="button" class="btn btn-primary" id="saveRemotesBtn">Save peers</button>
  <span id="remotesStatus" class="muted"></span>
</div>
```

(Match the surrounding markup's class names; `settings-section`/`btn`/`muted` are placeholders — use whatever the modal already uses.)

- [ ] **Step 2: Add the JS**

In the `<script>` block, add (adapt fetch/`api()` helper to the file's existing one):

```javascript
let remotesModel = []; // {name,url,kind,username,has_password,password?}

async function loadRemotes() {
  const r = await api('/api/remotes');
  remotesModel = (await r.json()).map(x => ({
    name: x.name, url: x.url, kind: x.kind || 'catacomb',
    username: x.username || '', has_password: !!x.has_password, password: ''
  }));
  renderRemotes();
}

function renderRemotes() {
  const box = document.getElementById('remotesRows');
  box.innerHTML = '';
  remotesModel.forEach((rm, i) => {
    const row = document.createElement('div');
    row.className = 'remote-row';
    row.innerHTML = `
      <select data-i="${i}" class="rm-kind">
        <option value="catacomb"${rm.kind==='catacomb'?' selected':''}>Catacomb</option>
        <option value="peertube"${rm.kind==='peertube'?' selected':''}>PeerTube</option>
      </select>
      <input data-i="${i}" class="rm-name" placeholder="name" value="${escapeHtml(rm.name)}">
      <input data-i="${i}" class="rm-url" placeholder="url" value="${escapeHtml(rm.url)}">
      <input data-i="${i}" class="rm-user" placeholder="username" value="${escapeHtml(rm.username)}"
             style="display:${rm.kind==='peertube'?'inline-block':'none'}">
      <input data-i="${i}" class="rm-pass" type="password"
             placeholder="${rm.has_password?'password set — leave blank to keep':'password'}">
      <button type="button" data-i="${i}" class="rm-test">Test</button>
      <button type="button" data-i="${i}" class="rm-del">✕</button>
      <span class="rm-result muted" data-i="${i}"></span>`;
    box.appendChild(row);
  });
  box.querySelectorAll('.rm-kind').forEach(el => el.onchange = e => {
    remotesModel[+e.target.dataset.i].kind = e.target.value; renderRemotes();
  });
  const bind = (cls, field) => box.querySelectorAll('.'+cls).forEach(el =>
    el.oninput = e => { remotesModel[+e.target.dataset.i][field] = e.target.value; });
  bind('rm-name','name'); bind('rm-url','url'); bind('rm-user','username'); bind('rm-pass','password');
  box.querySelectorAll('.rm-del').forEach(el => el.onclick = e => {
    remotesModel.splice(+e.target.dataset.i, 1); renderRemotes();
  });
  box.querySelectorAll('.rm-test').forEach(el => el.onclick = async e => {
    const i = +e.target.dataset.i, rm = remotesModel[i];
    const out = box.querySelector('.rm-result[data-i="'+i+'"]');
    out.textContent = 'testing…';
    const r = await api('/api/remotes/test', { method:'POST', headers:{'Content-Type':'application/json'},
      body: JSON.stringify({ url: rm.url, kind: rm.kind, username: rm.username, password: rm.password || null }) });
    const j = await r.json();
    out.textContent = j.ok ? ('✓ ok' + (j.channels!=null?` (${j.channels} channels)`:'')) : ('✗ ' + (j.error||'failed'));
  });
}

document.getElementById('addRemoteBtn').onclick = () => {
  remotesModel.push({ name:'', url:'', kind:'catacomb', username:'', has_password:false, password:'' });
  renderRemotes();
};
document.getElementById('saveRemotesBtn').onclick = async () => {
  const payload = remotesModel.map(rm => ({
    name: rm.name, url: rm.url, kind: rm.kind,
    username: rm.kind==='peertube' ? (rm.username||null) : null,
    password: rm.password ? rm.password : null
  }));
  const r = await api('/api/remotes', { method:'PUT', headers:{'Content-Type':'application/json'}, body: JSON.stringify(payload) });
  document.getElementById('remotesStatus').textContent = r.ok ? 'Saved.' : 'Save failed.';
  if (r.ok) { await loadRemotes(); if (typeof loadRemoteSwitcher==='function') loadRemoteSwitcher(); }
};
```

Call `loadRemotes()` when the Settings modal opens (add to the existing settings-open handler). `escapeHtml` and `api()` should already exist in the file; if `escapeHtml` doesn't, add a small helper.

- [ ] **Step 3: Syntax-check the SPA JS**

Run: `awk '/<script>/{f=1;next}/<\/script>/{f=0}f' src/web_ui/index.html > /tmp/spa.js && node --check /tmp/spa.js && echo "JS OK"`
Expected: `JS OK`.

- [ ] **Step 4: Build + visual check**

Run: `cargo build --release 2>&1 | grep -E "^error" | head` → no output.
Then run the server against a scratch dir and confirm in a browser (or the headless-chromium harness per HANDOFF gotcha #6) that the Settings modal shows the section, Add/Test/Save work, the username field toggles with kind, and a saved catacomb peer still appears in the remote switcher.

- [ ] **Step 5: Commit**

```bash
export SSH_AUTH_SOCK=/tmp/luna-ssh-agent.sock
git add src/web_ui/index.html
git commit -m "feat(web-ui): federation peers editor (add/edit/remove/test, kind-aware)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: Desktop editor section + test thread

**Files:**
- Modify: `src/app.rs` — a "Federation peers" section in the Settings screen; a test-connection thread + result channel.

**Interfaces:**
- Consumes: Task 1 `RemoteClientKind`; `RemoteSection` editing on `self.config.remotes`.

- [ ] **Step 1: Add a test-result channel field**

In the `App` struct (near `remote_rx`), add:
```rust
    /// Test-connection result for the settings editor: (row index, message).
    remote_test_rx: Option<Receiver<(usize, String)>>,
```
Initialise `remote_test_rx: None` in the constructor.

- [ ] **Step 2: Render the editor section in the Settings screen**

In `settings_screen` (find the settings UI in `src/app.rs`), add a section:

```rust
ui.separator();
ui.heading("Federation peers");
ui.label(egui::RichText::new(
    "Browse other Catacomb instances, or PeerTube channels, from this one.")
    .weak().small());
let mut remove: Option<usize> = None;
let mut test: Option<usize> = None;
for (i, r) in self.config.remotes.iter_mut().enumerate() {
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_source(("rm-kind", i))
            .selected_text(match r.kind {
                crate::config::RemoteKind::Catacomb => "Catacomb",
                crate::config::RemoteKind::Peertube => "PeerTube",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut r.kind, crate::config::RemoteKind::Catacomb, "Catacomb");
                ui.selectable_value(&mut r.kind, crate::config::RemoteKind::Peertube, "PeerTube");
            });
        ui.add(egui::TextEdit::singleline(&mut r.name).hint_text("name").desired_width(90.0));
        ui.add(egui::TextEdit::singleline(&mut r.url).hint_text("url").desired_width(180.0));
        if r.kind == crate::config::RemoteKind::Peertube {
            let mut user = r.username.clone().unwrap_or_default();
            if ui.add(egui::TextEdit::singleline(&mut user).hint_text("username").desired_width(90.0)).changed() {
                r.username = if user.is_empty() { None } else { Some(user) };
            }
        }
        let mut pass = r.password.clone().unwrap_or_default();
        if ui.add(egui::TextEdit::singleline(&mut pass).password(true).hint_text("password").desired_width(90.0)).changed() {
            r.password = if pass.is_empty() { None } else { Some(pass) };
        }
        if ui.button("Test").clicked() { test = Some(i); }
        if ui.button("✕").clicked() { remove = Some(i); }
    });
}
if ui.button("+ Add peer").clicked() {
    self.config.remotes.push(crate::config::RemoteSection {
        name: String::new(), url: String::new(),
        kind: crate::config::RemoteKind::Catacomb, username: None, password: None,
    });
}
if let Some(i) = remove { self.config.remotes.remove(i); }
if let Some(i) = test { self.start_remote_test(i); }
if !self.remote_status.is_empty() {
    ui.label(egui::RichText::new(&self.remote_status).weak().small());
}
```

(Place this before the settings "Save" button. The existing Save path already calls `self.config.save(...)`; extend that handler — Step 4.)

- [ ] **Step 3: Add the test thread**

Add to `impl App`:
```rust
/// Reachability-test the remote at `self.config.remotes[idx]` on a thread.
fn start_remote_test(&mut self, idx: usize) {
    let Some(section) = self.config.remotes.get(idx).cloned() else { return };
    self.remote_status = format!("Testing {}…", section.name);
    let (tx, rx) = std::sync::mpsc::channel();
    self.remote_test_rx = Some(rx);
    let ctx = self.egui_ctx.clone();
    std::thread::spawn(move || {
        let msg = match crate::remote::RemoteClientKind::from_section(&section) {
            crate::remote::RemoteClientKind::Catacomb(c) => match c.library_json() {
                Ok(_) => "✓ reachable".to_string(),
                Err(e) => format!("✗ {e}"),
            },
            crate::remote::RemoteClientKind::Peertube(p) => match p.list_channels() {
                Ok(ch) => format!("✓ {} channels", ch.len()),
                Err(e) => format!("✗ {e}"),
            },
        };
        let _ = tx.send((idx, msg));
        ctx.request_repaint();
    });
}
```

- [ ] **Step 4: Drain the test result + rebuild remotes on save**

In `update()` (near the `remote_rx` drain), add:
```rust
if let Some((_, msg)) = self.remote_test_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
    self.remote_status = msg;
    self.remote_test_rx = None;
    ctx.request_repaint();
}
```

In the settings Save handler (where `self.config.save(&self.config_path)` runs), after a successful save add:
```rust
self.remotes = self.config.remotes.iter()
    .map(|r| std::sync::Arc::new(crate::remote::RemoteClientKind::from_section(r)))
    .collect();
self.remote_selected = None;
self.remote_library = None;
```

- [ ] **Step 5: Build + full tests**

Run: `cargo build --release 2>&1 | grep -E "^error" | head` → no output.
Run: `cargo test --release 2>&1 | grep "test result"` → all `ok`.

- [ ] **Step 6: Manual verify (desktop)**

Launch the GUI (XWayland recipe from HANDOFF), open Settings, add a catacomb peer and a PeerTube remote (`https://framatube.org/c/<channel>`), Test each (expect ✓ + channel count for PeerTube), Save; confirm the Remotes screen lists both, catacomb browse works, and clicking the PeerTube remote shows the phase-3 stopgap message.

- [ ] **Step 7: Commit**

```bash
export SSH_AUTH_SOCK=/tmp/luna-ssh-agent.sock
git add src/app.rs
git commit -m "feat(desktop): federation peers editor (add/edit/remove/test, kind-aware)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:**
- Multi-kind storage (`RemoteClientKind` enum, both lists, web `RwLock`) → Task 1. ✓
- Browse dispatch + PeerTube stopgap → Task 1 Steps 5/7. ✓
- Editor UI (kind selector + conditional username, both UIs) → Tasks 3/4. ✓
- Web API deltas (`GET` kind/has_password, `PUT`, `POST /test`) → Tasks 1/2. ✓
- URL-keyed write-only password merge → Task 2 `merge_remote_passwords` + test. ✓
- Test-connection kind-branched → Task 2 (web) + Task 4 (desktop). ✓
- Live-apply rebuild both fronts → Task 2 `put_remotes`, Task 4 Step 4. ✓
- Testing: `from_section` unit, merge unit, `tests/api.rs` PUT/GET round-trip, manual → Tasks 1/2/3/4. ✓

**Placeholder scan:** UI class-name/`api()`/`escapeHtml`/settings-container references are explicitly "match the file's existing helper/markup" — they name the exact integration point, not vague work. All logic steps carry concrete code.

**Type consistency:** `RemoteClientKind::{from_section,name,kind}`, `RemoteInput { name,url,kind,username,password }`, `merge_remote_passwords(&[RemoteInput], &[RemoteSection]) -> Vec<RemoteSection>`, and the `has_password`/`base_url` accessors are named/typed identically where defined (Tasks 1/2) and used (Tasks 2/3/4). `RemoteKind` is `Clone` (config phase 1) so `i.kind.clone()` in merge is valid. ✓
```
