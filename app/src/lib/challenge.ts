/**
 * A room's two challenges, side by side: as it is seated, and with every seat
 * taken. The battle list and the room's own panel both write them `now–full`,
 * the full team last and in full colour, since that is what a lobby still
 * filling is heading for; each picks its own precision.
 */

/** Between the two figures: an en dash, unspaced, as between any two numbers. */
export const SPAN = '–'

/** A challenge to the decimal, as the room writes it. */
export const tenths = (value: number) => value.toFixed(1)
/** A chance, 0-1, as a whole percent. */
export const chance = (value: number) => `${Math.round(value * 100)}%`
/** A percentile, 0-100, as a whole percent: "Harder than". */
export const percentRank = (value: number) => `${Math.round(value)}%`

/**
 * The figures as written: `main`, and `now` before it when the room is not
 * full yet and the two read differently.
 *
 * `main` is the full-team figure whenever there is one; a room scored without
 * a seat count has only the figure it was scored at.
 */
export type ChallengeParts = { now: string | null; main: string }

export function challengePair(
	now: number,
	full: number | null,
	format: (challenge: number) => string,
): ChallengeParts {
	const seated = format(now)
	if (full === null) return { now: null, main: seated }
	const filled = format(full)
	return { now: filled === seated ? null : seated, main: filled }
}
