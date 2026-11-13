import type { Approval, RequestDecision } from "@agent-hud/protocol";

/** One button on a permission card, and the decision it sends. */
export interface PermissionChoice {
  tone: "deny" | "lasting" | "allow";
  label: string;
  decision: RequestDecision;
  /** What the choice adds, shown on hover. */
  title?: string;
}

/** A lasting choice Core named itself, such as allowing for the session:
 * a suggestion with its own words. It goes back whole as the `always`
 * decision's suggestion, so the client knows which one was chosen. */
function labelled(suggestion: unknown): suggestion is { label: string } {
  return (
    typeof suggestion === "object" &&
    suggestion !== null &&
    typeof (suggestion as { label?: unknown }).label === "string" &&
    (suggestion as { label: string }).label.trim() !== ""
  );
}

/**
 * A permission card's buttons, in order: deny, the lasting choices, allow.
 * Where Core labels the lasting choices the card offers, each is a button
 * of its own and the card offers exactly those; otherwise a card that
 * allows always has the one Always allow button, which leaves the rule to
 * Core.
 */
export function permissionChoices(
  request: Pick<
    Approval,
    "allowsAlways" | "suggestions" | "alwaysAllowRule" | "allowLabel"
  >,
  zh = false,
): PermissionChoice[] {
  const t = (en: string, cn: string) => (zh ? cn : en);
  const named = (request.suggestions ?? []).filter(labelled);
  const lasting: PermissionChoice[] = named.length
    ? named.map((suggestion) => ({
        tone: "lasting",
        label: suggestion.label,
        decision: { kind: "always", suggestion },
      }))
    : request.allowsAlways
      ? [
          {
            tone: "lasting",
            label: t("Always allow", "总是允许"),
            decision: { kind: "always" },
            title: request.alwaysAllowRule ?? undefined,
          },
        ]
      : [];
  return [
    { tone: "deny", label: t("Deny", "拒绝"), decision: { kind: "deny" } },
    ...lasting,
    {
      tone: "allow",
      label: request.allowLabel ?? t("Allow once", "批准一次"),
      decision: { kind: "allow" },
    },
  ];
}
