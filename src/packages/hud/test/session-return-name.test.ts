// A card's and a reply alert's way back, as the real components draw it: an
// app link that stands in for a session's own (Hermes desktop) is called by
// the name Core gives it, and every other way back is "Return to session",
// in both languages (ticket 65).
import { after, test } from "node:test";
import assert from "node:assert/strict";
import { createElement as h, type ComponentType, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type {
  Approval,
  IslandEvent,
  Session,
  Snapshot,
} from "@agent-hud/protocol";
import { close, inLanguage, load } from "./render.ts";

after(close);

const HERMES = { en: "Open in Hermes desktop", zh: "在 Hermes 桌面版中打开" };
const RETURN = { en: "Return to session", zh: "回到会话" };
const LANGUAGES = ["en", "zh"] as const;
const at = new Date().toISOString();

const session = (
  id: string,
  source: string,
  target: unknown,
  label?: string,
): Session => ({
  id,
  title: "Plan the trip",
  source,
  model: "m",
  cwd: null,
  updatedAt: at,
  state: "waiting",
  tokens: 0,
  cost: null,
  contextPercent: null,
  logPath: null,
  parentId: null,
  navigationTarget: target,
  ...(label ? { navigationLabel: label } : {}),
});

// A Hermes session on a far machine, by its far id, and a Cursor session
// on this one, whose own way back names no label.
const hermes = (language: "en" | "zh") =>
  session(
    "hermes:ca9b50a0779f1c57/default:20261118_205437_3c3f24",
    "hermes",
    { kind: "hermes-app" },
    HERMES[language],
  );
const cursor = () =>
  session("cursor:c1", "cursor", { kind: "cursor-chat", id: "c1" });

/** The interface language each of the two is drawn in. */
const SETTING = { en: "en", zh: "zh-Hans" } as const;
const snapshot = (language: "en" | "zh", sessions: Session[]) =>
  ({
    language: SETTING[language],
    sessions,
    settings: {},
  }) as unknown as Snapshot;

const request = (of: Session): Approval => ({
  id: `request-${of.id}`,
  title: "Which dates work?",
  summary: "Which dates work?",
  source: of.source,
  toolName: "clarify",
  input: "",
  sessionId: of.id,
  createdAt: at,
  allowsAlways: false,
  kind: "question",
});

type Provider = ComponentType<{ value: object; children?: ReactNode }>;

/** The open card, as the panel draws it from the snapshot. */
async function card(language: "en" | "zh", of: Session) {
  const { AlertStack } = await load<{
    AlertStack: ComponentType<Record<string, unknown>>;
  }>("/src/scene/AlertStack.tsx");
  const { PanelServicesProvider } = await load<{
    PanelServicesProvider: Provider;
  }>("/src/PanelContext.tsx");
  const asked = request(of);
  const news = {
    ends: { view: null, tail: [], clear() {}, drop() {} },
    queue: [],
    select() {},
    park() {},
    dismiss() {},
  };
  return renderToStaticMarkup(
    await inLanguage(
      SETTING[language],
      h(
        PanelServicesProvider,
        { value: { query: async () => null, openSession: async () => true } },
        h(AlertStack, {
          requests: [asked],
          request: asked,
          news,
          handoff() {},
          light: false,
          snapshot: snapshot(language, [of]),
          botLogos: false,
        }),
      ),
    ),
  );
}

/** A reply alert for `of`, listed in the snapshot or, when `listed` is
 * false, known only from the event Core sent. */
async function alert(language: "en" | "zh", of: Session, listed = true) {
  const { CompletionAlertView } = await load<{
    CompletionAlertView: ComponentType<Record<string, unknown>>;
  }>("/src/alerts/CompletionAlertView.tsx");
  const event: IslandEvent = {
    id: `completion-${of.id}`,
    kind: "completion",
    at,
    source: of.source,
    sessionId: of.id,
    title: "Plan the trip",
    navigationTarget: of.navigationTarget,
    ...(of.navigationLabel ? { navigationLabel: of.navigationLabel } : {}),
  };
  return renderToStaticMarkup(
    await inLanguage(
      SETTING[language],
      h(CompletionAlertView, {
        event,
        snapshot: snapshot(language, listed ? [of] : []),
        dismiss() {},
      }),
    ),
  );
}

/** The words of every button in `markup`. */
const buttons = (markup: string) =>
  [...markup.matchAll(/<button[^>]*>(.*?)<\/button>/g)].map(([, inner]) =>
    inner.replace(/<[^>]+>/g, ""),
  );

test("a Hermes card calls its way back by the name Core gives the desktop app's link", async () => {
  for (const language of LANGUAGES) {
    const shown = buttons(await card(language, hermes(language)));
    assert.ok(shown.includes(HERMES[language]), `${language}: ${shown}`);
    assert.ok(!shown.includes(RETURN[language]), `${language}: ${shown}`);
  }
});

test("another client's card keeps Return to session", async () => {
  for (const language of LANGUAGES) {
    const shown = buttons(await card(language, cursor()));
    assert.ok(shown.includes(RETURN[language]), `${language}: ${shown}`);
    assert.ok(!shown.includes(HERMES[language]), `${language}: ${shown}`);
  }
});

test("a Hermes reply alert calls its way back by that name, listed or known only from its event", async () => {
  for (const language of LANGUAGES) {
    for (const listed of [true, false]) {
      const shown = buttons(await alert(language, hermes(language), listed));
      const where = `${language}, listed ${listed}: ${shown}`;
      assert.ok(shown.includes(HERMES[language]), where);
      assert.ok(!shown.includes(RETURN[language]), where);
    }
  }
});

test("another client's reply alert keeps Return to session", async () => {
  for (const language of LANGUAGES) {
    for (const listed of [true, false]) {
      const shown = buttons(await alert(language, cursor(), listed));
      const where = `${language}, listed ${listed}: ${shown}`;
      assert.ok(shown.includes(RETURN[language]), where);
      assert.ok(!shown.includes(HERMES[language]), where);
    }
  }
});
