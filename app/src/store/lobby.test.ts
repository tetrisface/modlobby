import { reconcile } from 'solid-js/store'
import { afterEach, describe, expect, test } from 'vitest'
import type { BattleView } from '../ipc/bindings/BattleView'
import type { MyBattleView } from '../ipc/bindings/MyBattleView'
import type { Settings } from '../ipc/bindings/Settings'
import { newServer } from '../lib/servers'
import { emptyLobby, myRoom, sessions, setLobby } from './lobby'
import { setSettingsSignal } from './settings'
import { seedSession } from './testing'

const battle = (id: number): BattleView => ({
	id,
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
	gameName: '',
	members: ['host'],
	spectatorCount: 1,
	playerCount: 0,
	layout: null,
	bots: [],
	startRects: [],
	queue: [],
})

const mine = (id: number): MyBattleView => ({
	boss: null,
	autoBalance: 'off',
	preset: null,
	id,
	gameHash: '',
	scriptTags: {},
	vote: null,
	history: [],
	preReady: false,
	readyOnItsWay: null,
	seatOnItsWay: null,
	heldUntilMs: null,
})

afterEach(() => setLobby(reconcile(emptyLobby())))

describe('myRoom', () => {
	test('nothing while not in a room', () => {
		seedSession({ battles: { 5: battle(5) } })
		expect(myRoom()).toBeUndefined()
	})

	test('nothing when the room is no longer listed', () => {
		seedSession({ myBattle: mine(5) })
		expect(myRoom()).toBeUndefined()
	})

	test('the listed room otherwise', () => {
		seedSession({ battles: { 5: battle(5) }, myBattle: mine(5) })
		expect(myRoom()?.title).toBe('Room')
	})
})

describe('sessions', () => {
	afterEach(() => {
		setSettingsSignal(null)
		setLobby(reconcile(emptyLobby()))
	})

	test('come in the order the settings list their servers', () => {
		seedSession({}, 'alpha.example')
		seedSession({}, 'gone.example')
		seedSession({}, 'zulu.example')
		expect(sessions().map(([server]) => server)).toEqual([
			'alpha.example',
			'gone.example',
			'zulu.example',
		])

		setSettingsSignal({
			servers: [newServer('Zulu.example'), newServer('alpha.example')],
		} as Settings)
		expect(sessions().map(([server]) => server)).toEqual([
			'zulu.example',
			'alpha.example',
			'gone.example',
		])
	})
})
