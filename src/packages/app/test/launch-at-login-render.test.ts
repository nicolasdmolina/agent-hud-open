// Agent HUD Fork (ticket 43, held against upstream's language system by
// ticket 66): Settings → General shows what macOS said about launch at
// login, as the host words it, not a stock line. Drawn with the real pane.
import { after, test } from "node:test";
import assert from "node:assert/strict";
import { createElement as h, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { Snapshot } from "@agent-hud/protocol";
import { close, load } from "./render.ts";

after(close);

const STOCK = "This build can’t find the login service.";
const STOCK_PLAIN = "This build can't find the login service.";
const HINT = "Allow it in System Settings → General → Login Items.";

async function pane(launchAtLogin: Record<string, unknown>) {
  const { GeneralPane } = await load<{
    GeneralPane: ComponentType<Record<string, unknown>>;
  }>("/src/settings/GeneralPane.tsx");
  const snapshot = {
    language: "en",
    settings: { language: "en", appearance: "system", launchAtLogin: true },
    connection: { platform: "macos", system: { launchAtLogin } },
  } as unknown as Snapshot;
  return renderToStaticMarkup(
    h(GeneralPane, {
      snapshot,
      config: {},
      update() {},
      action: async () => null,
    }),
  )
    .replace(/&#x27;|&#39;/g, "'")
    .replace(/&amp;/g, "&");
}

test("a registration macOS refused shows macOS's own reason", async () => {
  // The notice of the host's test `a_failed_registration_from_not_found_keeps_its_warning`.
  const notice =
    "macOS could not register the login item: Operation not permitted";
  const markup = await pane({
    supported: true,
    enabled: false,
    status: "unavailable",
    notice,
  });
  assert.ok(markup.includes(notice), "the reason is not shown");
  assert.ok(!markup.includes(STOCK) && !markup.includes(STOCK_PLAIN));
});

test("an item that waits for approval names the app to approve", async () => {
  const notice =
    "Approve Agent HUD Fork in System Settings → General → Login Items.";
  const markup = await pane({
    supported: true,
    enabled: false,
    status: "requires-approval",
    notice,
  });
  assert.ok(markup.includes(notice), "the named hint is not shown");
  assert.ok(!markup.includes(HINT));
});

test("with no words from the host the stock hint stands, and plain states show nothing", async () => {
  const waiting = await pane({
    supported: true,
    enabled: false,
    status: "requires-approval",
    notice: null,
  });
  assert.ok(waiting.includes(HINT));
  for (const status of ["enabled", "disabled", "unavailable"]) {
    const markup = await pane({
      supported: true,
      enabled: status === "enabled",
      status,
      notice: null,
    });
    assert.ok(!markup.includes("Login Items"), status);
    assert.ok(!markup.includes("login service"), status);
  }
});
