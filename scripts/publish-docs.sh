#!/usr/bin/env bash
# Build the mdBook docs and publish them to Codeberg Pages.
#
# Usage:
#   scripts/publish-docs.sh
#
# Codeberg serves the `pages` branch of a repo at
# https://<user>.codeberg.page/<repo>/. This script renders docs/ with
# mdBook and force-pushes the rendered HTML to that branch. The source
# (docs/src/) is untouched; only the orphan `pages` branch is rewritten.
#
# We do this locally rather than in CI because Codeberg does not execute
# Forgejo Actions (no runners) — see ROADMAP / docs/architecture.
#
# Auth: pushes via `origin`'s configured credentials by default. To use a
# scoped token non-interactively (e.g. write:repository only), export
#   CODEBERG_TOKEN=<token>
# and it will be injected into the push URL for this run only.
#
# Prerequisites are installed on demand:
#   - mdbook → cargo install mdbook --no-default-features --features search

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT" || exit 1

BRANCH="pages"

die() { echo "error: $*" >&2; exit 1; }

# cargo-installed binaries (mdbook) aren't on PATH in non-interactive shells.
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"

# --- mdbook -----------------------------------------------------------------
if ! command -v mdbook >/dev/null 2>&1; then
    echo "==> mdbook not found; installing via cargo"
    cargo install mdbook --no-default-features --features search \
        || die "could not install mdbook"
fi

# --- build ------------------------------------------------------------------
echo "==> building the book"
mdbook build docs || die "mdbook build failed"
[ -f docs/book/index.html ] || die "no docs/book/index.html after build"

# --- resolve push URL + the resulting Pages URL -----------------------------
origin="$(git remote get-url origin 2>/dev/null)" \
    || die "no 'origin' remote — run from a clone with a Codeberg remote"

push_url="$origin"
if [ -n "${CODEBERG_TOKEN:-}" ]; then
    # Inject token into an https remote: https://<token>@host/owner/repo.git
    push_url="$(printf '%s' "$origin" | sed -E "s#^https://([^@/]+@)?#https://${CODEBERG_TOKEN}@#")"
fi

# owner/repo from the remote → https://<owner>.codeberg.page/<repo>/
# Handles both https://host/owner/repo(.git) and git@host:owner/repo(.git).
clean="${origin%.git}"          # drop trailing .git
repo="${clean##*/}"             # last path component
rest="${clean%/*}"              # everything before it
owner="${rest##*[:/]}"          # component after the last ':' or '/'
pages_url="https://${owner}.codeberg.page/${repo}/"

# --- publish ----------------------------------------------------------------
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cp -r docs/book/. "$work"/

echo "==> publishing to '$BRANCH'"
(
    cd "$work" || exit 1
    git init -q
    git config user.name  "$(git -C "$ROOT" config user.name  || echo docs)"
    git config user.email "$(git -C "$ROOT" config user.email || echo docs@localhost)"
    git add -A
    git commit -q -m "docs build $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    git push -f "$push_url" "HEAD:$BRANCH"
) || die "push to '$BRANCH' failed"

echo
echo "==> published. Live (allow a minute on first deploy) at:"
echo "    $pages_url"
