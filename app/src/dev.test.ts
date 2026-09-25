import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { BattleStatusView } from './ipc/bindings/BattleStatusView'
import type { UserView } from './ipc/bindings/UserView'
import { breath, emit, held, inGame } from './dev'
import { applySnapshot } from './store/apply'
import { lobby } from './store/lobby'

const seat = (over: Partial<BattleStatusView> = {}): BattleStatusView => ({
	ready: false,
	team: 0,
	allyTeam: 0,
	player: true,
	handicap: 0,
	sync: 'unsynced',
	side: 0,
	...over,
})

const user = (
	name: string,
	battleStatus: BattleStatusView | null,
): UserView => ({
	name,
	country: 'SE',
	userId: 1,
	lobbyClient: 'modlobby',
	status: { inGame: false, away: false, rank: 0, moderator: false, bot: false },
	battleStatus,
	battleId: 5,
})

/** Seated and not ready in room 5, with alice seated beside us. */
function inARoom() {
	applySnapshot({
		servers: [
			{
				server: 'S',
				phase: 'ready',
				retryIn: null,
				me: 'me',
				users: [user('me', seat()), user('alice', seat()), user('host', null)],
				battles: [
					{
						id: 5,
						founder: 'host',
						ip: '',
						port: 0,
						maxPlayers: 16,
						passworded: false,
						locked: false,
						mapHash: '',
						mapName: 'Map',
						engineName: '',
						engineVersion: '',
						title: 'Room',
						gameName: 'BAR',
						members: ['host', 'me', 'alice'],
						spectatorCount: 1,
						playerCount: 2,
						layout: null,
						bots: [],
						startRects: [],
						queue: [],
					},
				],
				myBattle: {
					boss: null,
					autoBalance: null,
					preset: null,
					id: 5,
					gameHash: '',
					scriptTags: {},
					vote: null,
					history: [],
					preReady: false,
					readyOnItsWay: null,
					seatOnItsWay: null,
					heldUntilMs: null,
				},
				gameRunning: null,
				channels: [],
				friends: { friends: [], requests: [], ignored: [] },
			},
		],
		engine: { state: 'idle' },
		download: { state: 'idle' },
		paste: { state: 'idle' },
		skirmish: null,
		ways: {},
		content: null,
		contentCheck: { game: null, map: null, mutators: [] },
	})
}

const S = () => lobby.servers.S!

describe('the dev console hooks', () => {
	beforeEach(() => {
		vi.useFakeTimers()
		inARoom()
	})
	afterEach(() => vi.useRealTimers())

	test('a held press shows, leaves, and the room goes back', () => {
		held(8000)
		expect(S().myBattle?.readyOnItsWay).toBe(true)
		expect(S().myBattle?.heldUntilMs).toBeGreaterThan(Date.now())
		vi.advanceTimersByTime(8000)
		expect(S().myBattle?.heldUntilMs).toBeNull()
		expect(S().myBattle?.readyOnItsWay).toBe(true)
		vi.advanceTimersByTime(400)
		expect(S().myBattle?.readyOnItsWay).toBeNull()
	})

	test('the breath readies everyone else and leaves us asked', () => {
		breath()
		// Never ready on the way, or our button would flash green.
		expect(S().users.me?.battleStatus?.ready).toBe(false)
		expect(S().users.alice?.battleStatus?.ready).toBe(false)
		vi.advanceTimersByTime(50)
		expect(S().users.alice?.battleStatus).toMatchObject({
			ready: true,
			sync: 'synced',
		})
		expect(S().users.me?.battleStatus?.ready).toBe(false)
	})

	test('alone in the room, the breath seats a stand-in to be ready', () => {
		emit({
			type: 'memberStatus',
			data: { name: 'alice', status: seat({ player: false }), teamColour: 0 },
		})
		breath()
		vi.advanceTimersByTime(50)
		expect(lobby.servers.S!.battles[5]?.members).toContain('dev-stand-in')
		expect(S().users['dev-stand-in']?.battleStatus).toMatchObject({
			player: true,
			ready: true,
			sync: 'synced',
		})
	})

	test('the breath is refused while a game runs, since nothing is asked', () => {
		emit({
			type: 'gameRunning',
			data: { id: 5, ip: '', port: 0, added: true },
		})
		expect(() => breath()).toThrow(/game is running/)
	})

	test('swords go on and come off', () => {
		inGame('alice')
		expect(S().users.alice?.status.inGame).toBe(true)
		inGame('alice', false)
		expect(S().users.alice?.status.inGame).toBe(false)
	})
})
