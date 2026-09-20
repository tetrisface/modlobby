import type { BattleView } from '../ipc/bindings/BattleView'
import type { LanRoomView } from '../ipc/bindings/LanRoomView'
import { api } from '../ipc/client'
import { battleKey, type Row } from '../lib/battles'
import { serverId } from '../lib/servers'
import type { RoomCaps, RoomModel } from '../views/room/model'
import { onlineRoom } from '../views/room/online'
import { lanApi } from './api'

/** The servers entry that is the local network, as `serverId` names it. */
export const LAN = 'lan'

export const isLan = (entry: { host: string }): boolean =>
	serverId(entry.host) === LAN

/**
 * What a LAN room may do, for whoever is in it.
 *
 * Nobody hosts it but a person, so the SPADS surfaces are off and the room
 * behaves as one you run: what you change applies. The founder alone starts
 * the game and picks what it is played on, since it is their engine that
 * runs it. Two readings are live, because who the founder is arrives with
 * the room.
 */
export function lanCaps(founder: () => boolean): RoomCaps {
	return {
		spads: false,
		chat: true,
		ready: true,
		get startsGame() {
			return founder()
		},
		leave: true,
		get picksContent() {
			return founder()
		},
		plays: true,
	}
}

/**
 * The room on the LAN: the online room, read the same way, with the two
 * things that differ. Starting is the host's engine coming up as the game
 * server, not a request to a host; and the title is the one the room was
 * opened with.
 */
export function lanRoom(): RoomModel {
	const base = onlineRoom()
	const founder = () => {
		const battle = base.battle()
		const me = base.me()
		return !!battle && me !== null && battle.founder === me
	}
	return {
		...base,
		caps: lanCaps(founder),
		io: {
			...base.io,
			launch: () => (founder() ? lanApi.start() : api.launch()),
			renameRoom: () =>
				Promise.reject(
					new Error('a room on the LAN keeps the name it opened with'),
				),
		},
	}
}

/**
 * Rooms heard on the network as rows of the battle list, tagged with the LAN
 * server. The one we are in is left out: the session lists it already, as
 * the room it is, and a second row would be the same room twice.
 */
export function lanRows(
	rooms: readonly LanRoomView[],
	inRoom: { founder: string; title: string } | null,
): Row[] {
	return rooms
		.filter(
			(room) =>
				inRoom === null ||
				room.host !== inRoom.founder ||
				room.title !== inRoom.title,
		)
		.map((room) => ({
			server: LAN,
			key: battleKey(LAN, room.id),
			battle: battleOf(room),
			running: false,
			hasFriend: false,
		}))
}

/** What the list draws a found room as, before anybody has joined it. */
function battleOf(room: LanRoomView): BattleView {
	return {
		id: room.id,
		founder: room.host,
		ip: room.address,
		port: room.port,
		maxPlayers: room.maxPlayers,
		passworded: room.passworded,
		locked: false,
		mapHash: '0',
		mapName: room.map,
		engineName: 'Recoil',
		engineVersion: room.engineVersion,
		title: room.title,
		gameName: room.game,
		members: [room.host],
		spectatorCount: 0,
		playerCount: room.players,
		layout: null,
		bots: [],
		startRects: [],
		queue: [],
	}
}
