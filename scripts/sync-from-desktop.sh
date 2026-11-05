#!/usr/bin/env bash
# Sync this repo's source from the cosmonic/desktop monorepo, which is where
# the server is developed alongside the daemon it drives.
#
#   ./scripts/sync-from-desktop.sh ../desktop
#   ./scripts/sync-from-desktop.sh ../desktop --check    # CI: fail on drift
#
# Two crates are copied, and nothing else:
#
#   rust/crates/cosmonic-mcp/{src,skills,ui}  → src/, skills/, ui/
#   rust/crates/cosmonic-api/src              → crates/cosmonic-api/src
#
# `ui/index.html` is the MCP App (Desktop's screens, built in the monorepo by
# js/packages/cosmonic-mcp-app) that src/ui.rs embeds with include_str!; it is
# copied as built, so this repo builds without Node.
#
# The crates moved from daemon/crates to rust/crates in desktop #552; an older
# checkout still syncs from daemon/crates.
#
# `cosmonic-api` is the wire contract between the daemon and this server, and
# depends on nothing but serde — carrying a copy is cheaper than reimplementing
# it and keeps the two from drifting silently.
#
# The Cargo.toml files are NOT synced: the monorepo's are workspace-relative
# and this repo's are standalone. Dependency versions are the ones to check by
# hand when the monorepo's move.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DESKTOP="${1:?usage: sync-from-desktop.sh <path-to-cosmonic/desktop> [--check]}"
CHECK="${2:-}"

WORKSPACE="$DESKTOP/rust"
[[ -d "$WORKSPACE/crates/cosmonic-mcp/src" ]] || WORKSPACE="$DESKTOP/daemon"
SRC="$WORKSPACE/crates"
[[ -d "$SRC/cosmonic-mcp/src" ]] || { echo "not a desktop checkout: $DESKTOP" >&2; exit 1; }

sync_dir() {
  local from="$1" to="$2"
  if [[ "$CHECK" == "--check" ]]; then
    diff -ru "$from" "$to" || { echo "DRIFT: $to is not $from" >&2; return 1; }
  else
    rm -rf "$to"
    mkdir -p "$(dirname "$to")"
    cp -R "$from" "$to"
  fi
}

rc=0
sync_dir "$SRC/cosmonic-mcp/src"    "$ROOT/src"                    || rc=1
sync_dir "$SRC/cosmonic-mcp/skills" "$ROOT/skills"                 || rc=1
if [[ -d "$SRC/cosmonic-mcp/ui" ]]; then
  sync_dir "$SRC/cosmonic-mcp/ui"   "$ROOT/ui"                     || rc=1
fi
sync_dir "$SRC/cosmonic-api/src"    "$ROOT/crates/cosmonic-api/src" || rc=1

if [[ "$CHECK" == "--check" ]]; then
  [[ $rc -eq 0 ]] && echo "in sync with $DESKTOP"
  exit $rc
fi

# The version the handshake reports must track the Desktop release this was cut
# from, or a client cannot tell which server it is talking to.
VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$WORKSPACE/Cargo.toml" | head -1)"
sed -i.bak "s/^version = \".*\"$/version = \"$VERSION\"/" "$ROOT/Cargo.toml"
rm -f "$ROOT/Cargo.toml.bak"

echo "synced from $DESKTOP (version $VERSION)"
echo "next: cargo build --release --bin cosmonic-mcp && cargo test"
