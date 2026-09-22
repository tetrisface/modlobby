import type { RoomModel } from './model'
import { readiness } from './readiness'

/**
 * How each segment of the posture control is drawn.
 *
 * - `plain`: not held, not offered as the next step.
 * - `held`: the posture we are in. Every posture to its left is implied.
 * - `next`: the step after where we are, outlined to invite it.
 * - `asked`: Ready while the room asks it of us. The one loud look.
 * - `armed`: a ready given in advance, for when it can take effect.
 */
export type Look = 'plain' | 'held' | 'next' | 'asked' | 'armed'

export type Segment = {
	look: Look
	/** A caption after the label: `queued 2 of 5`, `next game`, `when seated`. */
	note: string | null
	/** A press of ours the server has not answered yet. */
	pending: boolean
	/** Everyone else is ready: the room is waiting on this one press. */
	waiting: boolean
}

export type Posture = {
	watch: Segment
	play: Segment
	ready: Segment
	/**
	 * When the status we asked for leaves, in Unix milliseconds, while the
	 * flood window holds it; null otherwise. Drawn as a hairline draining
	 * along the pending segment.
	 */
	heldUntil: number | null
}

const segment = (
	look: Look,
	note: string | null = null,
	pending = false,
	waiting = false,
): Segment => ({ look, note, pending, waiting })

/**
 * The three postures, read left to right as commitment, from what the room
 * shows and what we have asked for.
 *
 * Ready follows our own wish before the server's answer (`readyOnItsWay`),
 * so a press shows at once; the server's word is still what is true.
 */
export function posture(room: RoomModel): Posture {
	const me = room.me()
	const mine = me === null ? undefined : room.users()[me]?.battleStatus
	const my = room.my()
	// Our own newest word on the seat comes first, so a press shows at once;
	// the server's word is what it settles to.
	const wished = my?.seatOnItsWay ?? null
	const seated = wished?.player ?? mine?.player ?? false
	const seatPending = wished !== null
	const queue = room.battle()?.queue ?? []
	const place = me === null ? -1 : queue.indexOf(me)
	const armed = my?.preReady ?? false
	const wish = my?.readyOnItsWay ?? null
	const heldUntil = my?.heldUntilMs ?? null

	if (!seated && place >= 0) {
		return {
			watch: segment('plain'),
			play: segment(
				'held',
				`queued ${place + 1} of ${queue.length}`,
				seatPending,
			),
			ready: segment(armed ? 'armed' : 'plain', armed ? 'when seated' : null),
			heldUntil,
		}
	}
	if (!seated) {
		// A Ready pressed while watching is a seat with a ready to follow: the
		// wish is on its way before there is a seat to show it on.
		return {
			watch: segment('held', null, seatPending),
			play: segment('next'),
			ready: segment('plain', null, wish === true),
			heldUntil,
		}
	}
	if (room.running() !== null) {
		return {
			watch: segment('plain'),
			play: segment('held', 'next game', seatPending),
			ready: segment(armed ? 'armed' : 'plain', 'next game'),
			heldUntil,
		}
	}
	const tier = readiness(room)
	const ready = wish ?? tier === 'settled'
	return {
		watch: segment('plain'),
		play: segment('held', null, seatPending),
		ready: segment(
			ready ? 'held' : 'asked',
			null,
			wish !== null,
			tier === 'waiting' && !ready,
		),
		heldUntil,
	}
}
