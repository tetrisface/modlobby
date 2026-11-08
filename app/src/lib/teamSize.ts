import type { Score } from '../ipc/bindings/Score'
import type { SizeScore } from '../ipc/bindings/SizeScore'

/**
 * A room's answer at another head count.
 *
 * The service scores the room at every team size up to its seat count in one
 * answer (`Score.sizes`), each size the answer a room of that size would get
 * with its players at the seated average handicap. So a join, a leave or a
 * player taking a seat can be shown at once, before the room is asked again:
 * as long as nothing else about the setup moved and the seated players still
 * average to the handicap the table assumed.
 */

/** Who sits in the room, as far as the per-size table cares. */
export type Seats = {
	/** Human players seated, spectators not counted. */
	players: number
	/** Their Player Handicap ({@link playerHandicap}); `null` with nobody seated. */
	handicap: number | null
}

/**
 * The service's Player Handicap for these seated players' handicaps, in
 * percent: the average of `(multiplier - 1) * 100` to one decimal, the same
 * steps in the same order as `crates/pve`'s `player_handicap`, so the two agree
 * to the bit.
 */
export function playerHandicap(handicaps: readonly number[]): number | null {
	if (handicaps.length === 0) return null
	const multipliers = handicaps.map((percent) => 1 + percent / 100)
	const average =
		multipliers.reduce((sum, value) => sum + (value - 1) * 100, 0) /
		multipliers.length
	return Math.round(average * 10) / 10
}

/**
 * `score` as it reads for `now`, or `null` when its table cannot say: another
 * handicap, a size it holds no entry for, an answer without a table.
 *
 * The room as asked is the answer itself. Another size gets that size's
 * challenge, win chance and "Harder than"; everything else in the answer — the
 * full-room figure, the histogram, the players' records — is the room's and
 * stays as it was until the room is asked again.
 */
export function reseated(score: Score, now: Seats): Score | null {
	if (score.handicap !== now.handicap) return null
	if (score.seated === now.players) return score
	if (score.handicap === null) return null
	const entry = score.sizes.find((size) => size.players === now.players)
	if (!entry) return null
	return {
		...score,
		challenge: entry.challenge,
		winChance: entry.winChance,
		percentile: entry.percentile,
		seated: entry.players,
	}
}

/**
 * The table's entry for the room as `score` reads it, which carries whether
 * the model saw enough games at that size to stand behind it.
 */
export function seatedEntry(score: Score): SizeScore | null {
	return score.sizes.find((size) => size.players === score.seated) ?? null
}

/**
 * Which of the room's figures are projections past the model's data: too few
 * games were ever played at that size to measure it.
 */
export function uncertainSizes(score: Score): number[] {
	const sizes = [seatedEntry(score), score.full]
		.filter((entry): entry is SizeScore => entry?.uncertain === true)
		.map((entry) => entry.players)
	return [...new Set(sizes)]
}
