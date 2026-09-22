import { describe, expect, test, vi } from 'vitest'
import type { BattleView } from '../ipc/bindings/BattleView'
import type { Delta } from '../ipc/bindings/Delta'
import type { SkirmishView } from '../ipc/bindings/SkirmishView'
import type { ServerSnapshot } from '../ipc/bindings/ServerSnapshot'
import type { Snapshot } from '../ipc/bindings/Snapshot'
import type { UserView } from '../ipc/bindings/UserView'
import { applyDelta, applyMessage, applySnapshot } from './apply'
import { chat } from './chat'
import { lobby } from './lobby'

/** The server the tests' session is on. */
const S = 'server4'
/** Its session as the store holds it. */
const at = (server = S) => lobby.servers[server]!

const user = (name: string, battleId: number | null = null): UserView => ({
	name,
	country: 'SE',
	userId: 1,
	lobbyClient: 'LuaLobby Chobby',
	status: { inGame: false, away: false, rank: 0, moderator: false, bot: false },
	battleStatus: null,
	battleId,
})

const battle = (id: number, members: string[]): BattleView => ({
	id,
	founder: 'host',
	ip: '1.2.3.4',
	port: 8452,
	maxPlayers: 16,
	passworded: false,
	locked: false,
	mapHash: 'h',
	mapName: 'Map',
	engineName: 'spring',
	engineVersion: '2026.07.04',
	title: 'Room',
	gameName: 'BAR',
	members,
	spectatorCount: 1,
	playerCount: Math.max(0, members.length - 1),
	layout: null,
	bots: [],
	startRects: [],
	queue: [],
})

/** A room with no server behind it, as the runtime sends one. */
const room: SkirmishView = {
	battle: { ...battle(0, ['me']), title: 'Skirmish', founder: 'me' },
	my: {
		boss: 'me',
		autoBalance: 'off',
		preset: null,
		id: 0,
		gameHash: '',
		scriptTags: { 'game/modoptions/ranked_game': '0' },
		vote: null,
		history: [],
		preReady: false,
		readyOnItsWay: null,
		seatOnItsWay: null,
		heldUntilMs: null,
	},
	users: [user('me', 0)],
	me: 'me',
	content: { engine: true, game: true, map: true },
}

const session: ServerSnapshot = {
	server: S,
	phase: 'ready',
	retryIn: null,
	me: 'me',
	users: [user('me'), user('host', 5), user('alice')],
	battles: [battle(5, ['host'])],
	myBattle: null,
	gameRunning: null,
	channels: [],
	friends: { friends: [], requests: [], ignored: [] },
}

const snapshot: Snapshot = {
	servers: [session],
	engine: { state: 'idle' },
	download: { state: 'idle' },
	paste: { state: 'idle' },
	skirmish: null,
	ways: {},
	content: null,
	contentCheck: { game: null, map: null },
}

