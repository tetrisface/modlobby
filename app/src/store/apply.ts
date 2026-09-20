import { batch } from 'solid-js'
import { produce, reconcile, unwrap } from 'solid-js/store'
import type { BattleView } from '../ipc/bindings/BattleView'
import type { Delta } from '../ipc/bindings/Delta'
import type { ServerSnapshot } from '../ipc/bindings/ServerSnapshot'
import type { Snapshot } from '../ipc/bindings/Snapshot'
import type { UiMessage } from '../ipc/bindings/UiMessage'
import { noteToldStart } from './running'
import {
	BATTLE_ROOM,
	applyChannel,
	applyDirectory,
	clearChat,
	clearRoom,
	leaveChannelsOf,
	pushLine,
	pushNotice,
	roomKey,
} from './chat'
import { raise } from '../ipc/alerts'
import {
	emptyLobby,
	emptyServer,
	lobby,
	setLobby,
	type LobbyState,
	type ServerState,
} from './lobby'

export function applyMessage(message: UiMessage): void {
	if (message.type === 'snapshot') {
		applySnapshot(message.data)
		return
	}
	if (message.type === 'session') {
		applySession(message.data)
		return
	}
	const { server, deltas } = message.data
	// One render pass per message: a room's join burst is thirty status lines,
	// and applied one by one each of them rebuilt every row on screen.
	batch(() => {
		for (const delta of deltas) applyDelta(delta, server)
	})
}

/** Whether the first snapshot has landed: the last startup milestone. */
let snapshotSeen = false

function serverState(snapshot: ServerSnapshot): ServerState {
	const state: ServerState = {
		...emptyServer(),
		phase: snapshot.phase,
		retryAt: retryAt(snapshot.retryIn),
		me: snapshot.me,
		myBattle: snapshot.myBattle,
		gameRunning: snapshot.gameRunning,
		friends: snapshot.friends,
	}
	for (const user of snapshot.users) state.users[user.name] = user
	for (const battle of snapshot.battles) state.battles[battle.id] = battle
	return state
}

export function applySnapshot(snapshot: Snapshot): void {
	if (!snapshotSeen) {
		snapshotSeen = true
		console.debug(
			`startup: first snapshot at ${Math.round(performance.now())} ms`,
		)
	}
	const next: LobbyState = {
		...emptyLobby(),
		engine: snapshot.engine,
		download: snapshot.download,
		paste: snapshot.paste,
		skirmish: snapshot.skirmish,
		ways: snapshot.ways,
	}
	for (const session of snapshot.servers)
		next.servers[session.server] = serverState(session)
	setLobby(reconcile(next))
	if (snapshot.servers.every((session) => session.phase === null)) clearChat()
	// Membership is replayed on reconnect; the chat backlog is not, so whatever
	// the front end still holds stays put.
	else
		for (const session of snapshot.servers)
			for (const channel of session.channels)
				applyChannel(roomKey(session.server, channel.name), channel)
}

/**
 * One server's session over again — what its login ends in — in place of
 * whatever was held of it. Everyone else's is left alone.
 */
export function applySession(session: ServerSnapshot): void {
	batch(() => {
		setLobby('servers', session.server, reconcile(serverState(session)))
		for (const channel of session.channels)
			applyChannel(roomKey(session.server, channel.name), channel)
	})
}

/** Mirrors `lobby-core`: members minus spectators, the host bot counting as one. */
function playerCount(battle: BattleView): number {
	return Math.max(0, battle.members.length - battle.spectatorCount)
}

/** The runtime's "in N seconds" as a moment on this clock. */
function retryAt(seconds: number | null): number | null {
	return seconds === null ? null : Date.now() + seconds * 1000
}

/**
 * Applies one change. What belongs to this machine — the engine, a download,
 * a skirmish — lands at the top whatever it is tagged with; everything else
 * is `server`'s session's, and without a server it has nowhere to go.
 */
export function applyDelta(delta: Delta, server: string | null = null): void {
	switch (delta.type) {
		case 'engine':
			setLobby('engine', delta.data)
			return
		case 'content':
			setLobby('content', delta.data)
			return
		case 'download':
			setLobby('download', delta.data)
			return
		case 'paste':
			setLobby('paste', delta.data)
			return
		case 'skirmish':
			setLobby('skirmish', delta.data)
			return
		case 'ways':
			setLobby('ways', reconcile(delta.data))
			return
		case 'alert':
			void raise(delta.data.kind, delta.data.text)
			return
		case 'notice':
			pushNotice(delta.data.level, delta.data.text, server)
			return
		case 'chat':
			pushLine({ ...delta.data, room: roomKey(server, delta.data.room) })
			return
	}
	if (server === null) {
		console.warn(`a ${delta.type} change with no server to file it under`)
		return
	}
	applySessionDelta(delta, server)
}

