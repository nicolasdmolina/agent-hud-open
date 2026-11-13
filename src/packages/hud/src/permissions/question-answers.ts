import type {
  Approval,
  ApprovalQuestion,
  RequestAnswer,
} from "@agent-hud/protocol";

/** A question's draft: the offered answers picked, the words typed, and
 * whether those words are the answer (`own`). Skipping or picking a single
 * option sets the words aside without erasing them, as Open's
 * QuestionDraft does. */
export type Draft = RequestAnswer & { own?: boolean };

/** The answers a card sends, by answer key: picks in the order they were
 * offered, typed words only where they are the chosen answer. A question
 * with neither is left out, and so is `omitKey` (the one just skipped). */
export function writtenAnswers(
  questions: ApprovalQuestion[],
  answers: Record<string, Draft>,
  omitKey?: string,
): Record<string, RequestAnswer> {
  return Object.fromEntries(
    Object.entries(answers)
      .map(([key, value]) => {
        const offered =
          questions
            .find((q) => q.answerKey === key)
            ?.options.map((o) => o.label) ?? value.selected;
        const selected = offered.filter((label) =>
          value.selected.includes(label),
        );
        const custom = value.own ? value.custom?.trim() : undefined;
        return [key, { selected, ...(custom ? { custom } : {}) }] as const;
      })
      .filter(
        ([key, value]) =>
          key !== omitKey && (value.selected.length > 0 || "custom" in value),
      ),
  );
}

/** Whether the draft answers the question. */
export function isAnswered(draft: Draft): boolean {
  return (
    draft.selected.length > 0 || Boolean(draft.own && draft.custom?.trim())
  );
}

/** Whether the card may go on from the question: it is answered, or another
 * viewer already answered or skipped it and leaving it alone keeps that. */
export function canGoOn(question: ApprovalQuestion, draft: Draft): boolean {
  return isAnswered(draft) || Boolean(question.lock);
}

/** Whether the card offers Skip on the question: not where the client takes
 * no skip, nor where another viewer settled it (going on without an answer
 * keeps theirs). */
export function offersSkip(request: Approval, question: ApprovalQuestion) {
  return request.canSkip !== false && !question.lock;
}
