// The real cards as the app draws them: a question card's offered answers
// carry the mark that hands the HUD the keyboard and lets Enter submit from
// them; a permission card's buttons never do.
import { after, test } from "node:test";
import assert from "node:assert/strict";
import { createElement as h, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { Approval } from "@agent-hud/protocol";
import { close, load } from "./render.ts";

after(close);

const base: Approval = {
  id: "r",
  title: "t",
  summary: "t",
  source: "hermes",
  toolName: "clarify",
  input: "",
  sessionId: "s",
  createdAt: new Date().toISOString(),
  allowsAlways: false,
  kind: "question",
  vendor: "Hermes Agent",
};
const marks = (markup: string) =>
  [...markup.matchAll(/<button[^>]*data-hud-choice[^>]*>/g)].length;

test("each offered answer on a question card is marked, and nothing else", async () => {
  const { PermissionQuestionCard } = await load<{
    PermissionQuestionCard: ComponentType<Record<string, unknown>>;
  }>("/src/permissions/PermissionQuestionCard.tsx");
  const markup = renderToStaticMarkup(
    h(PermissionQuestionCard, {
      request: {
        ...base,
        questions: [
          {
            answerKey: "q",
            question: "Which?",
            multiSelect: false,
            options: [{ label: "A" }, { label: "B" }],
          },
        ],
      },
      disabled: false,
      pending: false,
      answer() {},
    }),
  );
  assert.equal(marks(markup), 2);
});

test("no button on a permission card is marked, Always allow included", async () => {
  const { PermissionToolCard } = await load<{
    PermissionToolCard: ComponentType<Record<string, unknown>>;
  }>("/src/permissions/PermissionToolCard.tsx");
  const markup = renderToStaticMarkup(
    h(PermissionToolCard, {
      request: { ...base, kind: "tool", toolName: "Bash", allowsAlways: true },
      disabled: false,
      pending: false,
      answer() {},
    }),
  );
  assert.match(markup, /Always allow/);
  assert.equal(marks(markup), 0);
});