function applySessionDelta(delta: Delta, server: string): void {
	if (!lobby.servers[server]) setLobby('servers', server, emptyServer())
	const session = lobby.servers[server]
	if (!session) return
	switch (delta.type) {
		case 'retryIn':
			setLobby('servers', server, 'retryAt', retryAt(delta.data))
			return
		case 'phase':
			if (delta.data === null) {
				// Losing a session drops everything that server told us — and only
				// that server's. The engine, a download, the skirmish room and the
				// remembered ways in belong to this machine and outlive it, as does
				// every other server's session. A skirmish started from here keeps
				// running when the connection goes, and resetting `engine` to idle
				// would re-enable the button that starts a second one on top of it;
				// a skirmish being set up is somebody's work, and dropping it because
				// a socket died would be the one moment they most wanted to keep
				// playing.
				setLobby('servers', server, reconcile(emptyServer()))
				leaveChannelsOf(server)
				return
			}
			setLobby('servers', server, 'phase', delta.data)
			return
		case 'userAdded':
			setLobby('servers', server, 'users', delta.data.name, delta.data)
			return
		case 'userRemoved':
			setLobby(
				'servers',
				server,
				'users',
				produce((users) => {
					delete users[delta.data.name]
				}),
			)
			return
		case 'userStatus':
			if (session.users[delta.data.name]) {
				setLobby(
					'servers',
					server,
					'users',
					delta.data.name,
					'status',
					delta.data.status,
				)
			}
			return
		case 'battleOpened':
			setLobby('servers', server, 'battles', delta.data.id, delta.data)
			return
		case 'battleClosed':
			setLobby(
				'servers',
				server,
				'battles',
				produce((battles) => {
					delete battles[delta.data.id]
				}),
			)
			return
		case 'battleInfo': {
			const { id, spectatorCount, locked, mapHash, mapName } = delta.data
			if (!session.battles[id]) return
			setLobby(
				'servers',
				server,
				'battles',
				id,
				produce((battle) => {
					battle.spectatorCount = spectatorCount
					battle.locked = locked
					battle.mapHash = mapHash
					battle.mapName = mapName
					battle.playerCount = playerCount(battle)
				}),
			)
			return
		}
		case 'battleTitle':
			if (session.battles[delta.data.id]) {
				setLobby(
					'servers',
					server,
					'battles',
					delta.data.id,
					'title',
					delta.data.title,
				)
			}
			return
		case 'battleLayout':
			if (session.battles[delta.data.id]) {
				setLobby(
					'servers',
					server,
					'battles',
					delta.data.id,
					'layout',
					delta.data.layout,
				)
			}
			return
		case 'battleQueue':
			if (session.battles[delta.data.id]) {
				setLobby(
					'servers',
					server,
					'battles',
					delta.data.id,
					'queue',
					delta.data.names,
				)
			}
			return
		case 'member': {
			const { id, name, joined } = delta.data
			if (session.battles[id]) {
				setLobby(
					'servers',
					server,
					'battles',
					id,
					produce((battle) => {
						const members = battle.members.filter((m) => m !== name)
						if (joined) members.push(name)
						battle.members = members.sort()
						battle.playerCount = playerCount(battle)
					}),
				)
			}
			if (session.users[name]) {
				setLobby(
					'servers',
					server,
					'users',
					name,
					'battleId',
					joined ? id : null,
				)
			}
			return
		}
		case 'memberStatus':
			if (session.users[delta.data.name]) {
				setLobby(
					'servers',
					server,
					'users',
					delta.data.name,
					'battleStatus',
					delta.data.status,
				)
			}
			return
		case 'bot': {
			const { id, name, bot } = delta.data
			if (!session.battles[id]) return
			setLobby(
				'servers',
				server,
				'battles',
				id,
				'bots',
				produce((bots) => {
					const index = bots.findIndex((b) => b.name === name)
					if (bot === null) {
						if (index >= 0) bots.splice(index, 1)
					} else if (index >= 0) {
						bots[index] = bot
					} else {
						bots.push(bot)
					}
				}),
			)
			return
		}
		case 'startRect': {
			const id = session.myBattle?.id
			if (id === undefined || !session.battles[id]) return
			const { allyTeam, rect } = delta.data
			setLobby(
				'servers',
				server,
				'battles',
				id,
				'startRects',
				produce((rects) => {
					const index = rects.findIndex((r) => r.allyTeam === allyTeam)
					if (rect === null) {
						if (index >= 0) rects.splice(index, 1)
					} else if (index >= 0) {
						rects[index] = rect
					} else {
						rects.push(rect)
					}
				}),
			)
			return
		}
		case 'scriptTags':
			if (!session.myBattle) return
			setLobby(
				'servers',
				server,
				'myBattle',
				produce((my) => {
					if (!my) return
					for (const [key, value] of delta.data.set) my.scriptTags[key] = value
					for (const key of delta.data.removed) delete my.scriptTags[key]
				}),
			)
			return
		case 'modOption': {
			if (!session.myBattle) return
			const { key, value, change } = delta.data
			setLobby(
				'servers',
				server,
				'myBattle',
				produce((my) => {
					if (!my) return
					my.scriptTags[`game/modoptions/${key}`] = value
					if (!change) return
					const index = my.history.findIndex((h) => h.seq === change.seq)
					if (index >= 0) my.history[index] = change
					else my.history.push(change)
				}),
			)
			return
		}
		case 'vote':
			if (session.myBattle)
				setLobby('servers', server, 'myBattle', 'vote', delta.data)
			return
		case 'myBattle':
			// Another room is another conversation. Only on a change of room: a
			// reconnect replays the same room through the snapshot and keeps the
			// backlog, as the snapshot's own comment says.
			if (session.myBattle?.id !== delta.data?.id) clearRoom(BATTLE_ROOM)
			setLobby('servers', server, 'myBattle', delta.data)
			return
		case 'gameRunning':
			setLobby('servers', server, 'gameRunning', delta.data)
			return
		case 'gameStartedAgo':
			noteToldStart(server, delta.data.id, delta.data.seconds)
			return
		case 'channel':
			applyChannel(roomKey(server, delta.data.name), delta.data.channel)
			return
		case 'directory':
			applyDirectory(server, delta.data)
			return
		case 'friends':
			setLobby('servers', server, 'friends', delta.data)
			return
	}
}
