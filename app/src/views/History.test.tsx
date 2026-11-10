import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { reconcile } from 'solid-js/store'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { GameRow } from '../ipc/bindings/GameRow'
import type { HistoryView } from '../ipc/bindings/HistoryView'
import type { Plan } from '../ipc/bindings/Plan'
import type { Sections } from '../ipc/bindings/Sections'
import { fixtureOptions } from '../lib/setup.fixture'
import { chat } from '../store/chat'
import { emptyLobby, emptyServer, setLobby } from '../store/lobby'
import { onlySession } from '../store/testing'
import { History } from './History'
import { ALONE, fakeRoom } from './room/fixture'
import { RoomProvider } from './room/model'
import { onlineRoom } from './room/online'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({
	listen: vi.fn(async () => () => {}),
}))
vi.mock('./VoteDiff', () => ({
	VoteDiff: () => <div data-testid='vote-diff' />,
}))

/** Lets an awaited answer reach the component and the DOM. */
async function settle() {
	for (let turn = 0; turn < 12; turn++) await Promise.resolve()
}

const game = (over: Partial<GameRow>): GameRow => ({
	started: Math.floor(Date.now() / 1000) - 3600,
	role: 'played',
	gameSeconds: 1200,
	ais: [],
	players: [],
	gameType: 'BAR',
	replay: 'demos/a.sdfz',
	...over,
})

/** Two settings I played: one here, one somewhere else on another map. */
const VIEW: HistoryView = {
	settings: [
		{ key: 'here', map: 'Forge', differs: {}, games: [game({})] },
		{
			key: 'away',
			map: 'Comet Catcher',
			differs: { startmetal: '3000', gone_option: '1' },
			games: [
				game({
					started: Math.floor(Date.now() / 1000) - 7200,
				}),
			],
		},
	],
	usual: { startmetal: '1000' },
	blobs: {},
	ignoredPrefixes: ['date_', 'mapmetadata_'],
}

const NO_PLAN: Plan = {
	lines: [],
	alreadySet: 0,
	startBoxes: [],
	startBoxesUnsent: false,
	bots: 0,
	botOptionsUnsent: false,
}

/** Answers the pane's commands; `values` is the room's side, read once per change. */
function serve(
	view: HistoryView = VIEW,
	values: () => Promise<unknown> = async () => ({
		values: { startmetal: '2000' },
		blobs: {},
	}),
	plan: Plan = NO_PLAN,
) {
	vi.mocked(invoke).mockImplementation(async (command: string) => {
		switch (command) {
			case 'game_history':
			case 'game_history_scan':
				return view
			case 'game_history_values':
				return values()
			case 'game_history_apply':
				return plan
			case 'game_history_blob':
				return ''
			default:
				throw new Error(`unexpected ${command}`)
		}
	})
}

function calls(command: string) {
	return vi
		.mocked(invoke)
		.mock.calls.filter(([name]) => name === command)
		.map(([, args]) => args)
}

/**
 * A room run by `Host[EU6][012]` on Forge, with starting metal at 2000,
 * under the SPADS preset `preset` once the host has said.
 */
function enterRoom(preset: string | null = null) {
	const next = emptyServer()
	next.me = 'me'
	next.battles[7] = {
		id: 7,
		founder: 'Host[EU6][012]',
		ip: '',
		port: 0,
		maxPlayers: 16,
		passworded: false,
		locked: false,
		mapHash: '',
		mapName: 'Forge',
		engineName: 'spring',
		engineVersion: '',
		title: 'raptors',
		gameName: 'BAR',
		members: ['Host[EU6][012]', 'me'],
		spectatorCount: 0,
		playerCount: 1,
		layout: null,
		bots: [],
		startRects: [],
		queue: [],
	}
	next.myBattle = {
		boss: null,
		autoBalance: 'off',
		preset,
		id: 7,
		gameHash: '',
		scriptTags: { 'game/modoptions/startmetal': '2000' },
		vote: null,
		history: [],
		intent: 'spectate',
		readyOnItsWay: null,
		seatOnItsWay: null,
		heldUntilMs: null,
		settled: true,
		gameRoster: null,
		welcome: [],
	}
	onlySession(next)
}

function pane() {
	return (
		<RoomProvider value={onlineRoom()}>
			<History catalogue={fixtureOptions()} />
		</RoomProvider>
	)
}

