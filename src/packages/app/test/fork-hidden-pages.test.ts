// Agent HUD Fork (ticket 08, FORK.md): Settings never lists the phone
// pairing page, even when a product page asks to sit before it.
import { test } from "node:test";
import assert from "node:assert/strict";
import { translator } from "@agent-hud/ui/format";
import { sidebarPages } from "../src/settings/pages.ts";

test("the fork's Settings sidebar has no Local Network page", () => {
  const ids = sidebarPages(
    [
      {
        id: "extra",
        title: "Extra",
        before: "local-network",
        render: () => null,
      },
    ],
    translator("en"),
    "en",
  ).map((page) => page.id);
  assert.ok(!ids.includes("local-network"));
  assert.ok(ids.includes("extra"));
});

// Ticket 78: nor the Experimental page, which holds only the clipboard
// bubble and the browser extension, in English and in Chinese.
for (const language of ["en", "zh-Hans"] as const) {
  test(`the fork's Settings sidebar has no Experimental page (${language})`, () => {
    const t = translator(language);
    const pages = sidebarPages(
      [
        {
          id: "extra",
          title: "Extra",
          before: "experimental",
          render: () => null,
        },
      ],
      t,
      language,
    );
    const ids = pages.map((page) => page.id);
    assert.ok(!ids.includes("experimental"));
    assert.ok(!pages.some((page) => page.label === t("Experimental")));
    assert.ok(ids.includes("extra"));
    assert.ok(ids.includes("updates"));
  });
}
