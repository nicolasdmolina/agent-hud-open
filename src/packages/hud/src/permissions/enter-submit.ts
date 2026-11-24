// Enter on a question card does what pressing its Submit (or Next) does.
// Only the front card takes it, only while the HUD window has the keyboard,
// once per press, and never while an input method is composing. Permission
// cards take no part: none of their buttons is a safe default, and a lasting
// choice such as Always allow swallows Enter even when focused.
import { useEffect, useRef, type RefObject } from "react";

/** A question card's offered answer. A press on one hands the HUD the
 * keyboard, and Enter on it submits rather than pressing it again. */
export const CHOICE = "[data-hud-choice]:not(:disabled)";
/** A control the browser presses or types into on Enter itself. */
export const CONTROL =
  "button, a[href], input, textarea, select, [contenteditable]:not([contenteditable=false])";

export interface EnterKey {
  key: string;
  shiftKey: boolean;
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  repeat: boolean;
  defaultPrevented: boolean;
  isComposing: boolean;
  keyCode: number;
}

/** Where a key landed: anything with `closest`, as an element has. */
export interface Spot {
  closest(selector: string): unknown;
}

export interface EnterCard {
  /** Shown and not hidden behind another surface. */
  live(): boolean;
  owns(target: Spot): boolean;
  /** Submits as the card's primary button would; false where it is off. */
  submit(): boolean;
}

/** A plain press of Enter, not a chord, a repeat, one already handled (the
 * typed answer's own Enter), or one an input method is composing with. */
function plainEnter(event: EnterKey): boolean {
  return (
    event.key === "Enter" &&
    !event.shiftKey &&
    !event.altKey &&
    !event.ctrlKey &&
    !event.metaKey &&
    !event.repeat &&
    !event.defaultPrevented &&
    !event.isComposing &&
    event.keyCode !== 229
  );
}

/** The cards on show, the last added in front. */
export function enterRegistry() {
  const cards: EnterCard[] = [];
  return {
    add(card: EnterCard) {
      cards.push(card);
      return () => {
        const at = cards.indexOf(card);
        if (at >= 0) cards.splice(at, 1);
      };
    },
    /** Whether the press submitted the front card. `focused`: the HUD
     * window has the keyboard. */
    press(event: EnterKey, target: Spot | null, focused: boolean): boolean {
      if (!focused || !plainEnter(event)) return false;
      const front = cards.findLast((card) => card.live());
      if (!front) return false;
      if (target?.closest(CHOICE)) {
        if (!front.owns(target)) return false;
      } else if (target?.closest(CONTROL)) {
        return false;
      }
      return front.submit();
    },
  };
}

/** The offered answer a press landed on, if any. */
export function pressedChoice<T extends Spot>(target: T | null): T | null {
  return (target?.closest(CHOICE) as T | null | undefined) ?? null;
}

/** A lasting choice's keydown: Enter never presses it; Space and a click
 * still do. */
export function holdEnter(event: { key: string; preventDefault(): void }) {
  if (event.key === "Enter") event.preventDefault();
}

const shown = enterRegistry();
let listening = 0;
const onKey = (event: KeyboardEvent) => {
  // Bubbling to the window, so a field's own Enter has already marked it.
  const target = event.target instanceof Element ? event.target : null;
  if (shown.press(event, target, document.hasFocus())) event.preventDefault();
};

/** Takes Enter for a question card while it is on show. Put `root` on the
 * card and set `submit` each render; the card stays where it was added. */
export function useEnterSubmit() {
  const card = useRef<{
    root: RefObject<HTMLDivElement | null>;
    submit: () => boolean;
  }>({ root: { current: null }, submit: () => false });
  useEffect(() => {
    const it = card.current;
    const remove = shown.add({
      live: () => {
        const root = it.root.current;
        return Boolean(
          root?.isConnected && !root.closest("[inert], [aria-hidden=true]"),
        );
      },
      owns: (target) =>
        Boolean(it.root.current?.contains(target as unknown as Node)),
      submit: () => it.submit(),
    });
    if (listening++ === 0) window.addEventListener("keydown", onKey);
    return () => {
      remove();
      if (--listening === 0) window.removeEventListener("keydown", onKey);
    };
  }, []);
  return card.current;
}
