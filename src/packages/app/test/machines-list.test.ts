import { test } from "node:test";
import assert from "node:assert/strict";
import type { MachineStatus } from "@agent-hud/protocol";
import {
  machineTimes,
  showsMachines,
} from "../src/settings/agents/machines-list.ts";

const en = (english: string) => english;
const zh = (_: string, chinese: string) => chinese;
const now = Date.parse("2026-11-14T12:00:00Z");
const far: MachineStatus = {
  id: "a",
  label: "Studio",
  state: "connected",
  stateText: "Connected",
  lastContactAt: "2026-11-14T11:59:18Z",
  historyCheckedAt: "2026-11-14T11:00:00Z",
};

test("only this machine and no note: no section", () => {
  assert.equal(showsMachines([], undefined), false);
  assert.equal(showsMachines([{ here: true }], undefined), false);
  assert.equal(showsMachines([{ here: true }], "List unread"), true);
  assert.equal(showsMachines([], "List unread"), true);
  assert.equal(showsMachines([{ here: true }, {}], undefined), true);
});

test("a connected far machine shows its two times", () => {
  assert.equal(
    machineTimes(far, now, "en", en),
    "Last contact 42s ago · History checked 1h ago",
  );
  assert.equal(
    machineTimes(far, now, "zh", zh),
    "最近联系 42s 前 · 历史检查于 1h 前",
  );
});

test("a time not known shows nothing, never a zero or 1970", () => {
  const { historyCheckedAt: _, ...noHistory } = far;
  assert.equal(machineTimes(noHistory, now, "en", en), "Last contact 42s ago");
  const { lastContactAt: __, ...neither } = noHistory;
  assert.equal(machineTimes(neither, now, "en", en), null);
  const epoch = { ...far, lastContactAt: "1970-01-01T00:00:00Z" };
  assert.doesNotMatch(machineTimes(epoch, now, "en", en)!, /Last contact/);
  const garbage = { ...neither, historyCheckedAt: "soon" };
  assert.equal(machineTimes(garbage, now, "en", en), null);
});

test("times show only for a far machine that is connected", () => {
  for (const state of [
    "connecting",
    "not-trusted",
    "cannot-reach",
    "not-covered",
    "same-machine",
  ] as const)
    assert.equal(machineTimes({ ...far, state }, now, "en", en), null, state);
  assert.equal(machineTimes({ ...far, here: true }, now, "en", en), null);
});
