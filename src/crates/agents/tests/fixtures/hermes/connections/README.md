# Hermes desktop connection lists

Synthetic lists for `src/hermes/connections/tests.rs`. None was copied from a real `connections.json`. The shape of a real one, every value masked, is in ticket 19's `connection-list-shape.txt`: version 2, `primary`, `launchMode`, `lastUsed`, a `local` entry and an `ssh` entry whose `host` is an alias and which carries a `token` envelope. The other entries follow the desktop app's `connection-registry.ts` (`RegistryConnection`, and the legacy `cloud` entry that keeps a token).

Every token and header value is a made-up marker.

| File | What it holds |
|---|---|
| `real-shape.SYNTHETIC.json` | The real file's shape: `local`, and `ssh` with only a host alias and a token |
| `every-kind.SYNTHETIC.json` | `ssh` with every optional field, `remote` with a token and a header, `cloud` on OAuth, `cloud` stored with token auth, an unknown kind |
