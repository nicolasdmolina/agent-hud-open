#!/bin/sh
# Builds the fork's app bundle and puts it in /Applications (FORK.md).
# Usage: sh src/tooling/fork-install.sh [--no-open]
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
name="Agent HUD Fork"
built="${CARGO_TARGET_DIR:-$root/target}/release/bundle/macos/$name.app"
pnpm --dir "$root" --filter @agent-hud/desktop exec tauri build --bundles app \
  --config "$root/src/apps/desktop/src-tauri/tauri.fork.conf.json"
osascript -e "quit app \"$name\"" 2>/dev/null || true
while pgrep -xq "$name"; do sleep 1; done
rm -rf "/Applications/$name.app"
ditto "$built" "/Applications/$name.app"
echo "Installed /Applications/$name.app"
[ "${1:-}" = "--no-open" ] || open "/Applications/$name.app"
