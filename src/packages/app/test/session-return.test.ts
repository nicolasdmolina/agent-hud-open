import { test } from "node:test";
import assert from "node:assert/strict";
import { returnLabel } from "../src/lib/sessions.ts";

test("a way back Core names is called what it does (ticket 39)", () => {
  const hermes = {
    navigationTarget: { kind: "hermes-app" },
    navigationLabel: "Open in Hermes desktop",
  };
  assert.equal(returnLabel(hermes, false, true), "Open in Hermes desktop");
  // Core makes the words in the app's language; the page shows them.
  const zh = { ...hermes, navigationLabel: "在 Hermes 桌面版中打开" };
  assert.equal(returnLabel(zh, true, true), "在 Hermes 桌面版中打开");
});

test("every other way back reads as it did", () => {
  const target = { navigationTarget: { kind: "cursor-app" } };
  assert.equal(returnLabel(target, false, true), "Return to session");
  assert.equal(returnLabel(target, true, true), "回到会话");
  assert.equal(returnLabel({}, false, true), "Reveal log in Finder");
  assert.equal(returnLabel({}, true, true), "在访达中显示日志");
  assert.equal(returnLabel({}, false, false), "Reveal log");
  assert.equal(returnLabel({}, true, false), "显示日志");
  // A label with no way back names nothing.
  const stale = { navigationLabel: "Open in Hermes desktop" };
  assert.equal(returnLabel(stale, false, true), "Reveal log in Finder");
});
