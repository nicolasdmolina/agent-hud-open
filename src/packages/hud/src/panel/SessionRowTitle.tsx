import type { CSSProperties } from "react";

/** One line of text that shortens with an ellipsis where it does not fit. */
export const clipped: CSSProperties = {
  minWidth: 0,
  whiteSpace: "nowrap",
  overflow: "hidden",
  textOverflow: "ellipsis",
};

/** A row's title and folder, shortening where the row is narrow so its tokens
 * stay whole: the folder first, as it takes only the room the title leaves,
 * while a few letters of it fit, and below that it leaves (a first item as tall
 * as the line holds it, so the folder wraps onto a hidden second line instead
 * of a stub); then the title. */
export function RowTitle({
  parts: [task, folder],
  untitled,
}: {
  parts: [string, string | null];
  untitled: string;
}) {
  return (
    <span style={{ display: "flex", flex: 1, minWidth: 0 }}>
      <span style={clipped}>{task || untitled}</span>
      {folder && (
        <span
          style={{
            display: "flex",
            flexWrap: "wrap",
            flex: "1 1 0",
            height: 15,
            minWidth: 0,
            overflow: "hidden",
          }}
        >
          <span style={{ height: 15 }} />
          <span
            style={{
              ...clipped,
              flex: "1 1 40px",
              minWidth: 40,
              maxWidth: "max-content",
              whiteSpace: "pre",
            }}
          >
            {` · ${folder}`}
          </span>
        </span>
      )}
    </span>
  );
}

/** The row's title (Open's HoverPanelView.sessionTitle): the task cut short,
 * then the session's folder, so rows of one task in different folders differ. */
export function sessionTitleParts(
  task: string,
  cwd: string | null,
): [string, string | null] {
  const terminal = cwd?.split(/[\\/]/).filter(Boolean).pop();
  if (!terminal || !task) return [task, null];
  const words = task.split(" ").filter(Boolean);
  const short =
    words.length > 3
      ? words.slice(0, 3).join(" ")
      : [...task].slice(0, 14).join("");
  return [short, terminal];
}
