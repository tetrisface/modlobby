/**
 * How the battle list says what a room last scored.
 *
 * The numbers are the in-room panel's challenge, read back off the service's
 * live games list (`crates/pve/src/listing.rs`), so they are only as new as
 * the last time someone in that room asked about it. The tooltip says how
 * long ago that was, because the list cannot see a room's setup change --
 * only its map, which is why a room on another map shows nothing.
 *
 * Two of them where the service has both: the room with the players it has,
 * and the room with every seat taken, which is what a lobby that is still
 * filling is heading for -- and so the one the eye should land on. Where the
 * service listed the room's figure at every team size, "the players it has"
 * are the ones seated now, which the list knows without anybody asking.
 */

import type { ListedScore } from '../ipc/bindings/ListedScore'
import { age } from './age'
import { type ChallengeParts, challengePair, SPAN } from './challenge'

/** Whole numbers: a column of decimals is noise to scan; the tooltip keeps them. */
const whole = (challenge: number) => Math.round(challenge).toString()

/** The room's figure as seated, and for how many. */
type Seated = {
	/** `null` with nobody seated, or a head count the table does not reach. */
	challenge: number | null
	players: number | null
	/** Read off the table at the room's head count now. */
	now: boolean
}

/**
 * The room as it is seated: the table's entry at `players`, the room's head
 * count now, when the service listed a table; otherwise as it was scored.
 */
function seated(score: ListedScore, players: number): Seated {
	if (Object.keys(score.sizes).length === 0)
		return { challenge: score.challenge, players: score.players, now: false }
	return { challenge: score.sizes[players] ?? null, players, now: true }
}

/** The cell's figures, rounded: see `challengePair`. */
export function challengeParts(
	score: ListedScore,
	players: number,
): ChallengeParts {
	const { challenge } = seated(score, players)
	if (challenge === null)
		return { now: null, main: whole(score.fullChallenge ?? score.challenge) }
	return challengePair(challenge, score.fullChallenge, whole)
}

/** The cell's text: `12–15`, or one figure. */
export function challengeLabel(score: ListedScore, players: number): string {
	const { now, main } = challengeParts(score, players)
	return now === null ? main : `${now}${SPAN}${main}`
}

const playersText = (count: number) =>
	`${count} ${count === 1 ? 'player' : 'players'}`

/** What a hover on the figures explains: the scale, who each is for, and age. */
export function challengeTip(
	score: ListedScore,
	players: number,
	now: number,
): string {
	const held = seated(score, players)
	const lines = ['Challenge out of 34, where 17 is an even game.']
	if (held.challenge !== null) {
		const who =
			held.players === null
				? 'As it was scored'
				: held.now
					? `With the ${playersText(held.players)} seated now`
					: `With the ${playersText(held.players)} seated when it was scored`
		lines.push(`${who}: ${held.challenge.toFixed(1)}`)
	}
	if (score.fullChallenge !== null)
		lines.push(`With every seat taken: ${score.fullChallenge.toFixed(1)}`)
	const ago = age(score.scoredAt, now)
	if (ago)
		lines.push(
			held.now
				? `Last scored ${ago} by someone in this room; seats taken since are counted, other changes show here only if they moved the map.`
				: `Last scored ${ago} by someone in this room; a change since then shows here only if it moved the map.`,
		)
	return lines.join('\n')
}
