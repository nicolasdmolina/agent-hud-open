// Agent HUD Fork (ticket 08, FORK.md): Settings never lists the phone
// pairing page, even when a product page asks to sit before it.
import { test } from "node:test";
import assert from "node:assert/strict";
import { sidebarPages } from "../src/settings/pages.ts";

test("the fork's Settings sidebar has no Local Network page", () => {
  const ids = sidebarPages([
    {
      id: "extra",
      title: "Extra",
      before: "local-network",
      render: () => null,
    },
  ]).map((page) => page.id);
  assert.ok(!ids.includes("local-network"));
  assert.deepEqual(ids, [
    "general",
    "display",
    "agents",
    "agent-plugins",
    "relays",
    "prices",
    "updates",
    "extra",
  ]);
});
