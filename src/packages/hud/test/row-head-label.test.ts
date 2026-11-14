import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  OPEN_LABEL_SHARE,
  rowHeadLabel,
} from "../src/alerts/row-head-label.ts";

test("a closed queue row keeps the 150 px cap on its label", () => {
  assert.deepEqual(rowHeadLabel(false, false), {
    flexShrink: 0,
    maxWidth: 150,
  });
  assert.deepEqual(rowHeadLabel(false, true), rowHeadLabel(false, false));
});

test("an open question with no summary gives its label the row", () => {
  assert.deepEqual(rowHeadLabel(true, true), { flexShrink: 1, minWidth: 0 });
});

// "Hermes Agent · hud-spike" was cut to "Hermes Agent · hud-spi…" on an open approval card, whose summary shares
// the row. The label must not shrink for the summary, and must stop short of the row so the summary still shows.
test("an open card with a summary keeps its label whole up to a share of the row", () => {
  const style = rowHeadLabel(true, false);
  assert.equal(style.flexShrink, 0);
  assert.equal(style.maxWidth, OPEN_LABEL_SHARE);
  const share = Number.parseFloat(OPEN_LABEL_SHARE);
  assert.ok(share >= 50 && share <= 65, `share ${OPEN_LABEL_SHARE}`);
});

test("the row head takes its label's width from rowHeadLabel", () => {
  const source = readFileSync(
    new URL("../src/alerts/PermissionRowHead.tsx", import.meta.url),
    "utf8",
  );
  assert.match(source, /: rowHeadLabel\(open, hideSummary\)\),/);
  assert.doesNotMatch(source, /maxWidth: (hideSummary \? undefined : )?150/);
});
