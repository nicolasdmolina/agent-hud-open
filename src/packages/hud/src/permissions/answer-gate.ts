// The one way a card's decision reaches its client while the client may be
// out of reach.
import type { Approval, RequestDecision } from "@agent-hud/protocol";

/** Core's reason the request's client cannot be reached just now, or null. */
export function outOfReach(request: Pick<Approval, "unreachable">) {
  return request.unreachable || null;
}

/** Whether `decision` may be sent for `request`. While its client is out of
 * reach the card takes no answer, as Core refuses one; leaving it alone
 * (Dismiss) sends nothing to the client and still may. */
export function mayDecide(
  request: Pick<Approval, "unreachable">,
  decision: RequestDecision,
): boolean {
  return !outOfReach(request) || decision.kind === "leave";
}

/** One decision per card: none while one is on its way, and none after one
 * went, until delivery fails (the person may then act again) or the card
 * changes (a new card has a new gate). Leaving the card alone sends nothing
 * to the client and still may once an answer went. */
export function decisionGate() {
  let state: "open" | "sending" | "sent" = "open";
  return {
    begin(decision: RequestDecision): boolean {
      if (state === "sending") return false;
      if (state === "sent" && decision.kind !== "leave") return false;
      state = "sending";
      return true;
    },
    end(delivered: boolean) {
      state = delivered ? "sent" : "open";
    },
  };
}
