import { describe, expect, test, vi } from 'vitest'
import type { LanRoomView } from '../ipc/bindings/LanRoomView'
import { lanCaps, lanRows } from './lan'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const room = (id: number, host: string, title: string): LanRoomView => ({
	id,
	address: '192.168.1.5',
	port: 8200,
	title,
	host,
	engineVersion: '2026.07.04',
	game: 'Beyond All Reason test-31357-b06bb1a',
	map: 'Supreme Isthmus v2.1',
	players: 2,
	maxPlayers: 8,
	passworded: true,
})

describe('rooms heard on the network', () => {
	test('become rows of the battle list on the LAN server', () => {
		const rows = lanRows([room(7, 'ann', "Ann's game")], null)
		expect(rows).toHaveLength(1)
		const [row] = rows
		expect(row!.server).toBe('lan')
		expect(row!.key).toBe('lan/7')
		expect(row!.battle.founder).toBe('ann')
		expect(row!.battle.ip).toBe('192.168.1.5')
		expect(row!.battle.passworded).toBe(true)
		expect(row!.battle.playerCount).toBe(2)
		expect(row!.battle.mapName).toBe('Supreme Isthmus v2.1')
	})

	test('the room we are in is listed once, by the session', () => {
		const rooms = [room(7, 'ann', "Ann's game"), room(8, 'bob', "Bob's game")]
		const rows = lanRows(rooms, { founder: 'ann', title: "Ann's game" })
		expect(rows.map((row) => row.battle.founder)).toEqual(['bob'])
	})
})

describe('what a LAN room may do', () => {
	test('the founder starts and picks; a guest neither', () => {
		let founder = false
		const caps = lanCaps(() => founder)
		expect(caps.spads).toBe(false)
		expect(caps.plays).toBe(true)
		// Nobody is waiting on a flag here, so no button offers one; the room
		// reports every seat ready instead.
		expect(caps.ready).toBe(false)
		expect(caps.startsGame).toBe(false)
		expect(caps.picksContent).toBe(false)
		founder = true
		expect(caps.startsGame).toBe(true)
		expect(caps.picksContent).toBe(true)
	})
})
