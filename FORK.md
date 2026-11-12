# Fork notes

This is a private-use fork of [jazzenchen/agent-hud-open](https://github.com/jazzenchen/agent-hud-open). All of the fork's changes live on the branch `fork`, which is the default branch here. `main` mirrors upstream and never carries fork commits. Pull requests target `fork`. Nothing is sent upstream.

## Differences from upstream

Add one row for every difference the fork introduces.

| Area | Change | Files | Reason | Ticket |
| ---- | ------ | ----- | ------ | ------ |
| Hermes sessions | Each Hermes session shows "Hermes Agent", or "Hermes Agent · <profile>" for a named profile, in the Sessions list, the session page and search. Database readers can now name a session's client. The Hermes reader version goes from 2 to 3 so sessions already read are read again and pick up the label. | `src/crates/agents/src/transcript/records.rs`, `src/crates/agents/src/hermes/transcript.rs`, `src/crates/agents/src/naming/clients.rs`, `src/crates/core/src/source_db/scan.rs`, `src/crates/core/src/source_db.rs` | Profiles looked the same. Matches the Swift reference (`HermesSessions.swift:62-66`), so it is no difference from Swift. | 09 |
| Identity | Name `Agent HUD Fork` and bundle id `com.nickymolina.agenthud`, set by an overlay passed at build time; the base config is untouched | `src/apps/desktop/src-tauri/tauri.fork.conf.json` | Never confused with the official app; its web view data and preferences live under the fork's id | 03 |
| Identity | Settings and the menu bar item's tooltip say the fork's name | `src/apps/desktop/src/main.tsx`, `src/crates/host/src/tray.rs` | Tell the fork from the official app at a glance | 03 |
| Data folder | Folder `com.nickymolina.agenthud` on every system, in place of `app.agenthud.mac`/`.windows`/`.linux`; the identifier test reads the overlay | `src/crates/platform/src/data_folder.rs` | Never reads or writes the official app's data. Not `AGENTHUD_DATA_DIR`: hook callbacks inherit the agent's environment and would dial the default folder | 03 |
| Data folder | Cursor's turn log is read from the fork's folder | `src/crates/agents/src/cursor/mod.rs` | Follows the folder above | 03 |
| Instance lock | Hook owners lock `~/Library/Caches/com.nickymolina.agenthud/instance.lock`, not `~/Library/Caches/app.agenthud/instance.lock`; parity rows c03 #83 and c09 #3 say so | `src/crates/platform/src/lib.rs`, `docs/parity/` | The fork and the official app never refuse or hand over to each other | 03 |
| Naming | Processes started by bundle `com.nickymolina.agenthud` count as Agent HUD | `src/crates/agents/src/naming/apps.rs` | Sessions the fork starts are labeled as upstream labels its own | 03 |
| Self-update | None, verified: the open build has no updater plugin or feed URL, and Settings → Updates says the build does not update itself. A merge that adds `tauri-plugin-updater` or a product extension's update check must be turned off here | — | Never fetch the author's feeds or replace the fork with the official release | 03 |
| Install | `src/tooling/fork-install.sh` builds the bundle with the overlay and installs it to `/Applications/Agent HUD Fork.app`. `src/apps/installer` is upstream's release installer and is not used | `src/tooling/fork-install.sh` | One command for every later rebuild | 03 |

## Updating from upstream

Remotes: `origin` is this fork, `upstream` is the author's repo with pushing disabled.

1. Fetch both remotes: `git fetch upstream && git fetch origin`.
2. Fast-forward the fork's `main` to upstream: `git push origin upstream/main:main`. This must be a fast-forward. If it is refused, someone committed to `main`; move that commit to `fork` first.
3. Bring `fork` up to date by merging, not rebasing: `git checkout fork && git merge --ff-only origin/fork && git merge origin/main`, fix any conflicts, then `git push origin fork`. Merging keeps `fork` history stable, so open ticket branches built on it stay valid and no force push to a shared branch is needed.
4. Run the checks:

   ```sh
   pnpm install --frozen-lockfile
   pnpm run format
   pnpm run check
   pnpm run test
   pnpm run lint
   cargo test --workspace
   git diff --check
   ```

5. Rebuild and reinstall: `sh src/tooling/fork-install.sh`. It builds the bundle with the fork's overlay, quits the running fork, replaces `/Applications/Agent HUD Fork.app` and opens it (`--no-open` skips that). It always builds in this checkout's own `target/`: a build folder shared between worktrees can reuse another worktree's crates.
