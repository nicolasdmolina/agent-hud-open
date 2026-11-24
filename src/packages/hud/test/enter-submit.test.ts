// Enter on a question card: it does what pressing Submit (or Next) does,
// only on the front card, only while the HUD window has the keyboard, once
// per press, never while an input method is composing, and never on a
// permission card's lasting choice such as Always allow.
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  CHOICE,
  CONTROL,
  enterRegistry,
  holdEnter,
  pressedChoice,
  type EnterCard,
  type EnterKey,
} from "../src/permissions/enter-submit.ts";

const enter = (patch: Partial<EnterKey> = {}): EnterKey => ({
  key: "Enter",
  shiftKey: false,
  altKey: false,
  ctrlKey: false,
  metaKey: false,
  repeat: false,
  defaultPrevented: false,
  isComposing: false,
  keyCode: 13,
  ...patch,
});

/** Something that holds focus: the page itself, a card's offered answer, or
 * another control such as a Back or Always allow button. */
type Spot = { card?: string; choice?: boolean; control?: boolean };
const at = (spot: Spot) => ({
  spot,
  closest(selector: string) {
    if (selector === CHOICE) return spot.choice ? this : null;
    if (selector === CONTROL) return spot.choice || spot.control ? this : null;
    return null;
  },
});
const page = at({});

/** A question card: `answered` says whether Submit is on, `sent` counts the
 * submits it made. */
function card(id: string, answered = true) {
  const made = { sent: 0, live: true };
  const value: EnterCard = {
    live: () => made.live,
    owns: (target) => (target as ReturnType<typeof at>).spot.card === id,
    submit: () => {
      if (!answered) return false;
      made.sent++;
      return true;
    },
  };
  return { made, value };
}

test("Enter submits a chosen answer", () => {
  const keys = enterRegistry();
  const one = card("one");
  keys.add(one.value);
  assert.equal(keys.press(enter(), page, true), true);
  assert.equal(one.made.sent, 1);
  // From the answer just chosen too, which then is not pressed again.
  assert.equal(
    keys.press(enter(), at({ card: "one", choice: true }), true),
    true,
  );
  assert.equal(one.made.sent, 2);
});

test("Enter with nothing chosen does nothing", () => {
  const keys = enterRegistry();
  const one = card("one", false);
  keys.add(one.value);
  assert.equal(keys.press(enter(), page, true), false);
  assert.equal(one.made.sent, 0);
});

test("Enter during IME composition does nothing", () => {
  const keys = enterRegistry();
  const one = card("one");
  keys.add(one.value);
  assert.equal(keys.press(enter({ isComposing: true }), page, true), false);
  // WebKit reports a composing key as 229 without isComposing.
  assert.equal(keys.press(enter({ keyCode: 229 }), page, true), false);
  assert.equal(one.made.sent, 0);
});

test("Enter never triggers Always allow", () => {
  const keys = enterRegistry();
  // A permission card takes no part: a focused Always allow is a control.
  const lasting = at({ card: "perm", control: true });
  assert.equal(keys.press(enter(), lasting, true), false);
  // And the button itself swallows Enter, so the browser does not press it.
  let held = 0;
  holdEnter({ key: "Enter", preventDefault: () => held++ });
  assert.equal(held, 1);
  holdEnter({ key: " ", preventDefault: () => held++ });
  assert.equal(held, 1, "Space still presses it");
  // A question card behind it is not answered either.
  const one = card("one");
  keys.add(one.value);
  assert.equal(keys.press(enter(), lasting, true), false);
  assert.equal(one.made.sent, 0);
});

test("with two cards only the front one submits", () => {
  const keys = enterRegistry();
  const back = card("back");
  const front = card("front");
  keys.add(back.value);
  keys.add(front.value);
  assert.equal(keys.press(enter(), page, true), true);
  assert.deepEqual([back.made.sent, front.made.sent], [0, 1]);
  // An answer chosen on the card behind does not submit the front one.
  assert.equal(
    keys.press(enter(), at({ card: "back", choice: true }), true),
    false,
  );
  assert.deepEqual([back.made.sent, front.made.sent], [0, 1]);
  // A front card hidden (inert) or gone hands Enter to the one behind.
  front.made.live = false;
  assert.equal(keys.press(enter(), page, true), true);
  assert.deepEqual([back.made.sent, front.made.sent], [1, 1]);
});

test("Enter acts only while the HUD window has the keyboard", () => {
  const keys = enterRegistry();
  const one = card("one");
  keys.add(one.value);
  assert.equal(keys.press(enter(), page, false), false);
  assert.equal(one.made.sent, 0);
});

test("one press submits once", () => {
  const keys = enterRegistry();
  const one = card("one");
  keys.add(one.value);
  // Already handled, as the typed answer's own Enter marks it.
  assert.equal(
    keys.press(enter({ defaultPrevented: true }), page, true),
    false,
  );
  // A held key's repeats.
  assert.equal(keys.press(enter({ repeat: true }), page, true), false);
  assert.equal(one.made.sent, 0);
});

test("Enter leaves other keys, chords and focused buttons alone", () => {
  const keys = enterRegistry();
  const one = card("one");
  keys.add(one.value);
  for (const event of [
    enter({ shiftKey: true }),
    enter({ altKey: true }),
    enter({ ctrlKey: true }),
    enter({ metaKey: true }),
    enter({ key: " " }),
    enter({ key: "Escape" }),
  ])
    assert.equal(keys.press(event, page, true), false, JSON.stringify(event));
  // Back, Skip or Submit itself is pressed by the browser, not twice by us.
  assert.equal(
    keys.press(enter(), at({ card: "one", control: true }), true),
    false,
  );
  assert.equal(one.made.sent, 0);
});

test("a card that leaves takes no more presses", () => {
  const keys = enterRegistry();
  const one = card("one");
  const remove = keys.add(one.value);
  remove();
  assert.equal(keys.press(enter(), page, true), false);
  assert.equal(one.made.sent, 0);
});

test("a press on an offered answer is what hands the HUD the keyboard", () => {
  const choice = at({ card: "one", choice: true });
  assert.equal(pressedChoice(choice), choice);
  assert.equal(pressedChoice(at({ control: true })), null);
  assert.equal(pressedChoice(page), null);
  assert.equal(pressedChoice(null), null);
});
