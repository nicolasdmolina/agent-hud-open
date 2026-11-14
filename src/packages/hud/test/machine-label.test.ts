import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  MACHINE_LABEL_MAX,
  machineLabelText,
  oneLine,
} from "../src/machines/machine-label.ts";
import { whoBesideMachine } from "../src/alerts/row-head-label.ts";
import { mayDecide, outOfReach } from "../src/permissions/answer-gate.ts";

const source = (path: string) =>
  readFileSync(new URL(path, import.meta.url), "utf8");

test("a label of 24 characters or fewer shows whole", () => {
  assert.equal(MACHINE_LABEL_MAX, 24);
  assert.equal(machineLabelText("Mac mini 24"), "Mac mini 24");
  assert.equal(machineLabelText("x".repeat(24)), "x".repeat(24));
});

test("a longer label is cut to 24 characters and an ellipsis", () => {
  const long = "Build server in the downstairs closet behind the router";
  assert.equal(long.length > 50, true);
  const shown = machineLabelText(long);
  assert.equal(shown, "Build server in the down…");
  assert.equal([...shown].length, MACHINE_LABEL_MAX + 1);
});

test("wide characters and emoji count as one character each", () => {
  const wide = "楼下书房里的那台很旧的苹果迷你电脑它还在跑着很多个代理会话呢";
  const shown = machineLabelText(wide);
  assert.equal([...shown].length, MACHINE_LABEL_MAX + 1);
  assert.ok(shown.endsWith("…"));
  assert.ok(wide.startsWith(shown.slice(0, -1)));
  // A family emoji is several code points and one character.
  const family = "👨‍👩‍👧‍👦".repeat(30);
  const cut = machineLabelText(family);
  assert.equal(cut, `${"👨‍👩‍👧‍👦".repeat(24)}…`);
});

test("a label is one line", () => {
  assert.equal(oneLine("  Mac\n mini\t2 "), "Mac mini 2");
  assert.equal(machineLabelText("Mac\nmini"), "Mac mini");
});

test("beside a machine, the client's name gives way first", () => {
  const closed = whoBesideMachine({ flexShrink: 0, maxWidth: 150 });
  assert.equal(closed.maxWidth, 150);
  assert.ok(Number(closed.flexShrink) >= 100);
  assert.ok(Number(closed.minWidth) > 0);
});

test("a row without a machine draws exactly what it drew before", () => {
  const head = source("../src/alerts/PermissionRowHead.tsx");
  assert.match(head, /request\.machine \? \(\s*<MachineLabel/);
  assert.match(head, /\) : null\}/);
  const panel = source("../src/panel/SessionsSection.tsx");
  assert.match(panel, /session\.machine \? \(/);
});

test("a card out of reach takes no decision but leaving it alone", () => {
  const marked = { unreachable: "Hermes on Studio cannot be reached" };
  assert.equal(outOfReach(marked), marked.unreachable);
  for (const decision of [
    { kind: "allow" },
    { kind: "deny" },
    { kind: "always" },
    { kind: "answer", answers: { q: { selected: ["A"] } } },
    { kind: "approve_plan", acceptEdits: false },
    { kind: "keep_planning", message: "more" },
  ] as const)
    assert.equal(mayDecide(marked, decision), false, decision.kind);
  assert.equal(mayDecide(marked, { kind: "leave" }), true);
  for (const reachable of [{}, { unreachable: null }, { unreachable: "" }]) {
    assert.equal(outOfReach(reachable), null);
    assert.equal(mayDecide(reachable, { kind: "allow" }), true);
  }
});

test("every decision a card makes goes through the gate", () => {
  const actions = source("../src/RequestActions.tsx");
  assert.match(
    actions,
    /if \(!onDecision \|\| pending \|\| !mayDecide\(request, decision\)\) return;/,
  );
  // Only `answer` calls onDecision, and the cards are handed `answer`.
  assert.equal(actions.match(/onDecision\(/g)?.length, 1);
  assert.match(
    actions,
    /const disabled = !canDismiss \|\| Boolean\(unreachable\);/,
  );
  // Dismiss keeps its own switch.
  for (const card of ["PermissionQuestionCard", "PermissionPlanCard"])
    assert.match(
      source(`../src/permissions/${card}.tsx`),
      /canDismiss = !disabled/,
    );
  assert.match(
    source("../src/permissions/PermissionQuestionCard.tsx"),
    /<QuestionAnswerElsewhere\s+request=\{request\}\s+disabled=\{!canDismiss\}/,
  );
});
