import { cleanup, render } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { reconcile } from 'solid-js/store'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { BattleView } from '../ipc/bindings/BattleView'
import type { BotView } from '../ipc/bindings/BotView'
import type { Score } from '../ipc/bindings/Score'
import type { UserView } from '../ipc/bindings/UserView'
import type { Settings } from '../ipc/bindings/Settings'
import { age } from '../lib/age'
import { STAGGER_STEP } from '../lib/stagger'
import { emptyLobby, emptyServer, setLobby } from '../store/lobby'
import { TEST_SERVER, onlySession } from '../store/testing'
import { setSettingsSignal } from '../store/settings'
import { PveScore, QUIET_FOR } from './PveScore'
import { RoomProvider } from './room/model'
import { onlineRoom } from './room/online'

/** The panel over the online room these tests set up in the mirrored state. */
function panel() {
	return (
		<RoomProvider value={onlineRoom()}>
			<PveScore />
		</RoomProvider>
	)
}

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

/** A promise answered from the test, standing in for a slow service. */
function deferred<T>() {
	let resolve!: (value: T) => void
	let reject!: (reason: unknown) => void
	const promise = new Promise<T>((yes, no) => {
		resolve = yes
		reject = no
	})
	return { promise, resolve, reject }
}

/** Lets an awaited answer reach the component and the DOM. */
async function settle() {
	for (let turn = 0; turn < 8; turn++) await Promise.resolve()
}

function bot(ai: string, handicap = 0): BotView {
	return {
		name: ai,
		owner: 'host',
		status: {
			ready: true,
			team: 1,
			allyTeam: 1,
			player: true,
			handicap,
			sync: 'bot',
			side: 0,
		},
		teamColour: 0,
		ai,
		options: {},
	}
}

function room(bots: BotView[]): BattleView {
	return {
		id: 7,
		founder: 'host',
		ip: '',
		port: 0,
		maxPlayers: 16,
		passworded: false,
		locked: false,
		mapHash: '',
		mapName: 'Comet Catcher Remake 1.8',
		engineName: 'spring',
		engineVersion: '',
		title: 'raptors',
		gameName: 'BAR',
		members: ['host', 'me'],
		spectatorCount: 0,
		playerCount: 1,
		layout: null,
		bots,
		startRects: [],
		queue: [],
	}
}

function user(
	name: string,
	held: { player?: boolean; rank?: number; bot?: boolean } = {},
): UserView {
	return {
		name,
		country: '??',
		userId: null,
		lobbyClient: 'modlobby',
		status: {
			inGame: false,
			away: false,
			rank: held.rank ?? 0,
			moderator: false,
			bot: held.bot ?? false,
		},
		battleStatus: {
			ready: false,
			team: 0,
			allyTeam: 0,
			player: held.player ?? false,
			handicap: 0,
			sync: 'synced',
			side: 0,
		},
		battleId: 7,
	}
}

/**
 * A room where this client asks first: the host is a bot and holds no
 * place, so nothing waits. `others` puts people ahead of us.
 */
function enter(bots: BotView[], others: UserView[] = []) {
	const next = emptyServer()
	next.me = 'me'
	next.battles[7] = room(bots)
	next.battles[7].members = ['host', 'me', ...others.map((held) => held.name)]
	next.users.host = user('host', { bot: true })
	next.users.me = user('me', { player: true })
	for (const held of others) next.users[held.name] = held
	next.myBattle = {
		boss: null,
		autoBalance: 'off',
		preset: null,
		id: 7,
		gameHash: '',
		scriptTags: { 'game/modoptions/raptor_endless': '0' },
		vote: null,
		history: [],
		intent: 'spectate',
		readyOnItsWay: null,
		seatOnItsWay: null,
		heldUntilMs: null,
	}
	onlySession(next)
}

