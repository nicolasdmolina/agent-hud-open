import type { CSSProperties } from "react";

/** How much of a request row's width "who" may take on an open card that also shows a summary. */
export const OPEN_LABEL_SHARE = "58%";

/** How wide "who" (the project, or the agent with its profile) may be in a request's row head.
 *
 * A closed queue row keeps Open's cap of 150 px. An open card names the request it asks about, so its label is the
 * one that must read whole: with no summary beside it (an open question), the label takes the row's room; with a
 * summary, the label keeps its full width up to {@link OPEN_LABEL_SHARE} of the row and the summary gives way first,
 * cut with "…". A label longer than that is cut itself, so the summary, the badge and the time stay in the row. */
export function rowHeadLabel(
  open: boolean,
  hideSummary: boolean,
): CSSProperties {
  if (!open) return { flexShrink: 0, maxWidth: 150 };
  if (hideSummary) return { flexShrink: 1, minWidth: 0 };
  return { flexShrink: 0, maxWidth: OPEN_LABEL_SHARE };
}
