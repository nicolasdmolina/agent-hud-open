import { Fragment } from "react";
import { BadgeCheck, Check, LoaderCircle, X } from "lucide-react";
import type { Approval, RequestDecision } from "@agent-hud/protocol";
import { permissionChoices } from "./permission-choices";
import { permissionColors } from "../tokens";
import { useGround } from "../ground";
import { cardColumn } from "../card-column";
import { CardButton } from "./CardButton";

export function PermissionToolCard({
  request,
  disabled,
  pending,
  light = false,
  zh = false,
  answer,
}: {
  request: Approval;
  disabled: boolean;
  pending: boolean;
  light?: boolean;
  zh?: boolean;
  answer: (decision: RequestDecision) => void | Promise<void>;
}) {
  const t = (en: string, cn: string) => (zh ? cn : en);
  const text = (alpha: number) =>
    light ? `rgba(0,0,0,${alpha})` : `rgba(255,255,255,${alpha})`;
  const secondary = text(0.55);
  const inset = light ? "rgba(0,0,0,0.045)" : "rgba(255,255,255,0.055)";
  const controls = permissionColors(useGround(light));
  const allow = controls.allowFill;
  const allowText = controls.alwaysText;
  const allowEdge = controls.alwaysEdge;
  const onAllow = controls.onAllow;
  const denyText = controls.denyText;
  const denyFill = controls.denyFill;
  const denyStroke = controls.denyEdge;

  return (
    <div style={cardColumn(9)}>
      {request.removed || request.added ? (
        <div
          aria-label={t("Edit preview", "修改预览")}
          style={{
            ...cardColumn(2),
            padding: 9,
            borderRadius: 7,
            background: inset,
            fontSize: 11,
            fontVariantNumeric: "tabular-nums",
          }}
        >
          {request.removed &&
            request.removed
              .split("\n")
              .slice(0, 4)
              .map((line, idx) => (
                <div
                  key={`r-${idx}`}
                  style={{
                    display: "flex",
                    gap: 6,
                    color: denyText,
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                  }}
                >
                  <span>−</span>
                  <span
                    style={{
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                    }}
                  >
                    {line}
                  </span>
                </div>
              ))}
          {request.added &&
            request.added
              .split("\n")
              .slice(0, 4)
              .map((line, idx) => (
                <div
                  key={`a-${idx}`}
                  style={{
                    display: "flex",
                    gap: 6,
                    color: allowText,
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                  }}
                >
                  <span>+</span>
                  <span
                    style={{
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                    }}
                  >
                    {line}
                  </span>
                </div>
              ))}
        </div>
      ) : (
        (request.detail ?? request.input) && (
          <div
            style={{
              padding: "8px 9px",
              borderRadius: 7,
              background: inset,
              fontSize: 11,
              fontVariantNumeric: "tabular-nums",
              color: text(0.84),
              display: "-webkit-box",
              WebkitLineClamp: 6,
              WebkitBoxOrient: "vertical",
              overflow: "hidden",
              whiteSpace: "pre-wrap",
              overflowWrap: "anywhere",
            }}
          >
            {request.detail ?? request.input}
          </div>
        )
      )}
      {request.alwaysAllowRule && (
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 6,
            minWidth: 0,
          }}
        >
          <BadgeCheck
            size={9}
            strokeWidth={2.5}
            color={allowText}
            style={{ flexShrink: 0 }}
          />
          <span style={{ fontSize: 11, color: secondary, flexShrink: 0 }}>
            {t("Always allow adds", "总是允许会添加")}
          </span>
          <span
            style={{
              fontSize: 11,
              fontVariantNumeric: "tabular-nums",
              color: text(0.78),
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {request.alwaysAllowRule}
          </span>
        </div>
      )}
      <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
        {permissionChoices(request, zh).map((choice, index) => (
          <Fragment key={index}>
            <CardButton
              disabled={disabled}
              title={choice.title}
              onClick={() => void answer(choice.decision)}
              {...(choice.tone === "deny"
                ? { edge: denyStroke, fill: denyFill, color: denyText }
                : choice.tone === "lasting"
                  ? { edge: allowEdge, color: allowText }
                  : { fill: allow, color: onAllow })}
              style={{ whiteSpace: "nowrap" }}
            >
              {choice.tone === "deny" ? (
                <X size={9} strokeWidth={2.6} />
              ) : choice.tone === "lasting" ? (
                <BadgeCheck size={9} strokeWidth={2.6} />
              ) : pending ? (
                <LoaderCircle size={9} className="animate-spin" />
              ) : (
                <Check size={9} strokeWidth={2.6} />
              )}
              {choice.label}
            </CardButton>
            {choice.tone === "deny" && (
              <span style={{ flex: 1, minWidth: 8 }} />
            )}
          </Fragment>
        ))}
      </div>
    </div>
  );
}
