// The Sessions window and Settings as they draw a far machine: the tag on a
// session's row, its whole label on the session's page, and the machines
// list in Chinese from what core sends in Chinese.
import { after, test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { createElement as h, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { MachineStatus, Snapshot } from "@agent-hud/protocol";
import { close, load, machineTags } from "./render.ts";

after(close);

const SHORT = "Mac mini Lab";
const LONG = "Build server in the downstairs closet behind the big router!";
const LONG_SHOWN = "Build server in the down…";
// Any colour the theme is asked for.
const theme = new Proxy(
  { zh: false, light: false } as Record<string, unknown>,
  {
    get: (target, key: string) => (key in target ? target[key] : "#888"),
  },
);
const at = new Date().toISOString();
const session = (machine?: string) => ({
  id: "hermes:far",
  source: "hermes",
  client: "Hermes Agent",
  title: "Plan the weekend trip",
  model: "claude-sonnet-4-5",
  cwd: null,
  tokens: 1000,
  cost: 0.01,
  updatedAt: at,
  lastMessageAt: at,
  startedAt: at,
  state: "idle",
  ...(machine ? { machine } : {}),
});

test("a Sessions row draws its machine's tag, whole on hover and to a screen reader", async () => {
  const { SessionRow } = await load<{
    SessionRow: ComponentType<Record<string, unknown>>;
  }>("/src/sessions/SessionRow.tsx");
  const row = (machine?: string) =>
    renderToStaticMarkup(
      h(SessionRow, {
        session: session(machine),
        client: "Hermes Agent",
        theme,
        now: Date.now(),
        onSelect() {},
        first: true,
      }),
    );
  assert.deepEqual(machineTags(row(SHORT)), [
    { shown: SHORT, title: SHORT, name: SHORT },
  ]);
  assert.deepEqual(machineTags(row(LONG)), [
    { shown: LONG_SHOWN, title: LONG, name: LONG },
  ]);
  // The client is named as before, and a local session draws no tag.
  assert.match(row(LONG), /claude-sonnet-4-5 · Hermes Agent/);
  assert.deepEqual(machineTags(row()), []);
});

test("a session's page writes its machine's whole label", async () => {
  const { DetailHeader } = await load<{
    DetailHeader: ComponentType<Record<string, unknown>>;
  }>("/src/sessions/SessionDetailHeader.tsx");
  const page = (machine?: string) =>
    renderToStaticMarkup(
      h(DetailHeader, {
        session: session(machine),
        detail: null,
        client: "Hermes Agent",
        now: Date.now(),
        theme,
        reveal: null,
        revealLabel: "",
      }),
    );
  const line = page(LONG).match(
    /class="session-statistics-machine"[^>]*>.*?<span[^>]*>([^<]*)<\/span>/,
  );
  assert.equal(line?.[1], LONG);
  assert.doesNotMatch(page(), /session-statistics-machine/);
});

// What core sends for the same machines in Chinese, as its own test
// (core/tests/machine_words.rs) checks it: a file read, as shared fixtures are.
const words = JSON.parse(
  readFileSync(
    join(
      import.meta.dirname,
      "../../../crates/core/tests/fixtures/machine-words.json",
    ),
    "utf8",
  ),
);

test("the machines list says each state in Chinese, in core's words", async () => {
  const { MachinesSection } = await load<{
    MachinesSection: ComponentType<Record<string, unknown>>;
  }>("/src/settings/agents/MachinesSection.tsx");
  const machines = words.snapshot["zh-CN"] as MachineStatus[];
  const snapshot = {
    language: "zh",
    settings: { language: "zh-CN" },
    machines: { [words.source]: machines },
    machinesNote: { [words.source]: words.listNote },
  } as unknown as Snapshot;
  const t = (_en: string, zh: string) => zh;
  const markup = renderToStaticMarkup(
    h(MachinesSection, { t, snapshot, source: { id: words.source } }),
  );
  const shown = [
    ...markup.matchAll(
      /class="settings-machine-state"[^>]*>(?:<span[^>]*><\/span>)?([^<]*)</g,
    ),
  ].map((match) => match[1]);
  assert.deepEqual(shown, [
    "已连接",
    "已连接",
    "正在连接",
    "尚未信任",
    "无法连接",
    "不支持",
    "与另一项是同一台机器",
  ]);
  assert.deepEqual(
    shown,
    machines.map((machine) => machine.stateText),
  );
  // Its own words are Chinese too, and none of the English ones show.
  assert.match(markup, /<h2>机器<\/h2>/);
  assert.match(markup, /最近联系 /);
  assert.match(markup, /无法读取连接列表。/);
  assert.doesNotMatch(
    markup,
    /Connected|Connecting|Cannot reach|Not covered|Last contact/,
  );
});
