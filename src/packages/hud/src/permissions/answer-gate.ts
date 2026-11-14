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
