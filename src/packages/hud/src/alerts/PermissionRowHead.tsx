import { useEffect, useState } from "react";
import type { Approval } from "@agent-hud/protocol";
import { HUDArtwork } from "../HUDArtwork";
import { clientDisplayName, compactDuration } from "../IslandNews";
import { projectName } from "../project-path";
import { alertText } from "../tokens";
import { useGround } from "../ground";
import { MachineLabel } from "../machines/MachineLabel";
import { permissionToolBadge } from "./permission-tool-badge";
import { rowHeadLabel, whoBesideMachine } from "./row-head-label";

export function WaitedLabel({ createdAt }: { createdAt: string }) {
  const [, setTick] = useState(0);
  useEffect(() => {
    const timer = setInterval(() => setTick((n) => n + 1), 1000);
    return () => clearInterval(timer);
  }, []);
  const start = Date.parse(createdAt);
  const seconds = Number.isFinite(start)
    ? Math.max(0, Math.floor((Date.now() - start) / 1000))
    : 0;
  const label = seconds < 60 ? `${seconds}s` : compactDuration(seconds);
  return <span>{label}</span>;
}

export function PermissionRowHead({
  request,
  open,
  light = false,
}: {
  request: Approval;
  open: boolean;
  light?: boolean;
}) {
  const { badge, Icon } = permissionToolBadge(request);
  const who =
    projectName(request.cwd) ??
    request.vendor ??
    clientDisplayName(request.source);
  const summary = request.summary ?? request.title;
  const detail = request.detail ?? request.input;
  const isQuestion = request.kind === "question";
  const isPlan = request.kind === "plan";
  const showsDetail = Boolean(
    request.removed || request.added || detail || isPlan,
  );
  const hideSummary =
    open && (isQuestion || (showsDetail && detail === summary));
  const signalText = alertText("warning", useGround(light));
  const secondary = light ? "rgba(0,0,0,0.55)" : "rgba(255,255,255,0.55)";
  const tertiary = light ? "rgba(0,0,0,0.38)" : "rgba(255,255,255,0.38)";
  const openSummaryColor = light
    ? "rgba(0,0,0,0.85)"
    : "rgba(255,255,255,0.85)";

  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 8,
        minWidth: 0,
        width: "100%",
      }}
    >
      <span
        style={{
          opacity: open ? 1 : 0.8,
          display: "inline-flex",
          flexShrink: 0,
        }}
      >
        <HUDArtwork source={request.source} size={13} light={light} />
      </span>
      <span
        style={{
          fontSize: 12,
          fontWeight: 500,
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
          // An open card shows the whole name, such as a profile's after its
          // agent's; a closed row keeps the 150 px cap.
          ...(request.machine
            ? whoBesideMachine(rowHeadLabel(open, hideSummary))
            : rowHeadLabel(open, hideSummary)),
        }}
      >
        {who}
      </span>
      {request.machine ? (
        <MachineLabel
          label={request.machine}
          ink={secondary}
          edge={light ? "rgba(0,0,0,0.16)" : "rgba(255,255,255,0.18)"}
        />
      ) : null}
      <span
        style={{
          display: "inline-flex",
          alignItems: "center",
          gap: 3,
          padding: "1px 4px",
          borderRadius: 3,
          background: "rgba(233, 161, 109, 0.14)",
          color: signalText,
          flexShrink: 0,
        }}
      >
        <Icon size={8} strokeWidth={2.5} />
        <span
          style={{
            fontSize: 10,
            fontWeight: 700,
            fontVariantNumeric: "tabular-nums",
          }}
        >
          {badge}
        </span>
      </span>
      {!hideSummary && (
        <span
          style={{
            fontSize: 11,
            color: open ? openSummaryColor : secondary,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
            flex: 1,
            minWidth: 0,
          }}
        >
          {summary}
        </span>
      )}
      {hideSummary && <span style={{ flex: 1 }} />}
      <span
        style={{
          fontSize: 11,
          fontVariantNumeric: "tabular-nums",
          color: tertiary,
          flexShrink: 0,
        }}
      >
        <WaitedLabel createdAt={request.createdAt} />
      </span>
    </div>
  );
}
