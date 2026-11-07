/**
 * How the battle list says what a room last scored.
 *
 * The number is the in-room panel's challenge, read back off the service's
 * live games list (`crates/pve/src/listing.rs`), so it is only as new as the
 * last time someone in that room asked about it. The tooltip says how long
 * ago that was, because the list cannot see a room's setup change -- only
 * its map, which is why a room on another map shows nothing.
 */

import type { ListedScore } from '../ipc/bindings/ListedScore'
import { age } from './age'

/** The panel's figure without its "/34": a column of them needs no scale. */
export function challengeLabel(score: ListedScore): string {
	return score.challenge.toFixed(1)
}

/** What a hover on the figure explains: the scale, and how old it is. */
export function challengeTip(score: ListedScore, now: number): string {
	const scale = `Challenge ${challengeLabel(score)} of 34, where 17 is an even game.`
	const ago = age(score.scoredAt, now)
	if (!ago) return scale
	return `${scale}\nLast scored ${ago} by someone in this room; a change since then shows here only if it moved the map.`
}
