// Agent HUD Fork (ticket 07, held against upstream's language system by
// ticket 66): where the host cannot play sounds, the Sound switch says so
// in the app's language. The host's own notice is a literal in English.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { addCatalog, translator } from "@agent-hud/ui/format";
import { soundReason } from "../src/settings/sound-reason.ts";

addCatalog(
  "zh-Hans",
  JSON.parse(
    readFileSync(
      join(import.meta.dirname, "../../protocol/locales/zh-Hans.json"),
      "utf8",
    ),
  ),
);
const HOST = "Not implemented: sound playback on Windows is unverified.";

test("a host that cannot play says so in the app's language", () => {
  const off = { supported: false, notice: HOST, names: [] };
  assert.equal(
    soundReason(off, translator("en")),
    "Sound isn't played on this system yet.",
  );
  assert.equal(
    soundReason(off, translator("zh-Hans")),
    "此系统暂不支持播放声音。",
  );
});

test("a host that plays, or has not said yet, gives no reason", () => {
  const on = { supported: true, notice: null, names: ["Glass"] };
  assert.equal(soundReason(on, translator("en")), undefined);
  assert.equal(soundReason(undefined, translator("en")), undefined);
});
