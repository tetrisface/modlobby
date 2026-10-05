import type { IntentView } from '../../ipc/bindings/IntentView'
import type { RoomModel } from './model'
import { readiness } from './readiness'

/**
 * How each segment of the posture control is drawn.
 *
 * - `plain`: not held, not offered as the next step.
 * - `held`: the posture we are in. Every posture to its left is implied.
 * - `next`: the step after where we are, outlined to invite it.
 * - `asked`: Ready while the room asks it of us. The one loud look.
 *
 * A ready for the next game, or for when the queue seats us, is held like
 * any other: its note says when.
 */
export type Look = 'plain' | 'held' | 'next' | 'asked'

export type Segment = {
	look: Look
	/** A caption after the label: `queued 2 of 5`, `next game`, `when seated`. */
	note: string | null
	/** A press of ours the server has not answered yet. */
	pending: boolean
	/** Everyone else is ready: the room is waiting on this one press. */
	waiting: boolean
}

/**
 * Where the player stands: what they asked for (the session's intent) read
 * against the room. Readying is a way of playing, so every stance but
 * `spectate` is a seat, held or wanted.
 *
 * - `spectate`: watching.
 * - `queuing`: wanting a seat in a full room, in its join queue.
 * - `playNext`, `readyNext`: a seat while a game runs, so for the next one.
 *   Only a ready given for the next game outlasts the reset at its end.
 * - `play`, `ready`: a seat between games.
 */
export type Stance =
	| 'spectate'
	| 'queuing'
	| 'playNext'
	| 'readyNext'
	| 'play'
	| 'ready'

export type Posture = {
	stance: Stance
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

function stance(
	intent: IntentView,
	seated: boolean,
	place: number,
	running: boolean,
): Stance {
	if (intent === 'spectate') return 'spectate'
	if (!seated && place >= 0) return 'queuing'
	if (running) return intent === 'readyNext' ? 'readyNext' : 'playNext'
	return intent === 'play' ? 'play' : 'ready'
}

/** The three postures, read left to right as commitment. */
export function posture(room: RoomModel): Posture {
	const me = room.me()
	const my = room.my()
	const intent = my?.intent ?? 'spectate'
	// Our own newest word on the seat comes first, so a press shows at once;
	// the server's word is what it settles to.
	const wished = my?.seatOnItsWay ?? null
	const shown = me === null ? undefined : room.users()[me]?.battleStatus
	const seated = wished?.player ?? shown?.player ?? false
	const queue = room.battle()?.queue ?? []
	const place = me === null ? -1 : queue.indexOf(me)
	const running = room.running() !== null
	// A seat wanted and not yet shown: on its way, or the room still to say
	// whether its queue took us.
	const seatPending = wished !== null || (intent !== 'spectate' && !seated)
	const readyPending = (my?.readyOnItsWay ?? null) !== null
	const ready = (look: Look, note: string | null = null, waiting = false) =>
		segment(look, note, readyPending, waiting)

	const at = stance(intent, seated, place, running)
	const base = {
		stance: at,
		heldUntil: my?.heldUntilMs ?? null,
		watch: segment('plain'),
	}
	switch (at) {
		case 'spectate':
			return {
				...base,
				watch: segment('held', null, wished !== null),
				play: segment('next'),
				ready: ready('plain', running ? 'next game' : null),
			}
		case 'queuing': {
			const readies = intent !== 'play'
			return {
				...base,
				play: segment(
					'held',
					`queued ${place + 1} of ${queue.length}`,
					wished !== null,
				),
				ready: ready(
					readies ? 'held' : 'plain',
					readies ? 'when seated' : null,
				),
			}
		}
		case 'playNext':
			return {
				...base,
				play: segment('held', 'next game', seatPending),
				ready: ready('plain', 'next game'),
			}
		case 'readyNext':
			return {
				...base,
				play: segment('held', 'next game', seatPending),
				ready: ready('held', 'next game'),
			}
		case 'play':
			return {
				...base,
				play: segment('held', null, seatPending),
				ready: ready('asked', null, readiness(room) === 'waiting'),
			}
		case 'ready':
			return {
				...base,
				play: segment('held', null, seatPending),
				ready: ready('held'),
			}
	}
}
