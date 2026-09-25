import { createStore } from 'solid-js/store'
import type { ContentCheckView } from '../ipc/bindings/ContentCheckView'
import type { BattleView } from '../ipc/bindings/BattleView'
import type { EngineStatus } from '../ipc/bindings/EngineStatus'
import type { GameRunningView } from '../ipc/bindings/GameRunningView'
import type { MyBattleView } from '../ipc/bindings/MyBattleView'
import type { Phase } from '../ipc/bindings/Phase'
import type { SkirmishView } from '../ipc/bindings/SkirmishView'
import type { DownloadStatus } from '../ipc/bindings/DownloadStatus'
import type { PasteStatus } from '../ipc/bindings/PasteStatus'
import type { FriendsView } from '../ipc/bindings/FriendsView'
import type { UserView } from '../ipc/bindings/UserView'
import { serverId } from '../lib/servers'
import { settings } from './settings'

/** One server's session, as the runtime last described it. */
export type ServerState = {
	phase: Phase | null
	/**
	 * When the runtime next tries the last credentials on its own, as a
	 * `Date.now()` moment, while it means to. Made here from the seconds the
	 * runtime sends, so the corner can count down without another round trip.
	 */
	retryAt: number | null
	me: string | null
	users: Record<string, UserView>
	battles: Record<number, BattleView>
	myBattle: MyBattleView | null
	gameRunning: GameRunningView | null
	friends: FriendsView
}

/** A dumb mirror of the runtime's state; only `apply.ts` writes to it. */
export type LobbyState = {
	/** Each server there is a session with, by its id: the lowercased host. */
	servers: Record<string, ServerState>
	engine: EngineStatus
	/** Whether this machine has the room's engine, game and map. */
	content: { engine: boolean; game: boolean; map: boolean } | null
	/** The room's game and map here held to the checksums it announced. */
	contentCheck: ContentCheckView
	download: DownloadStatus
	/** A multi-line paste on its way to the room. */
	paste: PasteStatus
	/**
	 * The room with no server behind it.
	 *
	 * Its own branch rather than a row in some server's `battles`, because it
	 * is not a session's: it is still here after a logout, a dropped
	 * connection or a reconnect, all of which clear a server's state.
	 */
	skirmish: SkirmishView | null
	/**
	 * The way into each server that worked last, as it reads — "STLS on 8200,
	 * 46 ms" — by server id. This machine's memory rather than the session's,
	 * so a logout keeps it.
	 */
	ways: Record<string, string>
}

export function emptyServer(): ServerState {
	return {
		phase: null,
		retryAt: null,
		me: null,
		users: {},
		battles: {},
		myBattle: null,
		gameRunning: null,
		friends: { friends: [], requests: [], ignored: [] },
	}
}

export function emptyLobby(): LobbyState {
	return {
		servers: {},
		engine: { state: 'idle' },
		content: null,
		contentCheck: { game: null, map: null, mutators: [] },
		download: { state: 'idle' },
		paste: { state: 'idle' },
		skirmish: null,
		ways: {},
	}
}

export const [lobby, setLobby] = createStore<LobbyState>(emptyLobby())

/**
 * Every server's session, in the order the settings list the servers — the
 * one order the reader chose — with any the settings no longer have after.
 */
export function sessions(): [string, ServerState][] {
	const listed = (settings()?.servers ?? []).map((entry) =>
		serverId(entry.host),
	)
	const place = (id: string) => {
		const at = listed.indexOf(id)
		return at < 0 ? listed.length : at
	}
	return Object.entries(lobby.servers).sort(
		([a], [b]) => place(a) - place(b) || a.localeCompare(b),
	)
}

/**
 * The server whose room we are in. There is one room at most across every
 * server: joining one leaves the other.
 */
export function roomServer(): string | undefined {
	return sessions().find(([, session]) => session.myBattle !== null)?.[0]
}

/** The session the room is on. */
export function roomSession(): ServerState | undefined {
	const server = roomServer()
	return server === undefined ? undefined : lobby.servers[server]
}

/**
 * The server a view means when it means just one: the room's, else the
 * first that is logged in, else the first.
 */
export function mainServer(): string | undefined {
	const all = sessions()
	return (
		roomServer() ??
		all.find(([, session]) => session.phase === 'ready')?.[0] ??
		all[0]?.[0]
	)
}

/** The session of `mainServer`. */
export function mainSession(): ServerState | undefined {
	const server = mainServer()
	return server === undefined ? undefined : lobby.servers[server]
}

/** Whether any server is logged in and ready. */
export function anyReady(): boolean {
	return sessions().some(([, session]) => session.phase === 'ready')
}

/** Whether any server has a session, or one on its way. */
export function anyConnected(): boolean {
	return sessions().some(([, session]) => session.phase !== null)
}

/**
 * Whether we show as away: on every server logged in, the bit the server
 * keeps — what everyone else sees — says so. Nothing is away with no session.
 */
export function allAway(): boolean {
	const ready = sessions().filter(([, session]) => session.phase === 'ready')
	return (
		ready.length > 0 &&
		ready.every(
			([, session]) =>
				session.me !== null && session.users[session.me]?.status.away === true,
		)
	)
}

/** The servers logged in and ready, in a stable order. */
export function readyServers(): string[] {
	return sessions()
		.filter(([, session]) => session.phase === 'ready')
		.map(([server]) => server)
}

/**
 * Whether more than one server has a session: when what came from which
 * server is worth saying. With one, the lobby reads as it did before servers.
 */
export function severalServers(): boolean {
	return sessions().filter(([, session]) => session.phase !== null).length > 1
}

/** The soonest a server is tried again on its own, if any is to be. */
export function soonestRetry(): number | null {
	const moments = sessions()
		.map(([, session]) => session.retryAt)
		.filter((at): at is number => at !== null)
	return moments.length === 0 ? null : Math.min(...moments)
}

/**
 * The room you are in, if it is still on the list.
 *
 * `myBattle` can point at a row that is gone — closed under us, or not yet
 * replayed after a reconnect — so callers get nothing rather than a room
 * with empty lines.
 */
export function myRoom(): BattleView | undefined {
	const session = roomSession()
	const id = session?.myBattle?.id
	return id === undefined ? undefined : session?.battles[id]
}
