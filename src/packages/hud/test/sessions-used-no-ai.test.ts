// The panel's short Sessions list at its first paint, drawn from the
// snapshot's rows: a session Core marks as having used no AI (`usedNoAi`)
// is left out, and "+N more" counts only the rest. A session that waits or
// runs is never marked by Core, so it is drawn (ticket 70).
import { after, test } from "node:test";
import assert from "node:assert/strict";
import { createElement as h, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { Snapshot } from "@agent-hud/protocol";
import { close, load } from "./render.ts";

after(close);

const row = (id: string, title: string, minutesAgo: number, extra = {}) => {
  const at = new Date(Date.now() - minutesAgo * 60_000).toISOString();
  return {
    id,
    source: "hermes",
    title,
    cwd: null,
    tokens: 0,
    state: "idle",
    updatedAt: at,
    lastMessageAt: at,
    ...extra,
  };
};

async function draw(sessions: unknown[]) {
  const { SourceSessions } = await load<{
    SourceSessions: ComponentType<Record<string, unknown>>;
  }>("/src/panel/SessionsSection.tsx");
  const snapshot = {
    sessions,
    providers: [],
    sessionsRevision: 1,
  } as unknown as Snapshot;
  return renderToStaticMarkup(
    h(SourceSessions, {
      snapshot,
      light: false,
      Logo: () => null,
      onStats() {},
    }),
  );
}

test("sessions that used no AI are left out of the short list and its count", async () => {
  const jobs = Array.from({ length: 7 }, (_, n) =>
    row(`job-${n}`, "Hourly report job", n, { usedNoAi: true }),
  );
  const markup = await draw([
    ...jobs,
    row("chat", "Telegram chat about the trip", 30, { tokens: 1200 }),
    row("asks", "Waiting on a question", 40, { state: "waiting" }),
  ]);
  assert.doesNotMatch(markup, /Hourly report job/);
  assert.match(markup, /Telegram chat about the trip/);
  assert.match(markup, /Waiting on a question/);
  assert.doesNotMatch(markup, /more/);
});

test("past five sessions that used AI, the count names only those", async () => {
  const worked = Array.from({ length: 7 }, (_, n) =>
    row(`chat-${n}`, `Chat ${n}`, n + 10, { tokens: 500 }),
  );
  const markup = await draw([
    row("job", "Hourly report job", 0, { usedNoAi: true }),
    ...worked,
  ]);
  assert.match(markup, /\+2 more/);
  assert.doesNotMatch(markup, /Hourly report job/);
});

test("with only sessions that used no AI, the list does not say there are none", async () => {
  const markup = await draw([
    row("job-1", "Hourly report job", 5, { usedNoAi: true }),
    row("job-2", "Hourly report job", 65, { usedNoAi: true }),
  ]);
  assert.doesNotMatch(markup, /Hourly report job/);
  assert.doesNotMatch(markup, /No sessions yet/);
  assert.doesNotMatch(markup, /No sessions match/);
  // The heading still leads to the Sessions page, where they are, and still
  // names its window.
  assert.match(markup, /title="Show sessions"/);
  assert.match(markup, /Last 24 hours/);
  assert.doesNotMatch(markup, /visibility:hidden/);
});

test("with no sessions at all, it still says so", async () => {
  assert.match(await draw([]), /No sessions yet/);
});
