/* The machines an agent's native channel follows (a client's list of
 * connections): each one's label and state as Core words them, with the
 * channel's reason, and for a connected far machine when it was last heard
 * from and when its whole history was last checked. Nothing to press. */
import { useEffect, useState } from "react";
import { Laptop, Server } from "lucide-react";
import type { Provider, Snapshot } from "@agent-hud/protocol";
import { snapshotLanguage } from "@agent-hud/ui/format";
import { Notice, Row, Section, type Text } from "../SettingsPrimitives";
import { dot, machineTimes, showsMachines } from "./machines-list";

export function MachinesSection({
  t,
  snapshot,
  source,
}: {
  t: Text;
  snapshot: Snapshot;
  source: Provider;
}) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 30_000);
    return () => window.clearInterval(timer);
  }, []);
  const list = snapshot.machines?.[source.id] ?? [];
  const note = snapshot.machinesNote?.[source.id];
  if (!showsMachines(list, note)) return null;
  const language = snapshotLanguage(snapshot);
  return (
    <Section title={t("Machines", "机器")}>
      {note && <Notice tone="warning">{note}</Notice>}
      {list.map((machine) => (
        <Row
          key={machine.id}
          icon={
            machine.here ? (
              <Laptop size={17} strokeWidth={1.8} />
            ) : (
              <Server size={17} strokeWidth={1.8} />
            )
          }
          title={machine.label}
          subtitle={machine.note}
          detail={machineTimes(machine, now, language, t)}
          muted={
            machine.state === "not-covered" || machine.state === "same-machine"
          }
        >
          <span
            className="settings-machine-state"
            style={{
              display: "inline-flex",
              alignItems: "center",
              gap: 6,
              fontSize: 12,
              color: "var(--muted-foreground)",
              whiteSpace: "nowrap",
            }}
          >
            <span
              aria-hidden="true"
              style={{
                width: 7,
                height: 7,
                borderRadius: 999,
                background: dot[machine.state],
              }}
            />
            {machine.stateText}
          </span>
        </Row>
      ))}
    </Section>
  );
}
