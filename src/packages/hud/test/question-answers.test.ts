import { test } from "node:test";
import assert from "node:assert/strict";
import type { ApprovalQuestion } from "@agent-hud/protocol";
import {
  canGoOn,
  isAnswered,
  writtenAnswers,
} from "../src/permissions/question-answers.ts";

const question = (
  answerKey: string,
  labels: string[],
  lock: ApprovalQuestion["lock"] = null,
): ApprovalQuestion => ({
  id: answerKey,
  answerKey,
  question: answerKey,
  header: null,
  options: labels.map((label) => ({ label, description: null })),
  multiSelect: true,
  lock,
});

const colour = question("q0", ["blue (Recommended)", "green"], {
  state: "answered",
  answer: "blue",
});
const sizes = question("q1", ["small (Recommended)", "medium", "large"]);

test("picks go in the order they were offered, typed words only when chosen", () => {
  assert.deepEqual(
    writtenAnswers([colour, sizes], {
      q1: { selected: ["large", "small (Recommended)"], custom: "tiny" },
    }),
    { q1: { selected: ["small (Recommended)", "large"] } },
  );
  assert.deepEqual(
    writtenAnswers([colour, sizes], {
      q1: { selected: ["medium"], custom: " tiny ", own: true },
    }),
    { q1: { selected: ["medium"], custom: "tiny" } },
  );
});

test("a question left alone or skipped is not written", () => {
  // Left alone: another viewer's answer stands; nothing is sent for it.
  assert.deepEqual(
    writtenAnswers([colour, sizes], {
      q0: { selected: [] },
      q1: { selected: ["medium"] },
    }),
    { q1: { selected: ["medium"] } },
  );
  // Skipped on the card: left out, whatever was picked before.
  assert.deepEqual(
    writtenAnswers(
      [colour, sizes],
      { q0: { selected: ["green"] }, q1: { selected: ["medium"] } },
      "q1",
    ),
    { q0: { selected: ["green"] } },
  );
  // Nothing at all: an empty answer, which skips a lone question.
  assert.deepEqual(writtenAnswers([sizes], {}, "q1"), {});
});

test("a question another viewer settled may be passed without an answer", () => {
  assert.equal(isAnswered({ selected: [] }), false);
  assert.equal(isAnswered({ selected: [], custom: "teal" }), false);
  assert.equal(isAnswered({ selected: [], custom: "teal", own: true }), true);
  assert.equal(canGoOn(sizes, { selected: [] }), false);
  assert.equal(canGoOn(colour, { selected: [] }), true);
  assert.equal(
    canGoOn(question("q2", [], { state: "skipped" }), { selected: [] }),
    true,
  );
});
