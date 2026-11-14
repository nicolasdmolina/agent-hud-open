import { useState } from "react";
import type { Approval } from "@agent-hud/protocol";
import type { IslandCard } from "../IslandNews";
import { alertText } from "../tokens";
import { useGround } from "../ground";
import { cardColumn } from "../card-column";
import { CompletionClosedRow } from "./CompletionClosedRow";
import { PermissionRowHead } from "./PermissionRowHead";

export { PermissionRowHead, WaitedLabel } from "./PermissionRowHead";

export function PermissionClosedRow({
  request,
  onSelect,
  light = false,
}: {
  request: Approval;
  onSelect: () => void;
  light?: boolean;
}) {
  const [hover, setHover] = useState(false);
  const segmentBg = light ? "rgba(0,0,0,0.05)" : "rgba(255,255,255,0.06)";
  return (
    <button
      type="button"
      className="hud-closed-request"
      onClick={onSelect}
      onPointerEnter={() => setHover(true)}
      onPointerLeave={() => setHover(false)}
      style={{
        width: "100%",
        display: "flex",
        alignItems: "center",
        padding: "9px 10px",
        borderRadius: 8,
        border: 0,
        background: hover ? segmentBg : "transparent",
        color: "inherit",
        textAlign: "left",
        cursor: "pointer",
      }}
    >
      <PermissionRowHead request={request} open={false} light={light} />
    </button>
  );
}

/** "Waiting" over the requests that wait, with how many. */
export function WaitingHeader({
  count,
  padding,
  light,
  zh,
}: {
  count: number;
  padding: string;
  light: boolean;
  zh: boolean;
}) {
  const signalText = alertText("warning", useGround(light));
  const secondary = light ? "rgba(0,0,0,0.55)" : "rgba(255,255,255,0.55)";
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        padding,
      }}
    >
      <span style={{ fontSize: 11, fontWeight: 600, color: secondary }}>
        {zh ? "待审批" : "Waiting"}
      </span>
      <span
        style={{
          fontSize: 10,
          fontWeight: 600,
          fontVariantNumeric: "tabular-nums",
          color: signalText,
          padding: "2px 6px",
          borderRadius: 999,
          background: "rgba(233, 161, 109, 0.16)",
        }}
      >
        {count}
      </span>
    </div>
  );
}

export function PendingEventRows({
  queue,
  currentID,
  requests,
  select,
  includeRequests = false,
  light = false,
  zh = false,
}: {
  queue: IslandCard[];
  currentID: string;
  requests: Approval[];
  select: (id: string) => void;
  includeRequests?: boolean;
  light?: boolean;
  zh?: boolean;
}) {
  const waitingRequests = includeRequests
    ? requests.filter((req) => req.id !== currentID)
    : [];
  const otherCompletions = queue.filter(
    (card): card is Extract<IslandCard, { type: "news" }> =>
      card.type === "news" &&
      card.id !== currentID &&
      card.event.kind === "completion",
  );
  if (waitingRequests.length === 0 && otherCompletions.length === 0)
    return null;

  return (
    <div style={{ ...cardColumn(6), marginTop: 10 }}>
      {waitingRequests.length > 0 && (
        <div style={cardColumn(2)}>
          <WaitingHeader
            count={waitingRequests.length}
            padding="4px 4px 6px"
            light={light}
            zh={zh}
          />
          {waitingRequests.map((req) => (
            <PermissionClosedRow
              key={req.id}
              request={req}
              onSelect={() => select(req.id)}
              light={light}
            />
          ))}
        </div>
      )}
      {otherCompletions.length > 0 && (
        <div style={cardColumn(2)}>
          {otherCompletions.map((card) => (
            <CompletionClosedRow
              key={card.id}
              event={card.event}
              onSelect={() => select(card.id)}
              light={light}
              zh={zh}
            />
          ))}
        </div>
      )}
    </div>
  );
}
