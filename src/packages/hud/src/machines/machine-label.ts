// How a far machine's label is written beside a session's client: one line,
// cut with "…" past MACHINE_LABEL_MAX characters, whole on hover.

/** The most characters a machine's label shows before it is cut. */
export const MACHINE_LABEL_MAX = 24;

/** The label on one line: every run of spaces or line breaks as one space. */
export function oneLine(label: string): string {
  return label.replace(/\s+/g, " ").trim();
}

/** The characters a person counts: an emoji or a letter with its accent is one. */
function characters(text: string): string[] {
  const segmenter = new Intl.Segmenter(undefined, { granularity: "grapheme" });
  return [...segmenter.segment(text)].map((part) => part.segment);
}

/** The label as a row shows it: whole up to {@link MACHINE_LABEL_MAX}
 * characters, else that many and "…". The row cuts it further where it has
 * less room. */
export function machineLabelText(label: string): string {
  const line = oneLine(label);
  const parts = characters(line);
  if (parts.length <= MACHINE_LABEL_MAX) return line;
  return `${parts.slice(0, MACHINE_LABEL_MAX).join("").trimEnd()}…`;
}
