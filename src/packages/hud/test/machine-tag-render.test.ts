// The machine tag as the real card, queue row and panel row draw it: the
// label shown (cut past 24 characters), its whole label on hover and to a
// screen reader, and nothing at all for a session on this machine.
import { after, test } from "node:test";
import assert from "node:assert/strict";
import { createElement as h, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { Approval, Snapshot } from "@agent-hud/protocol";
import { close, load, machineTags } from "./render.ts";

after(close);

const SHORT = "Mac mini Lab";
const LONG = "Build server in the downstairs closet behind the big router!";
const LONG_SHOWN = "Build server in the down…";

const request = (machine?: string): Approval => ({
  id: "far",
  title: "Which dates work?",
  summary: "Which dates work?",
  source: "hermes",
  toolName: "clarify",
  input: "",
  sessionId: "s",
  createdAt: new Date().toISOString(),
  allowsAlways: false,
  kind: "question",
  vendor: "Hermes Agent",
  ...(machine ? { machine } : {}),
});

type Rows = {
  PermissionRowHead: ComponentType<{ request: Approval; open: boolean }>;
  PermissionClosedRow: ComponentType<{
    request: Approval;
    onSelect: () => void;
  }>;
};

test("a card and a queue row draw the machine's tag, whole on hover and to a screen reader", async () => {
  const { PermissionRowHead, PermissionClosedRow } = await load<Rows>(
    "/src/alerts/PendingEventRows.tsx",
  );
  for (const [label, shown] of [
    [SHORT, SHORT],
    [LONG, LONG_SHOWN],
  ]) {
    for (const markup of [
      renderToStaticMarkup(
        h(PermissionRowHead, { request: request(label), open: true }),
      ),
      renderToStaticMarkup(
        h(PermissionClosedRow, { request: request(label), onSelect() {} }),
      ),
    ]) {
      assert.deepEqual(machineTags(markup), [
        { shown, title: label, name: label },
      ]);
      // The client is still named beside it.
      assert.match(markup, /Hermes Agent/);
    }
  }
});

test("a card of this machine draws no tag", async () => {
  const { PermissionRowHead } = await load<Rows>(
    "/src/alerts/PendingEventRows.tsx",
  );
  const markup = renderToStaticMarkup(
    h(PermissionRowHead, { request: request(), open: true }),
  );
  assert.deepEqual(machineTags(markup), []);
  assert.doesNotMatch(markup, /machine-label/);
});

test("a hover panel session row draws its machine's tag, and a local one none", async () => {
  const { SourceSessions } = await load<{
    SourceSessions: ComponentType<Record<string, unknown>>;
  }>("/src/panel/SessionsSection.tsx");
  const at = new Date().toISOString();
  const row = (id: string, machine?: string) => ({
    id,
    source: "hermes",
    title: "Plan the weekend trip",
    cwd: null,
    tokens: 1000,
    updatedAt: at,
    lastMessageAt: at,
    ...(machine ? { machine } : {}),
  });
  const snapshot = {
    sessions: [row("far", SHORT), row("local"), row("long", LONG)],
    providers: [],
    sessionsRevision: 1,
  } as unknown as Snapshot;
  const markup = renderToStaticMarkup(
    h(SourceSessions, {
      snapshot,
      light: false,
      Logo: () => null,
      onStats() {},
    }),
  );
  assert.deepEqual(machineTags(markup), [
    { shown: SHORT, title: SHORT, name: SHORT },
    { shown: LONG_SHOWN, title: LONG, name: LONG },
  ]);
});
