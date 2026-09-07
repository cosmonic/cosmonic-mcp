#!/usr/bin/env bash
# Sync this repo's source from the cosmonic/desktop monorepo, which is where
# the server is developed alongside the daemon it drives.
#
#   ./scripts/sync-from-desktop.sh ../desktop
#   ./scripts/sync-from-desktop.sh ../desktop --check    # CI: fail on drift
#
# Two things are copied, and nothing else:
#
#   daemon/crates/cosmonic-mcp/{src,skills,README.md}  → src/, skills/, README.md
#   daemon/crates/cosmonic-api/src                     → crates/cosmonic-api/src
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

SRC="$DESKTOP/daemon/crates"
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
sync_dir "$SRC/cosmonic-api/src"    "$ROOT/crates/cosmonic-api/src" || rc=1

if [[ "$CHECK" == "--check" ]]; then
  [[ $rc -eq 0 ]] && echo "in sync with $DESKTOP"
  exit $rc
fi

# The version the handshake reports must track the Desktop release this was cut
# from, or a client cannot tell which server it is talking to.
VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$DESKTOP/daemon/Cargo.toml" | head -1)"
sed -i.bak "s/^version = \".*\"$/version = \"$VERSION\"/" "$ROOT/Cargo.toml"
rm -f "$ROOT/Cargo.toml.bak"

echo "synced from $DESKTOP (version $VERSION)"
echo "next: cargo build --release --bin cosmonic-mcp && cargo test"
