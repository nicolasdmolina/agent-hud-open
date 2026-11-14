import { useState } from "react";
import type { Approval, RequestDecision } from "@agent-hud/protocol";
import { PermissionPlanCard } from "./permissions/PermissionPlanCard";
import { PermissionQuestionCard } from "./permissions/PermissionQuestionCard";
import { PermissionToolCard } from "./permissions/PermissionToolCard";
import { useGround } from "./ground";
import { alertText } from "./tokens";
import { cardColumn } from "./card-column";
import { mayDecide, outOfReach } from "./permissions/answer-gate";
import { Unplug } from "lucide-react";

export interface RequestActionsProps {
  request: Approval;
  onDecision?: (decision: RequestDecision) => void | Promise<void>;
  /** Settings, on a page such as "agent-plugins". */
  onOpenSettings?: (page: string) => void;
  light?: boolean;
  language?: string;
}

/**
 * Mirrors PermissionAlertViews.swift, PermissionPlanViews.swift, and PermissionQuestionViews.swift.
 * Preserves native capabilities and exact answer keys.
 */
export function RequestActions({
  request,
  onDecision,
  onOpenSettings,
  light = false,
  language,
}: RequestActionsProps) {
  const zh = language?.startsWith("zh") ?? false;
  const t = (en: string, cn: string) => (zh ? cn : en);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const ground = useGround(light);
  const errorColor = ground.seeThrough
    ? alertText("warning", ground)
    : "#e9a16d";

  // A card whose client is out of reach takes no answer, from any button or
  // key, and keeps what was typed until it can be answered again.
  const unreachable = outOfReach(request);
  const answer = async (decision: RequestDecision) => {
    if (!onDecision || pending || !mayDecide(request, decision)) return;
    setPending(true);
    setError(null);
    try {
      await onDecision(decision);
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : String(failure));
    } finally {
      setPending(false);
    }
  };

  const canDismiss = Boolean(onDecision) && !pending;
  const disabled = !canDismiss || Boolean(unreachable);
  const isQuestion = request.kind === "question";
  const isPlan = request.kind === "plan";

  return (
    <div
      className="hud-request-actions"
      data-request-id={request.id}
      style={{ ...cardColumn(9), fontSize: 12 }}
    >
      {unreachable && (
        <p
          role="status"
          className="hud-request-unreachable"
          style={{
            margin: 0,
            display: "flex",
            alignItems: "flex-start",
            gap: 5,
            color: errorColor,
            fontSize: 11,
            fontWeight: 500,
            lineHeight: 1.4,
            overflowWrap: "anywhere",
          }}
        >
          <Unplug
            size={11}
            strokeWidth={2.2}
            aria-hidden="true"
            style={{ flexShrink: 0, marginTop: 2 }}
          />
          {unreachable}
        </p>
      )}
      {/* Off while out of reach, and looks it; the same element either way,
          so what was typed stays. */}
      <div
        className="hud-request-controls"
        aria-disabled={unreachable ? true : undefined}
        style={{
          opacity: unreachable ? 0.45 : undefined,
          transition: "opacity 160ms ease",
        }}
      >
        {isQuestion ? (
          <PermissionQuestionCard
            request={request}
            disabled={disabled}
            canDismiss={canDismiss}
            pending={pending}
            light={light}
            zh={zh}
            answer={answer}
            openSettings={onOpenSettings}
          />
        ) : isPlan ? (
          <PermissionPlanCard
            request={request}
            disabled={disabled}
            canDismiss={canDismiss}
            light={light}
            zh={zh}
            answer={answer}
          />
        ) : (
          <PermissionToolCard
            request={request}
            disabled={disabled}
            pending={pending}
            light={light}
            zh={zh}
            answer={answer}
          />
        )}
      </div>
      {!onDecision && (
        <p style={{ margin: 0, fontSize: 11, opacity: 0.6 }}>
          {t("Approval delivery is not connected.", "尚未连接审批处理。")}
        </p>
      )}
      {error && (
        <p
          role="alert"
          style={{
            margin: 0,
            color: errorColor,
            fontSize: 11,
            lineHeight: 1.4,
          }}
        >
          {error}
        </p>
      )}
    </div>
  );
}
export default RequestActions;
