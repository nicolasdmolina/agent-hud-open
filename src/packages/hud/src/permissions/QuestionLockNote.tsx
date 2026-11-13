import type { QuestionLock } from "@agent-hud/protocol";
import { alertText } from "../tokens";
import { useGround } from "../ground";

/** What another viewer already did with this question. Left alone, the
 * question keeps it; once answered here, it is struck through, as the
 * answer given here replaces it. */
export function QuestionLockNote({
  lock,
  replaced,
  light,
  zh,
}: {
  lock: QuestionLock;
  replaced: boolean;
  light: boolean;
  zh: boolean;
}) {
  const t = (en: string, cn: string) => (zh ? cn : en);
  const color = alertText("warning", useGround(light));
  return (
    <div
      style={{
        fontSize: 10,
        lineHeight: 1.35,
        color,
        opacity: replaced ? 0.55 : 1,
        textDecoration: replaced ? "line-through" : undefined,
        overflowWrap: "anywhere",
      }}
    >
      {lock.state === "answered"
        ? t(
            `Answered elsewhere: ${lock.answer}`,
            `已在别处回答：${lock.answer}`,
          )
        : t("Skipped elsewhere", "已在别处跳过")}
    </div>
  );
}
