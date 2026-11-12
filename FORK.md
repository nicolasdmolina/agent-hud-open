# Fork notes

This is a private-use fork of [jazzenchen/agent-hud-open](https://github.com/jazzenchen/agent-hud-open). All of the fork's changes live on the branch `fork`, which is the default branch here. `main` mirrors upstream and never carries fork commits. Pull requests target `fork`. Nothing is sent upstream.

## Differences from upstream

Add one row for every difference the fork introduces.

| Area | Change | Files | Reason | Ticket |
| ---- | ------ | ----- | ------ | ------ |

## Updating from upstream

Remotes: `origin` is this fork, `upstream` is the author's repo with pushing disabled.

1. Fetch both remotes: `git fetch upstream && git fetch origin`.
2. Fast-forward the fork's `main` to upstream: `git push origin upstream/main:main`. This must be a fast-forward. If it is refused, someone committed to `main`; move that commit to `fork` first.
3. Bring `fork` up to date by merging, not rebasing: `git checkout fork && git merge origin/main`, fix any conflicts, then `git push origin fork`. Merging keeps `fork` history stable, so open ticket branches built on it stay valid and no force push to a shared branch is needed.
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
