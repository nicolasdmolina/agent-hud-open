#!/bin/sh
# Builds the fork's app bundle and puts it in /Applications (FORK.md).
# Usage: sh src/tooling/fork-install.sh [--no-open]
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
name="Agent HUD Fork"
app="/Applications/$name.app"
# Own target folder: a folder shared between worktrees can reuse another
# worktree's crates, and that would be what gets installed.
export CARGO_TARGET_DIR="$root/target"
built="$CARGO_TARGET_DIR/release/bundle/macos/$name.app"
pnpm --dir "$root" --filter @agent-hud/desktop exec tauri build --bundles app \
  --config "$root/src/apps/desktop/src-tauri/tauri.fork.conf.json"
# Copy next to the installed app first, so a failed copy leaves it in place.
staged="/Applications/.$name.new-$$.app"
trap 'rm -rf "$staged"' EXIT
ditto "$built" "$staged"
id=$(/usr/libexec/PlistBuddy -c "Print :CFBundleIdentifier" "$staged/Contents/Info.plist")
[ "$id" = "com.nickymolina.agenthud" ] || { echo "Staged bundle id is $id" >&2; exit 1; }
if pgrep -xq "$name"; then osascript -e "quit app \"$name\""; fi
waited=0
while pgrep -xq "$name"; do
  [ "$waited" -lt 30 ] || { echo "$name did not quit in 30 s" >&2; exit 1; }
  sleep 1 && waited=$((waited + 1))
done
# Swap: the old copy stays until the new one is in place.
old="/Applications/.$name.old-$$.app"
[ ! -e "$app" ] || mv "$app" "$old"
mv "$staged" "$app" || { [ ! -e "$old" ] || mv "$old" "$app"; exit 1; }
rm -rf "$old"
echo "Installed $app"
[ "${1:-}" = "--no-open" ] || open "$app"
