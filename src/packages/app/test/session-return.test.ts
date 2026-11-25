import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { addCatalog, translator } from "@agent-hud/ui/format";
import { returnLabel } from "../src/lib/sessions.ts";

// The app's text in English and in Simplified Chinese, from its catalogs.
for (const language of ["en", "zh-Hans"] as const)
  addCatalog(
    language,
    JSON.parse(
      readFileSync(
        join(import.meta.dirname, `../../protocol/locales/${language}.json`),
        "utf8",
      ),
    ),
  );
const en = translator("en");
const zh = translator("zh-Hans");

test("a way back Core names is called what it does (ticket 39)", () => {
  const hermes = {
    navigationTarget: { kind: "hermes-app" },
    navigationLabel: "Open in Hermes desktop",
  };
  assert.equal(returnLabel(hermes, en, true), "Open in Hermes desktop");
  // Core makes the words in the app's language; the page shows them.
  const named = { ...hermes, navigationLabel: "在 Hermes 桌面版中打开" };
  assert.equal(returnLabel(named, zh, true), "在 Hermes 桌面版中打开");
});

test("every other way back reads as it did", () => {
  const target = { navigationTarget: { kind: "cursor-app" } };
  assert.equal(returnLabel(target, en, true), "Return to session");
  assert.equal(returnLabel(target, zh, true), "回到会话");
  assert.equal(returnLabel({}, en, true), "Reveal log in Finder");
  assert.equal(returnLabel({}, zh, true), "在访达中显示日志");
  assert.equal(returnLabel({}, en, false), "Reveal log");
  assert.equal(returnLabel({}, zh, false), "显示日志");
  // A label with no way back names nothing.
  const stale = { navigationLabel: "Open in Hermes desktop" };
  assert.equal(returnLabel(stale, en, true), "Reveal log in Finder");
});
