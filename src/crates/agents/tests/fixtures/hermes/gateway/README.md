# Hermes gateway recordings

Frames recorded on 2026-11-12 from Hermes 0.22.5 (`9847fb8d5dd0`) over the `hermes-gateway-v1` WebSocket, for the tests in `src/hermes/gateway/tests/`. Every scenario was recorded from a throwaway gateway with its own `HOME` and `HERMES_HOME`, a profile named `hud-spike`, `approvals.mode: manual`, `approvals.timeout: 20`, `agent.clarify_timeout: 20` and a scripted loopback model. `real-gateway-readonly-aggregate.json` and one entry of `gateway-self-description.json` come from a live gateway, read only, as counts and field names.

`gateway-self-description-old-build.SYNTHETIC.json` was not recorded. No older gateway was at hand, so it is written from the checks Hermes's own client applies (`hermes_cli/gateway_runtime.py`).

## Shape of a scenario file

```
{ "scenario", "description", "captured", "notes": {...},
  "clients": { "<name>": [ { "t": seconds since that client connected, "dir": "in" | "out", "frame": {...} } ] } }
```

- `hud` is the client under test, which is what Agent HUD will be. `hud-reconnected` is its second socket after a drop or a restart, and `hud-host` a socket whose ticket covers every profile.
- `dir: "out"` frames are what the client sent, `dir: "in"` what the gateway sent.
- A frame with `method: "event"` is a gateway event of kind `params.type`.
- A frame with a string id starting `srq-` and a `method` is a request the gateway makes of its viewers (a set of questions). It is answered with a result frame carrying the same id.
- `notes` holds what the scenario pulled out: replies, what the model saw, database rows.

## What was removed or replaced

All question and answer text is synthetic. The home path and login name are `/Users/USER`. System prompts are removed, tool and skill inventories and colour tables trimmed, and strings over 1500 characters cut. A ticket never travels in a frame (it rides the WebSocket subprotocol header), and ticket fields in notes are masked.

Three more changes were made when the recordings were copied here:

- The frames of the session's own client (`other`, `stranger`) are left out. The model never sees that socket.
- `tools`, which the scrub had replaced with a string, is `{}`. The contract says an object or null.
- Host account ids are `uid:1000`.

`the_fixtures_carry_nothing_personal` checks the paths, tickets and account ids on every run.

## Files

| File | Scenario |
|---|---|
| `gateway-self-description.json` | identify (ready, starting, stopped), ticket grant, `gateway.ready`, `runtime.describe`, refusals |
| `gateway-self-description-old-build.SYNTHETIC.json` | not recorded: identify answers Hermes's own client rejects |
| `ticket-lifetime.json` | single use, 30 s expiry, HTTP 403 otherwise |
| `session-list.json` | `session.list`, `session.active_list`, `session.info`, two profiles, a host's socket, `profiles.list` |
| `session-list-scheduled-job.json` | a cron run appearing as `source: cron` |
| `real-gateway-readonly-aggregate.json` | a live gateway, counts only |
| `question-answered-here.json` | `clarify.request`, `clarify.respond`, `clarify.settled` |
| `question-answered-elsewhere.json` | another client answers; late answer `already_resolved` |
| `question-race.json` | both answer at once; the first wins |
| `question-open-ended.json` | no choices, a typed answer |
| `question-two-in-one-call.json` | two cards in sequence, the second with several picks |
| `question-skipped.json` | an empty answer |
| `question-timeout.json` | nobody answers; `clarify.settled` at 20 s |
| `question-peek-without-subscribing.json` | status `waiting`, `session.events.since`, an answer refused until subscribed |
| `question-batch-answered-here.json` | `clarify` request (`srq-` id), answered by a result frame, `request.cancel` reason `resolved` |
| `question-batch-answered-elsewhere.json` | the same, answered by the other client |
| `question-batch-timeout.json` | `request.cancel` reason `timeout` |
| `approval-answered-once.json`, `-session.json`, `-always.json`, `-deny.json` | the four choices |
| `approval-after-always-new-session.json` | no prompt after `always` |
| `approval-answered-elsewhere.json` | another client approves |
| `approval-race.json` | both answer at once; the first wins |
| `approval-invalid-answers.json` | unknown choice, wrong generation, wrong prompt id, not subscribed |
| `approval-timeout.json` | nobody answers; `approval.settled` arrives only at the end of the turn |
| `approval-limited-choice.json` | choices `["once","deny"]` only |
| `approval-cancelled-by-interrupt.json` | the turn is stopped while it waits |
| `session-ending-closed-by-its-client.json` | not subscribed; the session leaves the live list |
| `session-ending-while-hud-subscribed.json` | subscribed; the close does not end the session |
| `session-ending-client-vanished.json` | closed for want of a viewer after the grace period |
| `connection-drop-and-replay.json` | reconnect, `session.events.since`, a wrong cursor |
| `gateway-restart-graceful.json` | close code 1012, a new instance, no replay |
| `gateway-restart-hard.json` | close code 1006, the turn left `unknown` |
| `session-ids-and-hooks.json`, `hook-payloads-same-session.json` | the same session id in the gateway, `state.db` and the hook payloads; several picks as a JSON list |
| `subscribe-side-effect-policy-refresh.json` | subscribing to an idle session re-commits its policy; a busy one is left alone |
