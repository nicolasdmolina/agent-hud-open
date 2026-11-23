// Agent HUD Fork (ticket 07): why the Sound switch is off for good on a
// host that cannot play (Windows and Linux, until playback is verified
// there). Said in the app's language here, as upstream says why launch at
// login is unavailable: the host's own notice is a literal in English.
import type { Translate } from "@agent-hud/ui/format";

export function soundReason(
  host: { supported?: boolean } | undefined,
  t: Translate,
): string | undefined {
  return host?.supported === false
    ? t("Sound isn't played on this system yet.")
    : undefined;
}
