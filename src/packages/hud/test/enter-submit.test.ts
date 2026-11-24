// Enter on a question card: it does what pressing Submit (or Next) does,
// only on the card the stack shows in front, only while the HUD window has
// the keyboard, once per press, never while an input method is composing,
// and never on a permission or plan card's approving button.
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  CHOICE,
  CONTROL,
  enterRegistry,
  holdEnter,
  holdRepeat,
  holdsEnter,
  pressedChoice,
  typedEnter,
  type EnterCard,
  type EnterKey,
} from "../src/permissions/enter-submit.ts";
import { decisionGate } from "../src/permissions/answer-gate.ts";
import { permissionChoices } from "../src/permissions/permission-choices.ts";

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
 * another control such as a Back or Always allow button. `request` names a
 * request card as the stack draws it. */
type Spot = {
  card?: string;
  choice?: boolean;
  control?: boolean;
  request?: string;
};
const at = (spot: Spot) => ({
  spot,
  closest(selector: string) {
    if (selector === CHOICE) return spot.choice ? this : null;
    if (selector === CONTROL) return spot.choice || spot.control ? this : null;
    return null;
  },
});
const page = at({});
/** The request card the stack shows in front. */
const shown = (request: string) => at({ request });

/** A question card: `answered` says whether Submit is on, `sent` counts the
 * submits it made. */
function card(id: string, answered = true) {
  const made = { sent: 0, live: true };
  const value: EnterCard = {
    live: () => made.live,
    within: (request) => (request as ReturnType<typeof at>).spot.request === id,
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
  assert.equal(keys.press(enter(), page, true, shown("one")), true);
  assert.equal(one.made.sent, 1);
  // From the answer just chosen too, which then is not pressed again.
  assert.equal(
    keys.press(enter(), at({ card: "one", choice: true }), true, shown("one")),
    true,
  );
  assert.equal(one.made.sent, 2);
});

test("Enter with nothing chosen does nothing", () => {
  const keys = enterRegistry();
  const one = card("one", false);
  keys.add(one.value);
  assert.equal(keys.press(enter(), page, true, shown("one")), false);
  assert.equal(one.made.sent, 0);
  // On an answer reached with Tab, the press is not taken, so the browser
  // presses that answer: it is chosen, Submit comes on, and nothing is sent
  // until the next Enter.
  assert.equal(
    keys.press(enter(), at({ card: "one", choice: true }), true, shown("one")),
    false,
  );
  assert.equal(one.made.sent, 0);
});

test("Enter during IME composition does nothing", () => {
  const keys = enterRegistry();
  const one = card("one");
  keys.add(one.value);
  const front = shown("one");
  assert.equal(
    keys.press(enter({ isComposing: true }), page, true, front),
    false,
  );
  // WebKit reports a composing key as 229 without isComposing.
  assert.equal(keys.press(enter({ keyCode: 229 }), page, true, front), false);
  assert.equal(one.made.sent, 0);
  // The typed answer's own Enter too.
  assert.equal(typedEnter(enter({ isComposing: true })), false);
  assert.equal(typedEnter(enter({ keyCode: 229 })), false);
  assert.equal(typedEnter(enter()), true);
});

test("Enter never triggers Always allow", () => {
  const keys = enterRegistry();
  // A permission card takes no part: a focused Always allow is a control.
  const lasting = at({ card: "perm", control: true });
  assert.equal(keys.press(enter(), lasting, true, shown("perm")), false);
  // And the button itself swallows Enter, so the browser does not press it.
  let held = 0;
  holdEnter({ key: "Enter", preventDefault: () => held++ });
  assert.equal(held, 1);
  holdEnter({ key: " ", preventDefault: () => held++ });
  assert.equal(held, 1, "Space still presses it");
});

test("Enter presses no approving button, only Deny", () => {
  const card = {
    allowsAlways: true,
    suggestions: [{ label: "Allow for session", behavior: "allow" }],
  };
  const lasting = { allowsAlways: true };
  for (const request of [card, lasting, { allowsAlways: false }])
    for (const choice of permissionChoices(request))
      assert.equal(
        holdsEnter(choice.tone),
        choice.tone !== "deny",
        `${choice.label} (${choice.tone})`,
      );
});

