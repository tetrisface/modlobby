import type { RoomModel } from './model'

/**
 * How much readying up asks of us in this room, which is how loud the ready
 * control is.
 *
 * - `none`: nothing to say. Not seated, a room nobody waits on, or a game
 *   running -- the server unreadies everyone when it ends, so a ready given
 *   now is wiped (a pre-ready answers that instead).
 * - `settled`: ready. Nothing more is asked.
 * - `asked`: seated and not ready. SPADS will not start, or even put a start
 *   to the vote, while a synced player is unready.
 * - `waiting`: as `asked`, and every other seated player is ready and synced,
 *   so the room is waiting on us alone.
 *
 * Our own sync is left out: readying while downloading is allowed, and the
 * download is its own mark. Everyone else's counts, because SPADS refuses an
 * unsynced player before it looks at who is ready.
 */
export type Readiness = 'none' | 'settled' | 'asked' | 'waiting'

export function readiness(room: RoomModel): Readiness {
	if (!room.caps.ready || room.running() !== null) return 'none'
	const me = room.me()
	const users = room.users()
	const mine = me === null ? undefined : users[me]?.battleStatus
	if (!mine?.player) return 'none'
	if (mine.ready) return 'settled'
	// AIs are the room's bots, not its members, and an autohost only ever
	// spectates, so neither is somebody the room could be waiting on.
	const others = (room.battle()?.members ?? []).flatMap((name) => {
		const status = name === me ? undefined : users[name]?.battleStatus
		return status?.player ? [status] : []
	})
	const alone =
		others.length > 0 &&
		others.every((status) => status.ready && status.sync === 'synced')
	return alone ? 'waiting' : 'asked'
}
