/* General's Sound section: the switch for Core's sound alerts and the system
 * sound each kind plays. Choosing a sound plays it once. */
import { invoke, isTauri } from "@tauri-apps/api/core";
import type { ReactNode } from "react";
import type { Snapshot } from "@agent-hud/protocol";
import type { Translate } from "@agent-hud/ui/format";
import { Menu, ToggleRow } from "./SettingsControls";
import { Row, Section } from "./SettingsPrimitives";

export const soundKeys = ["sound", "soundNeedsYou", "soundFinished"];

export function SoundSection({
  snapshot,
  t,
  save,
  problem,
}: {
  snapshot: Snapshot;
  t: Translate;
  /** Answers whether the setting was saved. */
  save: (key: string, value: unknown) => Promise<boolean>;
  problem: ReactNode;
}) {
  const settings = snapshot.settings;
  const host = snapshot.connection.system?.sound;
  const on = settings.sound ?? false;
  const choose = (key: string) => async (name: string) => {
    if ((await save(key, name)) && isTauri())
      await invoke("native_play_sound", { name }).catch((error) =>
        console.warn("The sound could not be previewed", error),
      );
  };
  const picker = (title: string, key: string, value: string) => (
    <Row title={title}>
      <Menu
        label={title}
        value={value}
        // The saved choice stays listed even when the host does not offer it.
        options={[...new Set([...(host?.names ?? []), value])]
          .sort()
          .map((name) => [name, name])}
        disabled={!on || host?.supported === false}
        change={choose(key)}
      />
    </Row>
  );

  return (
    <Section title={t("Sound")}>
      <ToggleRow
        title={t("Play sounds")}
        checked={on}
        disabled={host?.supported === false}
        reason={host?.notice ?? undefined}
        change={(value) => void save("sound", value)}
      />
      {picker(
        t("Needs you [sound]"),
        "soundNeedsYou",
        settings.soundNeedsYou ?? "Glass",
      )}
      {picker(
        t("Finished [sound]"),
        "soundFinished",
        settings.soundFinished ?? "Hero",
      )}
      {problem}
    </Section>
  );
}