describe('apply', () => {
	test('a notice names its server once there is more than one', () => {
		const kicked: Delta = {
			type: 'notice',
			data: { level: 'warning', text: 'kicked' },
		}
		applySnapshot(snapshot)
		applyDelta(kicked, 'rapid')
		expect(chat.notices.at(-1)).toMatchObject({
			text: 'kicked',
			server: 'rapid',
		})
	})

	test("a server's login replaces its session and nobody else's", () => {
		applySnapshot({
			...snapshot,
			servers: [session, { ...session, server: 'rapid', users: [] }],
		})
		applyMessage({
			type: 'session',
			data: {
				...session,
				server: 'rapid',
				users: [user('me'), user('dave')],
				channels: [{ name: 'main', members: ['me'], topicAuthor: null }],
			},
		})
		expect(Object.keys(at('rapid').users)).toEqual(['me', 'dave'])
		expect(Object.keys(at().users)).toHaveLength(3)
		expect('rapid main' in chat.channels).toBe(true)
	})

	test('a session that ends takes its channels with it, and only its own', () => {
		applySnapshot({
			...snapshot,
			servers: [
				{
					...session,
					channels: [{ name: 'main', members: [], topicAuthor: null }],
				},
				{
					...session,
					server: 'rapid',
					channels: [{ name: 'main', members: [], topicAuthor: null }],
				},
			],
		})
		applyDelta({ type: 'phase', data: null }, 'rapid')
		expect(Object.keys(chat.channels)).toEqual([`${S} main`])
	})

	test("a reloaded window has the room's content and its check from the snapshot", () => {
		applySnapshot({
			...snapshot,
			content: { engine: true, game: true, map: false },
			contentCheck: { game: { verdict: 'same', hash: 7 }, map: null },
		})
		expect(lobby.content).toEqual({ engine: true, game: true, map: false })
		expect(lobby.contentCheck.game).toEqual({ verdict: 'same', hash: 7 })
	})

	test('snapshot then deltas keep the mirror consistent', () => {
		applySnapshot(snapshot)
		expect(Object.keys(at().users)).toHaveLength(3)
		expect(at().battles[5]?.playerCount).toBe(0)

		const deltas: Delta[] = [
			{ type: 'member', data: { id: 5, name: 'alice', joined: true } },
			{
				type: 'battleInfo',
				data: {
					id: 5,
					spectatorCount: 1,
					locked: true,
					mapHash: 'h',
					mapName: 'Map v2',
				},
			},
			{
				type: 'userStatus',
				data: {
					name: 'host',
					status: {
						inGame: true,
						away: false,
						rank: 0,
						moderator: false,
						bot: true,
					},
				},
			},
			{
				type: 'chat',
				data: {
					seq: 1,
					room: '#battle',
					from: 'host',
					text: 'hi',
					kind: 'announcement',
					mention: false,
					at: 0,
				},
			},
			{ type: 'battleQueue', data: { id: 5, names: ['bob', 'alice'] } },
			{ type: 'userRemoved', data: { name: 'alice' } },
		]
		for (const delta of deltas) applyDelta(delta, S)

		expect(at().battles[5]?.members).toEqual(['alice', 'host'])
		expect(at().battles[5]?.queue).toEqual(['bob', 'alice'])
		expect(at().battles[5]?.playerCount).toBe(1)
		expect(at().battles[5]?.locked).toBe(true)
		expect(at().battles[5]?.mapName).toBe('Map v2')
		expect(at().users.host?.status.inGame).toBe(true)
		expect(at().users.alice).toBeUndefined()
		expect(chat.rooms['#battle']!.at(-1)?.text).toBe('hi')
	})

	test('disconnect resets the mirror', () => {
		applySnapshot(snapshot)
		applyDelta({ type: 'phase', data: null }, S)
		expect(at().phase).toBeNull()
		expect(Object.keys(at().battles)).toHaveLength(0)
	})

	test('losing one server leaves the other alone', () => {
		applySnapshot({
			...snapshot,
			servers: [session, { ...session, server: 'other', me: 'someone' }],
		})
		applyDelta({ type: 'phase', data: null }, S)
		expect(at().me).toBeNull()
		expect(at('other').me).toBe('someone')
		expect(Object.keys(at('other').battles)).toHaveLength(1)
	})

	test('a change to a session with no server to file it under is dropped', () => {
		const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
		applySnapshot(snapshot)
		applyDelta({ type: 'userRemoved', data: { name: 'alice' } })
		expect(at().users.alice).toBeDefined()
		expect(warn).toHaveBeenCalledOnce()
		warn.mockRestore()
	})

	test('a server’s rooms are kept under its name; the battle room is one', () => {
		applySnapshot(snapshot)
		const line = (room: string): Delta => ({
			type: 'chat',
			data: {
				seq: 1,
				room,
				from: 'alice',
				text: 'hi',
				kind: 'chat',
				mention: false,
				at: 0,
			},
		})
		applyDelta(line('main'), S)
		applyDelta(line('#battle'), S)
		applyDelta(
			{
				type: 'channel',
				data: {
					name: 'main',
					channel: { name: 'main', members: ['alice'], topicAuthor: null },
				},
			},
			S,
		)
		expect(chat.rooms['server4 main']?.at(-1)?.text).toBe('hi')
		expect(chat.rooms['#battle']?.at(-1)?.text).toBe('hi')
		expect(chat.channels['server4 main']?.members).toEqual(['alice'])
	})

	test('losing the session leaves the running engine and the download alone', () => {
		applySnapshot(snapshot)
		applyDelta({ type: 'engine', data: { state: 'running', pid: 42 } })
		applyDelta({
			type: 'download',
			data: { state: 'running', what: 'BAR', current: 1, total: 4 },
		})

		applyDelta({ type: 'phase', data: null }, S)

		// The game keeps playing when the lobby loses its session, and a reset
		// here would re-enable the button that starts a second one on top of it.
		expect(lobby.engine).toEqual({ state: 'running', pid: 42 })
		expect(lobby.download).toEqual({
			state: 'running',
			what: 'BAR',
			current: 1,
			total: 4,
		})
		// Everything the server told us still goes.
		expect(Object.keys(at().battles)).toHaveLength(0)
		expect(at().me).toBeNull()
	})

	test('a skirmish being set up outlives the session that dropped', () => {
		applySnapshot(snapshot)
		applyDelta({ type: 'skirmish', data: room })
		expect(lobby.skirmish?.battle.title).toBe('Skirmish')

		applyDelta({ type: 'phase', data: null }, S)

		// Somebody's half-built game is not the server's to take away, and a
		// dropped connection is the moment they most want to keep playing.
		expect(lobby.skirmish?.battle.title).toBe('Skirmish')
		expect(lobby.skirmish?.my.scriptTags).toEqual({
			'game/modoptions/ranked_game': '0',
		})
		// And it goes when it is closed, not before.
		applyDelta({ type: 'skirmish', data: null })
		expect(lobby.skirmish).toBeNull()
	})

	test('the remembered ways in are the machine’s, kept past a lost session', () => {
		const stls = 'STLS on 8200, 46 ms'
		applySnapshot({ ...snapshot, ways: { server4: stls } })
		expect(lobby.ways.server4).toEqual(stls)

		applyDelta({ type: 'phase', data: null }, S)
		expect(lobby.ways.server4).toEqual(stls)

		// Replaced whole: a forgotten way is gone, not left behind.
		applyDelta({ type: 'ways', data: {} })
		expect(lobby.ways).toEqual({})
	})

	test('a change of room empties the battle chat, a repeat does not', () => {
		const room = (id: number) => ({
			type: 'myBattle' as const,
			data: {
				boss: null,
				autoBalance: 'off',
				preset: null,
				id,
				gameHash: 'h',
				scriptTags: {},
				vote: null,
				history: [],
				preReady: false,
				readyOnItsWay: null,
				seatOnItsWay: null,
				heldUntilMs: null,
			},
		})
		const said = (seq: number, text: string): Delta => ({
			type: 'chat',
			data: {
				seq,
				room: '#battle',
				from: 'host',
				text,
				kind: 'announcement',
				mention: false,
				at: 0,
			},
		})
		applySnapshot(snapshot)
		applyDelta(room(5), S)
		applyDelta(said(1, 'welcome to 5'), S)
		applyDelta(room(5), S)
		expect(chat.rooms['#battle']!.map((l) => l.text)).toEqual(['welcome to 5'])
		applyDelta(room(6), S)
		applyDelta(said(2, 'welcome to 6'), S)
		expect(chat.rooms['#battle']!.map((l) => l.text)).toEqual(['welcome to 6'])
		applyDelta({ type: 'myBattle', data: null }, S)
		expect(chat.rooms['#battle']).toEqual([])
	})
})