test("with two cards only the front one submits", () => {
  const keys = enterRegistry();
  const back = card("back");
  const front = card("front");
  keys.add(back.value);
  keys.add(front.value);
  assert.equal(keys.press(enter(), page, true, shown("front")), true);
  assert.deepEqual([back.made.sent, front.made.sent], [0, 1]);
  // An answer chosen on the card behind does not submit the front one.
  assert.equal(
    keys.press(
      enter(),
      at({ card: "back", choice: true }),
      true,
      shown("front"),
    ),
    false,
  );
  assert.deepEqual([back.made.sent, front.made.sent], [0, 1]);
  // The stack puts the other card in front: Enter follows it, not the order
  // the cards came in.
  assert.equal(keys.press(enter(), page, true, shown("back")), true);
  assert.deepEqual([back.made.sent, front.made.sent], [1, 1]);
  // A permission card in front: no question behind it is answered.
  assert.equal(keys.press(enter(), page, true, shown("perm")), false);
  assert.equal(keys.press(enter(), page, true, null), false);
  // A card hidden (inert) takes nothing.
  front.made.live = false;
  assert.equal(keys.press(enter(), page, true, shown("front")), false);
  assert.deepEqual([back.made.sent, front.made.sent], [1, 1]);
});

test("Enter acts only while the HUD window has the keyboard", () => {
  const keys = enterRegistry();
  const one = card("one");
  keys.add(one.value);
  assert.equal(keys.press(enter(), page, false, shown("one")), false);
  assert.equal(one.made.sent, 0);
});

test("one press submits once", () => {
  const keys = enterRegistry();
  const one = card("one");
  keys.add(one.value);
  const front = shown("one");
  // Already handled, as the typed answer's own Enter marks it.
  assert.equal(
    keys.press(enter({ defaultPrevented: true }), page, true, front),
    false,
  );
  // A held key's repeats: on the page, in the typed answer, on Submit.
  assert.equal(keys.press(enter({ repeat: true }), page, true, front), false);
  assert.equal(typedEnter(enter({ repeat: true })), false);
  let held = 0;
  holdRepeat({ key: "Enter", repeat: true, preventDefault: () => held++ });
  assert.equal(held, 1);
  holdRepeat({ key: "Enter", repeat: false, preventDefault: () => held++ });
  assert.equal(held, 1, "the first press still presses Submit");
  assert.equal(one.made.sent, 0);
});

test("a card sends one decision while one is on its way or after it went", () => {
  const gate = decisionGate();
  const answer = { kind: "answer", answers: {} } as const;
  assert.equal(gate.begin(answer), true);
  assert.equal(gate.begin(answer), false, "on its way");
  gate.end(true);
  assert.equal(gate.begin(answer), false, "already sent");
  assert.equal(gate.begin({ kind: "allow" }), false, "already sent");
  // Leaving the card alone sends nothing to the client and still may.
  assert.equal(gate.begin({ kind: "leave" }), true);

  const failed = decisionGate();
  assert.equal(failed.begin(answer), true);
  failed.end(false);
  // Delivery failed: the person may act again.
  assert.equal(failed.begin(answer), true);
});

test("Enter leaves other keys, chords and focused buttons alone", () => {
  const keys = enterRegistry();
  const one = card("one");
  keys.add(one.value);
  const front = shown("one");
  for (const event of [
    enter({ shiftKey: true }),
    enter({ altKey: true }),
    enter({ ctrlKey: true }),
    enter({ metaKey: true }),
    enter({ key: " " }),
    enter({ key: "Escape" }),
  ]) {
    assert.equal(
      keys.press(event, page, true, front),
      false,
      JSON.stringify(event),
    );
    if (event.key !== "Enter" || event.shiftKey)
      assert.equal(typedEnter(event), false, JSON.stringify(event));
  }
  // Back, Skip or Submit itself is pressed by the browser, not twice by us.
  assert.equal(
    keys.press(enter(), at({ card: "one", control: true }), true, front),
    false,
  );
  assert.equal(one.made.sent, 0);
});

test("a card that leaves takes no more presses", () => {
  const keys = enterRegistry();
  const one = card("one");
  const remove = keys.add(one.value);
  remove();
  assert.equal(keys.press(enter(), page, true, shown("one")), false);
  assert.equal(one.made.sent, 0);
});

test("a press on an offered answer is what hands the HUD the keyboard", () => {
  const choice = at({ card: "one", choice: true });
  assert.equal(pressedChoice(choice), choice);
  assert.equal(pressedChoice(at({ control: true })), null);
  assert.equal(pressedChoice(page), null);
  assert.equal(pressedChoice(null), null);
});
