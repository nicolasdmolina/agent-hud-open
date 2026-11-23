// Agent HUD Fork (ticket 66): Approve automatically never answers a client
// whose requests always wait for the person (Hermes; Core marks it
// `by-hand`), so Settings offers its switch only where it can do something.
import type { ClientRow } from "@agent-hud/protocol";

/** Whether Approve automatically can answer `client`'s permission requests
 * at all: it answers in the HUD, and not by hand only. */
export function canApprove(client: Pick<ClientRow, "features">) {
  const features = client.features ?? [];
  return features.includes("answer") && !features.includes("by-hand");
}