const scored: Score = {
	challenge: 21.5,
	percentile: 88,
	winChance: 0.31,
	evidenceGames: 4200,
	bestEffort: false,
	dataUntil: '2026-06-20T22:46:17Z',
	dataSince: '2025-01-02T03:04:05Z',
	aiType: 'Raptors',
	histogram: [
		{ lowerBound: 0, upperBound: 17, games: 100, wins: 90 },
		{ lowerBound: 17, upperBound: 34, games: 300, wins: 30 },
	],
	totalGames: 400,
	matched: {
		status: 'exact',
		method: '',
		similarity: null,
		differenceCount: 0,
		hiddenDifferences: 0,
		diffs: [],
	},
	degradation: null,
	completeness: null,
	players: [
		{
			playerId: 1,
			playerName: 'me',
			setupExperience: { clears: 2, maxDefeated: 50 },
			accomplishments: {
				participation: { gamesPlayed: 10, victories: 4, distinctMapsPlayed: 3 },
				encounters: {
					raptorQueensDefeated: 8,
					scavengerBossesDefeated: 0,
					barbarianAisDefeated: 0,
				},
				personalBests: {
					maxQueensOneVictory: 5,
					maxBossesOneVictory: 0,
					maxBarbarianAisOneVictory: 0,
				},
				challenges: {
					challenge20Clears: 1,
					challenge25Clears: 0,
					challenge30Clears: 0,
					highestChallengeCleared: 24.8,
					clearHistogram: [1, 1],
				},
			},
			awards: { mostKilled: { raptors: 2, scavengers: 0, barbarians: 0 } },
		},
	],
	unresolvedPlayers: ['newbie'],
	setup: { source: 'exact', encounterCount: 20 },
	settingHash: 'abc',
	contractHash: 'def',
}

const asked = vi.mocked(invoke)

