# Fork notes

This is a private-use fork of [jazzenchen/agent-hud-open](https://github.com/jazzenchen/agent-hud-open). All of the fork's changes live on the branch `fork`, which is the default branch here. `main` mirrors upstream and never carries fork commits. Pull requests target `fork`. Nothing is sent upstream.

## Differences from upstream

Add one row for every difference the fork introduces.

| Area | Change | Files | Reason | Ticket |
| ---- | ------ | ----- | ------ | ------ |
| Sound | Core decides which moments deserve a sound: `needs-you` when a session starts waiting on the user (a request, a question, or a wait its hooks report), `finished` when a turn ends; steps make none, and each alert has an id played once. Several requests that newly wait in one session at one refresh make one `needs-you` (one ping per session per refresh); none of them sounds later on its own. A card and a transcript sighting count as one request only when both name the same tool call, or, where the card's hook names none (Claude Code's `PermissionRequest`), while both are visible at once as the only possible pairing; replaced without overlap they are two requests, preferring an occasional extra sound to a silenced question. Snapshot field `soundAlerts`; settings `sound` (off), `soundNeedsYou` (`Glass`), `soundFinished` (`Hero`). No playback yet. | `src/crates/core/src/sound_alerts.rs`, `core/src/presentation/requests.rs`, `core/src/requests/views.rs`, `agents/src/hooks/request.rs` (`call_id`), `agents/src/transcript/session_state.rs` (`callId`), `core/src/settings.rs`, `core/src/lib.rs`, `core/src/snapshot.rs`, `src/packages/protocol/src/{settings,requests,snapshot}.ts` | Upstream removed its sound on 2026-10-21 (parity c09 #40b); Nicky wants to hear when an agent needs him or finishes without watching the panel. | 05 |

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

5. Rebuild the app with the fork's build command. Placeholder: ticket 04 adds this command.
6. Quit the running app, replace it in `/Applications` with the new build, and relaunch it.
