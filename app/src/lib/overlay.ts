/**
 * What counts as "give the game back" while the lobby sits over one.
 *
 * Pure, because the interesting part is not the listener but the exceptions:
 * a dialog that wants Esc for itself, a click that only looks like it landed
 * on the scrim, a key pressed while typing a message. Getting one of those
 * wrong means the lobby vanishes mid-sentence, which is worth a test rather
 * than a careful reading.
 */

/** Elements that consume Escape themselves before the overlay may have it. */
const CLAIMS_ESCAPE =
  'input, textarea, select, [contenteditable=""], [contenteditable="true"]'

/**
 * Whether this key press should hand the game back.
 *
 * Escape only, and only when nothing nearer has claimed it: a dialog that
 * called `preventDefault`, or a field someone is typing in — Escape in a
 * composer clears the draft, and losing the whole lobby instead would be a
 * nasty surprise.
 */
export function escapeLeavesOverlay(event: {
  key: string
  defaultPrevented: boolean
  target: EventTarget | null
}): boolean {
  if (event.key !== 'Escape' || event.defaultPrevented) return false
  const target = event.target
  if (target instanceof Element && target.closest(CLAIMS_ESCAPE)) return false
  return true
}

/**
 * Whether this click landed on the scrim rather than in the lobby.
 *
 * The card is `.shell`; everything outside it is the game showing through, and
 * clicking the game is how you say you want the game. Anything drawn past the
 * card's visible edge must still sit inside `.shell` in the markup for exactly
 * this reason, or a click on it would be taken for a click-away.
 */
export function clickLeavesOverlay(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false
  return target.closest('.shell') === null
}

/**
 * Whether the page at this path draws the room card.
 *
 * The card carries the game buttons in its own column, so the corner draws
 * them only where there is no card. Asked of the path rather than of the room
 * view: both room pages (`/room`, `/skirmish`) render the card, and a URL
 * needs no mount and cleanup to answer for.
 */
export function roomOnScreen(pathname: string): boolean {
  return /^\/(room|skirmish)(\/|$)/.test(pathname)
}
