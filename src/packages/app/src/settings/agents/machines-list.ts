// What the machines list shows of each machine, apart from how it is drawn.
import type { MachineStatus } from "@agent-hud/protocol";
import { age, type Language } from "@agent-hud/ui/format";

type Text = (en: string, zh: string) => string;

/** Whether the list is worth a section: not when it holds only this machine
 * (or nothing) and nothing is wrong with the list itself. */
export function showsMachines(
  list: readonly Pick<MachineStatus, "here">[],
  note: string | undefined,
): boolean {
  return Boolean(note) || list.some((machine) => !machine.here);
}

/** "Last contact 2m ago · History checked 1h ago": only the times there are,
 * and only for a far machine that is connected. */
export function machineTimes(
  machine: MachineStatus,
  now: number,
  language: Language,
  t: Text,
): string | null {
  if (machine.state !== "connected" || machine.here) return null;
  const since = (at: string | undefined) => {
    const instant = at ? Date.parse(at) : Number.NaN;
    return Number.isFinite(instant) && instant > 0
      ? age((now - instant) / 1000, language)
      : null;
  };
  const contact = since(machine.lastContactAt);
  const history = since(machine.historyCheckedAt);
  const parts = [
    contact && t(`Last contact ${contact}`, `最近联系 ${contact}`),
    history && t(`History checked ${history}`, `历史检查于 ${history}`),
  ].filter(Boolean);
  return parts.length ? parts.join(" · ") : null;
}
