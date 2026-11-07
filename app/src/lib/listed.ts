/**
 * How the battle list says what a room last scored.
 *
 * The numbers are the in-room panel's challenge, read back off the service's
 * live games list (`crates/pve/src/listing.rs`), so they are only as new as
 * the last time someone in that room asked about it. The tooltip says how
 * long ago that was, because the list cannot see a room's setup change --
 * only its map, which is why a room on another map shows nothing.
 *
 * Two of them where the service has both: the room with the players it had
 * then, and the room with every seat taken, which is what a lobby that is
 * still filling is heading for -- and so the one the eye should land on.
 */

import type { ListedScore } from '../ipc/bindings/ListedScore'
import { age } from './age'
import { type ChallengeParts, challengePair, SPAN } from './challenge'

/** Whole numbers: a column of decimals is noise to scan; the tooltip keeps them. */
const whole = (challenge: number) => Math.round(challenge).toString()

/** The cell's figures, rounded: see `challengePair`. */
export function challengeParts(score: ListedScore): ChallengeParts {
	return challengePair(score.challenge, score.fullChallenge, whole)
}

/** The cell's text: `12–15`, or one figure. */
export function challengeLabel(score: ListedScore): string {
	const { now, main } = challengeParts(score)
	return now === null ? main : `${now}${SPAN}${main}`
}

/** What a hover on the figures explains: the scale, who each is for, and age. */
export function challengeTip(score: ListedScore, now: number): string {
	const players = score.players
	const seated =
		players === null
			? 'As it was scored'
			: `With the ${players} ${players === 1 ? 'player' : 'players'} seated when it was scored`
	const lines = [
		'Challenge out of 34, where 17 is an even game.',
		`${seated}: ${score.challenge.toFixed(1)}`,
	]
	if (score.fullChallenge !== null)
		lines.push(`With every seat taken: ${score.fullChallenge.toFixed(1)}`)
	const ago = age(score.scoredAt, now)
	if (ago)
		lines.push(
			`Last scored ${ago} by someone in this room; a change since then shows here only if it moved the map.`,
		)
	return lines.join('\n')
}
