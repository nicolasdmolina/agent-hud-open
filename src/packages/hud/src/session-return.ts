/** What a card's way back to its session is called: the name Core gives an
 * app link that stands in for the session's own ("Open in Hermes desktop",
 * already in the app's language), else "Return to session". */
export function returnName(label: string | undefined, zh: boolean): string {
  return label || (zh ? "回到会话" : "Return to session");
}
