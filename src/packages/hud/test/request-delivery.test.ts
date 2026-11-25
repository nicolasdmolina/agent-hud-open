// The notice under a card that an earlier answer of its session did not
// arrive (Agent HUD Fork, ticket 66, round 8). The HUD picks the receipt
// by its session: the same plain id the card carries. Drawn from what
// core's snapshot carries, which core's own test (tests::delivery_notice)
// holds to the same file.
import { after, test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { Approval, RequestDelivery, Snapshot } from "@agent-hud/protocol";
import { close, load } from "./render.ts";

after(close);

// A file read, not an import, as the shared fixtures are read.
const fixture = JSON.parse(
  readFileSync(
    join(
      import.meta.dirname,
      "../../../crates/core/tests/fixtures/request-delivery-notice.json",
    ),
    "utf8",
  ),
) as { card: Approval; receipt: Omit<RequestDelivery, "at"> };

type Select = (
  snapshot: Snapshot,
  request: Approval | undefined,
  focusedKey: string | undefined,
) => RequestDelivery | undefined;

/** What the HUD's own selector picks for the card, as a component draws it. */
async function reasonShown(receipt: Omit<RequestDelivery, "at">) {
  const { useRequestDelivery } = await load<{ useRequestDelivery: Select }>(
    "/src/scene/request-delivery.ts",
  );
  const at = "2026-11-25T10:00:00.000+00:00";
  const snapshot = {
    generatedAt: "2026-11-25T10:00:30.000+00:00",
    requestDeliveries: [{ ...receipt, at }],
  } as Snapshot;
  function Notice() {
    const delivery = useRequestDelivery(snapshot, fixture.card, undefined);
    return createElement("p", null, delivery?.reason ?? "no notice");
  }
  return renderToStaticMarkup(createElement(Notice));
}

test("a failed answer's reason shows under an open card of its session", async () => {
  assert.equal(fixture.receipt.sessionId, fixture.card.sessionId);
  assert.equal(await reasonShown(fixture.receipt), "<p>fixture refusal</p>");
});

test("a receipt that names its session any other way is not that card's", async () => {
  // What round 7 wrote by mistake: core's own key for the session.
  const keyed = {
    ...fixture.receipt,
    sessionId: `${fixture.receipt.source}:${fixture.receipt.sessionId}`,
  };
  assert.equal(await reasonShown(keyed), "<p>no notice</p>");
  // Another session's receipt is not shown under this card either.
  const other = { ...fixture.receipt, sessionId: "another-session" };
  assert.equal(await reasonShown(other), "<p>no notice</p>");
});
