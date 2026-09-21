import type { ServerState } from '../store/lobby'
import { when } from './presets'

/**
 * How many were on a server, and when that was: unix seconds, as `when`
 * reads them. No server says before a login, so a server not logged in to
 * shows the last count seen while it was.
 */
export type Seen = { users: number; rooms: number; at: number }

/** Every server's last count, in one map by server id. */
const KEY = 'servers.seen'

export function headCount(session: Pick<ServerState, 'users' | 'battles'>): {
	users: number
	rooms: number
} {
	return {
		users: Object.keys(session.users).length,
		rooms: Object.keys(session.battles).length,
	}
}

function readAll(storage: Storage | null): Record<string, Seen> {
	try {
		const parsed: unknown = JSON.parse(storage?.getItem(KEY) ?? '{}')
		return parsed && typeof parsed === 'object'
			? (parsed as Record<string, Seen>)
			: {}
	} catch {
		return {}
	}
}

export function readSeen(storage: Storage | null, server: string): Seen | null {
	return readAll(storage)[server] ?? null
}

export function rememberSeen(
	storage: Storage | null,
	server: string,
	seen: Seen,
): void {
	try {
		storage?.setItem(
			KEY,
			JSON.stringify({ ...readAll(storage), [server]: seen }),
		)
	} catch {
		// Full or refused: the card goes without its last count, nothing worse.
	}
}

/** "312 online, 41 rooms" while logged in; after, the same and how long ago. */
export function seenText(
	live: { users: number; rooms: number } | null,
	remembered: Seen | null,
	now: number,
): string | null {
	if (live) return count(live)
	if (remembered) return `${count(remembered)}, ${when(remembered.at, now)}`
	return null
}

function count({ users, rooms }: { users: number; rooms: number }): string {
	return `${users} online, ${rooms} ${rooms === 1 ? 'room' : 'rooms'}`
}
