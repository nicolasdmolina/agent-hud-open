import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { permissionChoices } from "../src/permissions/permission-choices.ts";

// Cards a client adapter really shows, and the buttons they must get. The
// Rust test hermes::gateway::tests::buttons checks the same file from the
// other side: the cards are the model's, and each decision is the frame the
// client got in a recording.
// A file read, not an import, as the shared parity fixtures are read.
const contract = JSON.parse(
  readFileSync(
    join(
      import.meta.dirname,
      "../../../crates/agents/tests/fixtures/hermes/approval-card-buttons.json",
    ),
    "utf8",
  ),
);
const drawn = (card: Parameters<typeof permissionChoices>[0], zh = false) =>
  permissionChoices(card, zh).map(({ label, decision }) => ({
    label,
    decision,
  }));
const expected = (buttons: { label: string; decision: unknown }[]) =>
  buttons.map(({ label, decision }) => ({ label, decision }));

test("a card whose lasting choices Core labels gets one button for each", () => {
  const { card, buttons } = contract.full;
  assert.deepEqual(drawn(card), expected(buttons));
  // The session choice is never worded as always allowing.
  const session = drawn(card)[1];
  assert.equal(
    (session.decision as { suggestion: { scope: string } }).suggestion.scope,
    "session",
  );
  assert.doesNotMatch(session.label, /always/i);
});

test("a card that offers no lasting choice gets deny and allow once only", () => {
  const { card, buttons } = contract.limited;
  assert.deepEqual(drawn(card), expected(buttons));
});

test("one lasting choice offered alone is the only one drawn", () => {
  const [session, always] = contract.full.card.suggestions;
  for (const only of [session, always]) {
    const choices = drawn({ ...contract.full.card, suggestions: [only] });
    assert.deepEqual(
      choices.map((c) => c.label),
      ["Deny", only.label, "Allow once"],
    );
    assert.deepEqual(choices[1].decision, {
      kind: "always",
      suggestion: only,
    });
  }
});

test("a card with rules Core does not label keeps its one Always allow button", () => {
  // A hook client's card: its suggestion is a rule Core picks itself.
  const hook = {
    allowsAlways: true,
    alwaysAllowRule: "Edit(src/**)",
    allowLabel: null,
    suggestions: [
      {
        type: "addRules",
        behavior: "allow",
        destination: "localSettings",
        rules: [{ toolName: "Edit", ruleContent: "src/**" }],
      },
    ],
  };
  assert.deepEqual(permissionChoices(hook), [
    { tone: "deny", label: "Deny", decision: { kind: "deny" } },
    {
      tone: "lasting",
      label: "Always allow",
      decision: { kind: "always" },
      title: "Edit(src/**)",
    },
    { tone: "allow", label: "Allow once", decision: { kind: "allow" } },
  ]);
  // A native card that names its allow button and offers no rule.
  const native = {
    allowsAlways: false,
    allowLabel: "Allow this conversation",
    suggestions: [],
  };
  assert.deepEqual(
    drawn(native).map((c) => c.label),
    ["Deny", "Allow this conversation"],
  );
  assert.deepEqual(
    drawn({ ...native, allowLabel: null }, true).map((c) => c.label),
    ["拒绝", "批准一次"],
  );
});
