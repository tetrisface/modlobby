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
 * Escape only, pressed afresh, and only when nothing nearer has claimed it: a
 * dialog that called `preventDefault`, or a field someone is typing in —
 * Escape in a composer clears the draft, and losing the whole lobby instead
 * would be a nasty surprise.
 *
 * Not a repeat: the in-game Escape raises the lobby and focuses it mid-press,
 * so a key held past the keyboard's repeat delay sends its repeats here, and
 * taking one would toggle the game straight back.
 */
export function escapeLeavesOverlay(event: {
	key: string
	defaultPrevented: boolean
	repeat: boolean
	target: EventTarget | null
}): boolean {
	if (event.key !== 'Escape' || event.defaultPrevented || event.repeat) {
		return false
	}
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
 * A click-away, from its press to its release.
 *
 * Both ends have to be on the scrim: a text selection dragged out of the card
 * never pressed it, and a press that wanders back into the card changed its
 * mind.
 *
 * Acted on at the release, because a game given focus mid-press gets a release
 * it never saw the press of, and SDL (the engine's input library) spends the
 * player's next click on the game's Lobby button correcting for it.
 */
export function clickAway(leave: () => void) {
	let pressedAway = false
	return {
		press(event: { target: EventTarget | null }) {
			pressedAway = clickLeavesOverlay(event.target)
		},
		release(event: { target: EventTarget | null }) {
			if (pressedAway && clickLeavesOverlay(event.target)) leave()
			pressedAway = false
		},
	}
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
