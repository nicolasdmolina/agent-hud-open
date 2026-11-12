#!/bin/sh
# Builds the fork's app bundle and puts it in /Applications (FORK.md).
# Usage: sh src/tooling/fork-install.sh [--no-open]
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
name="Agent HUD Fork"
# Own target folder: a folder shared between worktrees can reuse another
# worktree's crates, and that would be what gets installed.
export CARGO_TARGET_DIR="$root/target"
built="$CARGO_TARGET_DIR/release/bundle/macos/$name.app"
pnpm --dir "$root" --filter @agent-hud/desktop exec tauri build --bundles app \
  --config "$root/src/apps/desktop/src-tauri/tauri.fork.conf.json"
if pgrep -xq "$name"; then osascript -e "quit app \"$name\""; fi
while pgrep -xq "$name"; do sleep 1; done
rm -rf "/Applications/$name.app"
ditto "$built" "/Applications/$name.app"
echo "Installed /Applications/$name.app"
[ "${1:-}" = "--no-open" ] || open "/Applications/$name.app"