const maps = (root: ParentNode) =>
	[...root.querySelectorAll('.past-row .past-map')].map(
		(cell) => cell.firstChild?.textContent,
	)

beforeEach(() => {
	vi.mocked(invoke).mockReset()
	serve()
	enterRoom()
})
afterEach(() => {
	cleanup()
	setLobby(reconcile(emptyLobby()))
})

describe('History', () => {
	test('shows what it holds, then looks for newer replays', async () => {
		const { container } = render(pane)
		await settle()
		const order = vi.mocked(invoke).mock.calls.map(([name]) => name)
		expect(order.indexOf('game_history')).toBeLessThan(
			order.indexOf('game_history_scan'),
		)
		expect(maps(container)).toEqual(['Forge', 'Comet Catcher'])
	})

	test('counts how far each setting is from the room', async () => {
		const { container } = render(pane)
		await settle()
		const deltas = [...container.querySelectorAll('.past-row')].map(
			(row) => row.querySelector('.past-delta')?.textContent,
		)
		// Starting metal on Forge; that, the map and an option gone on Comet.
		expect(deltas).toEqual(['1', '3'])
	})

	test('the list says nothing about what hosted a game', async () => {
		const { container, queryByText } = render(pane)
		await settle()
		expect(queryByText('This room')).toBeNull()
		const marks = [...container.querySelectorAll('.past-marks .doc-tag')].map(
			(mark) => mark.textContent,
		)
		expect(marks).not.toContain('here')
	})

	test('a click compares a setting with the room', async () => {
		const { container, getByText } = render(pane)
		await settle()
		fireEvent.click(getByText('Comet Catcher'))
		await settle()
		const head = container.querySelector('.settings-diff-row.head')
		expect(head?.textContent).toContain('This room')
		expect(container.querySelector('.settings-diff')?.textContent).toContain(
			'3000',
		)
	})

	test('a pinned setting is compared with the next one picked', async () => {
		const { container, getAllByTitle, getByText } = render(pane)
		await settle()
		fireEvent.click(
			getAllByTitle('Compare other settings with this one')[0] as Element,
		)
		fireEvent.click(getByText('Comet Catcher'))
		await settle()
		const head = container.querySelector('.settings-diff-row.head')
		expect(head?.textContent).not.toContain('This room')
		expect(head?.textContent).toContain('Forge')
	})

	test('Load leaves out what the game no longer has', async () => {
		const { getByRole, getByText } = render(pane)
		await settle()
		fireEvent.click(getByText('Comet Catcher'))
		fireEvent.click(getByRole('button', { name: 'Load' }))
		await settle()
		expect(calls('game_history_apply')).toEqual([
			{
				key: 'away',
				sections: {
					map: true,
					modoptions: true,
					battle: true,
					startBoxes: true,
					bots: true,
					reset: false,
				},
				skip: ['gone_option'],
			},
		])
	})

	test('a setting I only came into after it began says so', async () => {
		serve({
			...VIEW,
			settings: [
				{
					key: 'late',
					map: 'Forge',
					differs: {},
					games: [game({ role: 'late' })],
				},
			],
		})
		const { container, getByText } = render(pane)
		await settle()
		fireEvent.click(getByText('Spectated'))
		expect(container.querySelector('.past-marks')?.textContent).toContain(
			'late',
		)
	})

	test('Played is the part shown first; All lights alone and shows the rest', async () => {
		serve({
			...VIEW,
			settings: [
				...VIEW.settings,
				{
					key: 'watched',
					map: 'Tundra',
					differs: {},
					games: [
						game({
							role: 'spectated',
							started: Math.floor(Date.now() / 1000) - 10_800,
						}),
					],
				},
			],
		})
		const { container, getByText } = render(pane)
		await settle()
		expect(maps(container)).toEqual(['Forge', 'Comet Catcher'])
		fireEvent.click(getByText('All'))
		expect(maps(container)).toEqual(['Forge', 'Comet Catcher', 'Tundra'])
		const lit = [...container.querySelectorAll('.choice button.on')].map(
			(button) => button.textContent,
		)
		expect(lit).toEqual(['All'])
	})

	test('a setting shows who else played it, the newest game first', async () => {
		serve({
			...VIEW,
			settings: [
				{
					key: 'here',
					map: 'Forge',
					differs: { startmetal: '3000' },
					games: [
						game({ players: ['Zed', 'Amy'] }),
						game({ players: ['amy', 'Bo'] }),
					],
				},
			],
		})
		const { container } = render(pane)
		await settle()
		const row = container.querySelector('.past-row') as HTMLElement
		expect(row.querySelector('.past-players')?.textContent).toBe('Zed, Amy, Bo')
		// The row's hover lists them one a line, where it named the unusual settings.
		expect(row.title).toBe('Zed\nAmy\nBo')
	})

	test('the parts a Load sends are labelled so, the AIs among them', async () => {
		const { container, getByText } = render(pane)
		await settle()
		expect(
			container.querySelector('.preset-sections .setup-bar-label')?.textContent,
		).toBe('Load')
		const ais = getByText('AIs') as HTMLButtonElement
		expect(ais.disabled).toBe(false)
		expect(ais.classList.contains('on')).toBe(true)
	})

	test('in a skirmish a Load seats the AIs too', async () => {
		const applied: Sections[] = []
		const room = fakeRoom({
			caps: ALONE,
			presets: () => ({
				savePreset: async () => ({ version: 1, presets: [] }),
				applyPreset: async () => NO_PLAN,
				applyHistory: async (_key, sections) => {
					applied.push(sections)
					return NO_PLAN
				},
			}),
		})
		const { getByRole, getByText } = render(() => (
			<RoomProvider value={room}>
				<History catalogue={fixtureOptions()} />
			</RoomProvider>
		))
		await settle()
		fireEvent.click(getByText('Comet Catcher'))
		fireEvent.click(getByRole('button', { name: 'Load' }))
		await settle()
		expect(applied).toEqual([expect.objectContaining({ bots: true })])
	})

	test('the setting other ones are compared with is held by the scales', async () => {
		const { container } = render(pane)
		await settle()
		expect(
			container.querySelectorAll('.past-row use[href="#act-compare"]'),
		).toHaveLength(2)
	})

	test('Reset lobby goes back to the preset the room runs, then loads', async () => {
		enterRoom('coop')
		const { getByRole, getByText } = render(pane)
		await settle()
		fireEvent.click(getByText('Reset lobby'))
		fireEvent.click(getByText('Comet Catcher'))
		fireEvent.click(getByRole('button', { name: 'Load' }))
		await settle()
		expect(calls('game_history_apply')).toEqual([
			expect.objectContaining({
				sections: expect.objectContaining({ reset: true }),
			}),
		])
	})

	test('start boxes the room keeps are said to be kept', async () => {
		serve(VIEW, undefined, { ...NO_PLAN, startBoxesUnsent: true })
		const { getByRole, getByText } = render(pane)
		await settle()
		fireEvent.click(getByText('Comet Catcher'))
		fireEvent.click(getByRole('button', { name: 'Load' }))
		await settle()
		expect(chat.notices.map((notice) => notice.text)).toContain(
			'the room already has start boxes; left as they are',
		)
	})

	test('Reset lobby waits for the host to say which preset it runs', async () => {
		const { getByText } = render(pane)
		await settle()
		const reset = getByText('Reset lobby') as HTMLButtonElement
		expect(reset.disabled).toBe(true)
		expect(reset.title).not.toBe('')
	})

	test('how far each setting is waits for the room to be read', async () => {
		serve(VIEW, () => new Promise(() => {}))
		const { container } = render(pane)
		await settle()
		const deltas = [...container.querySelectorAll('.past-row')].map(
			(row) => row.querySelector('.past-delta')?.textContent,
		)
		expect(deltas).toEqual(['…', '…'])
	})

	test('a preset saved from a setting starts with no name, as a new one does', async () => {
		const { getByText } = render(pane)
		await settle()
		fireEvent.click(getByText('Comet Catcher'))
		fireEvent.click(getByText('Save as preset'))
		const field = document.querySelector(
			'.sheet-card input',
		) as HTMLInputElement
		expect(field.value).toBe('')
	})

	test('a game ending looks at the replays again', async () => {
		render(pane)
		await settle()
		expect(calls('game_history_scan')).toHaveLength(1)
		setLobby('engine', { state: 'running', pid: 1 })
		await settle()
		setLobby('engine', { state: 'exited', code: 0 })
		await settle()
		expect(calls('game_history_scan')).toHaveLength(2)
	})
})
