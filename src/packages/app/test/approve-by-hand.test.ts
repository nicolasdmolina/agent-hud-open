import { test } from "node:test";
import assert from "node:assert/strict";
import { canApprove } from "../src/settings/agents/approve-by-hand.ts";

// Agent HUD Fork (ticket 66): Approve automatically never answers a client
// whose requests always wait for the person (Hermes), so Settings neither
// offers its switch there nor tags the client as approving.
test("a client answered in the HUD can be approved automatically", () => {
  assert.equal(canApprove({ features: ["live", "answer", "talk"] }), true);
});

test("a client answered by hand cannot, whatever the switch says", () => {
  assert.equal(canApprove({ features: ["live", "answer", "by-hand"] }), false);
});

test("a client that is not answered here has no switch either", () => {
  assert.equal(canApprove({ features: ["live"] }), false);
  assert.equal(canApprove({}), false);
});
