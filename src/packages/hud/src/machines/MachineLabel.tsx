import type { CSSProperties } from "react";
import { Server } from "lucide-react";
import { machineLabelText, oneLine } from "./machine-label";

/** Where a session runs when that is not this machine: the far machine's
 * label in a small outlined tag, its own element beside the client. It keeps
 * its room while the client's name gives way, and is cut itself only past
 * 24 characters, or where the row has no room left; the whole label shows on
 * hover. */
export function MachineLabel({
  label,
  ink,
  edge,
  size = 10,
  style,
}: {
  label: string;
  /** The tag's text and icon colour. */
  ink: string;
  /** Its outline. */
  edge: string;
  size?: number;
  style?: CSSProperties;
}) {
  return (
    <span
      className="machine-label"
      title={oneLine(label)}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 3,
        flex: "0 1 auto",
        minWidth: 0,
        // Never wider than a short name, however few its characters.
        maxWidth: "min(13em, 100%)",
        padding: "1px 4px",
        borderRadius: 4,
        boxShadow: `inset 0 0 0 1px ${edge}`,
        color: ink,
        fontSize: size,
        fontWeight: 500,
        lineHeight: `${size + 4}px`,
        whiteSpace: "nowrap",
        position: "relative",
        ...style,
      }}
    >
      <Server
        size={size - 2}
        strokeWidth={2.4}
        aria-hidden="true"
        style={{ flexShrink: 0 }}
      />
      <span
        aria-hidden="true"
        style={{ minWidth: 0, overflow: "hidden", textOverflow: "ellipsis" }}
      >
        {machineLabelText(label)}
      </span>
      {/* The whole label for a screen reader, which reads no hover title. */}
      <span
        style={{
          position: "absolute",
          width: 1,
          height: 1,
          overflow: "hidden",
          clip: "rect(0 0 0 0)",
          whiteSpace: "nowrap",
        }}
      >
        {oneLine(label)}
      </span>
    </span>
  );
}