describe('PveScore', () => {
	beforeEach(() => {
		// Date too: the pace between asks is measured on the clock.
		vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] })
		asked.mockReset()
	})
	afterEach(() => {
		// Without globals, nothing unmounts the last test's panel for us, and a
		// panel left behind would ask about this test's room too.
		cleanup()
		vi.useRealTimers()
		setLobby(reconcile(emptyLobby()))
		setSettingsSignal(null)
	})

	/** The settings file with the one field this panel reads set. */
	function pveStats(on: boolean) {
		setSettingsSignal({ play: { pveStats: on } } as unknown as Settings)
	}

	test('every figure shows dots in its own slot while the service is asked', () => {
		asked.mockReturnValue(deferred<Score>().promise)
		enter([bot('RaptorsAI')])
		const { container } = render(panel)
		vi.advanceTimersByTime(0)

		expect(asked).toHaveBeenCalledWith('pve_score')
		const slots = container.querySelectorAll('.pve-figure b')
		expect(slots).toHaveLength(3)
		// Still ones: the panel is looked at for minutes, and a cycle there is noise.
		for (const slot of slots)
			expect(slot.querySelector('.thinking.still')).not.toBeNull()
		// The labels stay put around them, and nothing claims anything yet.
		expect(container.textContent).toContain('Challenge')
		expect(container.textContent).toContain('Win')
		expect(container.textContent).not.toContain('unplaced')
		expect(container.textContent).not.toContain('—')
	})

	test('while asking the icon waits with still dots, like the figures', () => {
		asked.mockReturnValue(deferred<Score>().promise)
		enter([bot('RaptorsAI')])
		const { getByLabelText } = render(panel)
		vi.advanceTimersByTime(0)
		const button = getByLabelText('Open PvE stats')
		expect(
			button.querySelector('.pve-miniature .thinking.still'),
		).not.toBeNull()
		expect(button.querySelector('.pve-miniature rect')).toBeNull()
	})

	test('a failed ask leaves the icon as bars at zero, with no tick', async () => {
		asked.mockRejectedValue({ code: 'pve', message: 'pve stats answered 504' })
		const quiet = vi.spyOn(console, 'warn').mockImplementation(() => {})
		enter([bot('RaptorsAI')])
		const { getByLabelText } = render(panel)
		vi.advanceTimersByTime(0)
		await settle()
		const button = getByLabelText('Open PvE stats')
		const bars = button.querySelectorAll('.pve-miniature rect')
		expect(bars).toHaveLength(9)
		const heights = new Set([...bars].map((bar) => bar.getAttribute('height')))
		expect(heights.size).toBe(1)
		expect(button.querySelector('.pve-miniature .at')).toBeNull()
		quiet.mockRestore()
	})

	test('the dots are there before the ask goes out, not a verdict', () => {
		asked.mockReturnValue(deferred<Score>().promise)
		// Someone outranks us, so our ask waits its turn.
		enter([bot('BARb')], [user('boss', { player: true, rank: 8 })])
		const { container } = render(panel)
		expect(asked).not.toHaveBeenCalled()
		expect(container.querySelectorAll('.pve-figure b .thinking')).toHaveLength(
			3,
		)
		expect(container.textContent).not.toContain('unplaced')

		vi.advanceTimersByTime(STAGGER_STEP)
		expect(asked).toHaveBeenCalledTimes(1)
	})

	test('the numbers land in their slots', async () => {
		asked.mockResolvedValue(scored)
		enter([bot('RaptorsAI')])
		const { container } = render(panel)
		vi.advanceTimersByTime(0)
		await settle()

		const slots = [...container.querySelectorAll('.pve-figure b')].map(
			(slot) => slot.textContent,
		)
		expect(slots).toEqual(['21.5/34', '31%', '88%'])
		expect(container.querySelector('.thinking')).toBeNull()
		// How fresh the service's data is rides along as each tooltip's last line.
		const ago = age('2026-06-20T22:46:17Z')
		for (const stat of container.querySelectorAll('.pve-stat'))
			expect(
				stat.getAttribute('title')?.endsWith(`\nData updated ${ago}`),
			).toBe(true)
	})

	test('the PvE row keeps a compact label/value layout', async () => {
		asked.mockResolvedValue(scored)
		enter([bot('RaptorsAI')])
		const { container } = render(panel)
		vi.advanceTimersByTime(0)
		await settle()

		expect(container.querySelectorAll('.pve-stat')).toHaveLength(3)
		expect(container.querySelectorAll('.pve-label')).toHaveLength(3)
		expect(container.textContent).toContain('Challenge')
		expect(container.textContent).toContain('Win')
		expect(container.textContent).toContain('Harder than')
	})

	test('the chart button opens the full panel with the histogram and each record', async () => {
		asked.mockResolvedValue(scored)
		enter([bot('RaptorsAI')])
		const { container, getByLabelText } = render(panel)
		vi.advanceTimersByTime(0)
		await settle()

		// The row's label is the way in; the panel is not drawn until asked for.
		expect(container.textContent).not.toContain('Training games')
		const button = getByLabelText('Open PvE stats') as HTMLButtonElement
		// Its icon is the histogram in little: the two bins merge into one bar,
		// with the tick where the setup sits.
		expect(button.querySelectorAll('.pve-miniature rect')).toHaveLength(1)
		expect(button.querySelector('.pve-miniature .at')).not.toBeNull()
		button.click()

		const sheet = container.querySelector('.pve-stats')!
		expect(sheet.textContent).toContain('Training games')
		expect(sheet.querySelectorAll('.pve-bar')).toHaveLength(2)
		expect(sheet.querySelector('.pve-bar.current')).not.toBeNull()
		expect(sheet.textContent).toContain('your best 24.8')
		expect(sheet.textContent).toContain('2025-01-02 –')
		// Known players first, then the name the service did not know, and the
		// own row marked.
		const names = [...sheet.querySelectorAll('tbody .pve-name')].map(
			(td) => td.textContent,
		)
		expect(names).toEqual(['me', 'newbie'])
		expect(sheet.querySelector('tr.own .pve-name')?.textContent).toBe('me')
		// The default tab sorts by Max here; the figure lands in that column.
		expect(
			sheet.querySelector('th[aria-sort="descending"]')?.textContent,
		).toContain('Max here')
		expect(sheet.querySelector('tr.own')?.textContent).toContain('50')
		// The widget's Diagnostics is a fifth view in the same box.
		const diagnostics = [...sheet.querySelectorAll('[role=tab]')].find((held) =>
			held.textContent?.includes('Diagnostics'),
		) as HTMLButtonElement
		diagnostics.click()
		expect(sheet.querySelector('.pve-diag')?.textContent).toContain(
			'exact setting',
		)
		expect(sheet.querySelector('tbody .pve-name')).toBeNull()

		;(
			sheet.querySelector('header button:last-child') as HTMLButtonElement
		).click()
		expect(container.querySelector('.pve-stats')).toBeNull()
	})

	test('a figure in the row opens the panel too', async () => {
		asked.mockResolvedValue(scored)
		enter([bot('RaptorsAI')])
		const { container } = render(panel)
		vi.advanceTimersByTime(0)
		await settle()

		const win = [...container.querySelectorAll('.pve-stat')].find((stat) =>
			stat.textContent?.includes('Win'),
		) as HTMLElement
		win.click()
		expect(container.querySelector('.pve-stats')).not.toBeNull()
	})

	test('a setup the service cannot place gets the same blank as any other missing figure', async () => {
		asked.mockResolvedValue({
			...scored,
			challenge: null,
			percentile: null,
			dataUntil: null,
		})
		enter([bot('BARb')])
		const { container } = render(panel)
		vi.advanceTimersByTime(0)
		await settle()

		const slots = [...container.querySelectorAll('.pve-figure b')].map(
			(slot) => slot.textContent,
		)
		expect(slots).toEqual(['—', '31%', '—'])
		// No date from the service, no second line.
		for (const stat of container.querySelectorAll('.pve-stat'))
			expect(stat.getAttribute('title')).not.toContain('\n')
		expect(container.textContent).not.toContain('unplaced')
	})

	test('a change during a slow first ask is asked about once, afterwards', async () => {
		// The first ask is a cold start: the service is still thinking.
		const cold = deferred<Score>()
		asked.mockReturnValueOnce(cold.promise).mockResolvedValue(scored)
		enter([bot('RaptorsAI')])
		render(panel)
		vi.advanceTimersByTime(0)
		expect(asked).toHaveBeenCalledTimes(1)

		// Two settings change while it is out; each settles on its own.
		setLobby(
			'servers',
			TEST_SERVER,
			'myBattle',
			'scriptTags',
			'game/modoptions/raptor_endless',
			'1',
		)
		vi.advanceTimersByTime(QUIET_FOR)
		setLobby(
			'servers',
			TEST_SERVER,
			'myBattle',
			'scriptTags',
			'game/modoptions/raptor_graceperiod',
			'5',
		)
		vi.advanceTimersByTime(QUIET_FOR)
		// Nothing goes out alongside a question still being answered.
		expect(asked).toHaveBeenCalledTimes(1)

		cold.resolve({ ...scored, challenge: 5 })
		await settle()
		vi.advanceTimersByTime(0)
		// One follow-up covers both changes: Rust reads the room afresh.
		expect(asked).toHaveBeenCalledTimes(2)
	})

	test('a room that stops changing is asked about again after it settles', async () => {
		asked.mockResolvedValue(scored)
		enter([bot('ScavengersAI')])
		render(panel)
		vi.advanceTimersByTime(0)
		await settle()
		expect(asked).toHaveBeenCalledTimes(1)

		setLobby(
			'servers',
			TEST_SERVER,
			'myBattle',
			'scriptTags',
			'game/modoptions/scav_endless',
			'1',
		)
		vi.advanceTimersByTime(QUIET_FOR - 1)
		expect(asked).toHaveBeenCalledTimes(1)
		vi.advanceTimersByTime(1)
		// The settle timer hands over to the pace's own zero-length timer, which
		// fake timers push one millisecond out when it is set inside a timer.
		vi.advanceTimersByTime(1)
		expect(asked).toHaveBeenCalledTimes(2)
	})

	test('a failure is said rather than hidden', async () => {
		asked.mockRejectedValue({ code: 'pve', message: 'pve stats answered 504' })
		const quiet = vi.spyOn(console, 'warn').mockImplementation(() => {})
		enter([bot('RaptorsAI')])
		const { container } = render(panel)
		vi.advanceTimersByTime(0)
		await settle()

		const said = container.querySelector('[title="pve stats answered 504"]')
		expect(said?.textContent).toBe('unavailable')
		quiet.mockRestore()
	})

	test('a room without a PvE opponent asks nothing and shows nothing', () => {
		enter([])
		const { container } = render(panel)
		expect(asked).not.toHaveBeenCalled()
		expect(container.querySelector('.pve-score')).toBeNull()
	})

	test('a room the setting turns off shows nothing', async () => {
		asked.mockResolvedValue(null)
		enter([bot('RaptorsAI')])
		const { container } = render(panel)
		vi.advanceTimersByTime(0)
		await settle()
		expect(container.querySelector('.pve-score')).toBeNull()
	})

	test('with the setting off nothing is asked and nothing is drawn', () => {
		pveStats(false)
		asked.mockResolvedValue(scored)
		enter([bot('RaptorsAI')])
		const { container } = render(panel)
		vi.advanceTimersByTime(STAGGER_STEP)
		expect(asked).not.toHaveBeenCalled()
		expect(container.querySelector('.pve-score')).toBeNull()
	})

	test('turning the setting on in a room asks once', async () => {
		pveStats(false)
		asked.mockResolvedValue(scored)
		enter([bot('RaptorsAI')])
		const { container } = render(panel)
		vi.advanceTimersByTime(0)
		expect(asked).not.toHaveBeenCalled()

		pveStats(true)
		vi.advanceTimersByTime(0)
		await settle()
		expect(asked).toHaveBeenCalledTimes(1)
		expect(container.querySelector('.pve-score')).not.toBeNull()
	})
})
