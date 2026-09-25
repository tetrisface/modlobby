import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { afterEach, describe, expect, test, vi } from 'vitest'
import type { BotView } from '../ipc/bindings/BotView'
import { emptyLobby, setLobby } from '../store/lobby'
import { seedSession } from '../store/testing'
import { battle, bot, myBattle, status, user } from '../views/room/fixture'
import {
	PlayerMenu,
	showBotMenu,
	showPlayerMenu,
	showTeamMenu,
	type Moves,
} from './PlayerMenu'

vi.mock('@tauri-apps/api/core', () => ({
	invoke: vi.fn(),
	convertFileSrc: (path: string) => path,
}))

afterEach(cleanup)

const BOT: BotView = {
	name: 'BARb(1)',
	owner: 'me',
	status: {
		ready: true,
		team: 1,
		allyTeam: 1,
		player: true,
		handicap: 25,
		sync: 'bot',
		side: 0,
	},
	teamColour: 0,
	ai: 'BARb',
	options: {},
}

/** A pointer event as the rows hand it over. */
function press(): MouseEvent {
	return new MouseEvent('contextmenu', { clientX: 10, clientY: 20 })
}

function moves(given: number[]): Moves {
	return {
		teams: [0, 1],
		on: 1,
		to: () => Promise.resolve(),
		bonus: (percent) => {
			given.push(percent)
			return Promise.resolve()
		},
		bonusNow: 25,
	}
}

/** A real click: the mouse goes down before it comes up. */
function click(element: Element) {
	fireEvent.mouseDown(element)
	fireEvent.click(element)
}

async function settle() {
	for (let turn = 0; turn < 4; turn++) await Promise.resolve()
}

const labels = (container: HTMLElement) =>
	[...container.querySelectorAll('.player-menu button')].map(
		(b) => b.textContent,
	)

describe('the bonus, from the row menu', () => {
	test('an AI of ours offers it, and the panel sets it', async () => {
		const given: number[] = []
		const { container } = render(() => <PlayerMenu />)
		showBotMenu(BOT, () => Promise.resolve(), press(), {
			moves: moves(given),
		})
		await settle()
		expect(labels(container)).toEqual(['Bonus', 'Move to team 1', 'Remove'])

		click(
			[...container.querySelectorAll('button')].find(
				(b) => b.textContent === 'Bonus',
			) as HTMLElement,
		)
		await settle()
		const panel = container.querySelector('.bonus-pick')
		expect(panel, 'the panel opens in the menu').toBeTruthy()
		const number = panel?.querySelector(
			'input[type=number]',
		) as HTMLInputElement
		expect(number.value).toBe('25')

		fireEvent.input(number, { target: { value: '40' } })
		fireEvent.submit(panel as HTMLFormElement)
		await settle()
		expect(given).toEqual([40])
		expect(container.querySelector('.player-menu')).toBeNull()
	})

	test('Edit opens the AI sheet, and is absent where there is none', async () => {
		let opened = 0
		const { container } = render(() => <PlayerMenu />)
		showBotMenu(BOT, () => Promise.resolve(), press(), {
			moves: moves([]),
			edit: () => {
				opened += 1
			},
		})
		await settle()
		expect(labels(container)).toEqual([
			'Bonus',
			'Edit',
			'Move to team 1',
			'Remove',
		])
		click(
			[...container.querySelectorAll('button')].find(
				(b) => b.textContent === 'Edit',
			) as HTMLElement,
		)
		await settle()
		expect(opened).toBe(1)
		expect(container.querySelector('.player-menu')).toBeNull()
	})

	test('a whole team offers the same menu, and says how big it is', async () => {
		const given: number[] = []
		const cleared: string[] = []
		const { container } = render(() => <PlayerMenu />)
		const added: number[] = []
		showTeamMenu(1, '2 players · 3 AIs', press(), {
			moves: moves(given),
			addAi: () => added.push(1),
			removeBots: () => {
				cleared.push('all')
				return Promise.resolve()
			},
		})
		await settle()
		expect(labels(container)).toEqual([
			'Add AI',
			'Bonus',
			'Move to team 1',
			'Remove the AIs',
		])
		expect(container.querySelector('.player-menu-name')?.textContent).toBe(
			'Team 2',
		)
		expect(container.querySelector('.player-menu-about')?.textContent).toBe(
			'2 players · 3 AIs',
		)

		click(
			[...container.querySelectorAll('button')].find(
				(b) => b.textContent === 'Remove the AIs',
			) as HTMLElement,
		)
		await settle()
		expect(cleared).toEqual(['all'])
		expect(added, 'the entries left alone did not run').toEqual([])
	})

	test('a person offers it too, where the room lets us', async () => {
		const given: number[] = []
		const { container } = render(() => <PlayerMenu />)
		showPlayerMenu('alice', press(), { moves: moves(given) })
		await settle()
		expect(labels(container)).toContain('Bonus')
	})
})

describe('boss and unboss', () => {
	/** A room with us and alice in it, bossed by `boss`. */
	function room(player: boolean, boss: string | null) {
		setLobby(emptyLobby())
		seedSession({
			me: 'me',
			users: {
				me: user('me', { battleStatus: status({ player }) }),
				alice: user('alice'),
			},
			myBattle: myBattle({ boss }),
		})
	}

	async function menuFor(name: string) {
		const { container } = render(() => <PlayerMenu />)
		showPlayerMenu(name, press())
		await settle()
		return container
	}

	test('a seated player may call the vote, and the words go to the room', async () => {
		vi.mocked(invoke).mockClear()
		room(true, null)
		const container = await menuFor('alice')
		expect(labels(container)).toContain('Boss')
		click(
			[...container.querySelectorAll('button')].find(
				(b) => b.textContent === 'Boss',
			) as HTMLElement,
		)
		await settle()
		expect(invoke).toHaveBeenCalledWith('say_battle', { text: '!boss alice' })
		cleanup()
		expect(labels(await menuFor('me')), 'yourself too').toContain('Boss')
	})

	test('a boss is offered Unboss, yourself included', async () => {
		room(false, 'alice,me')
		expect(labels(await menuFor('alice'))).toContain('Unboss')
		cleanup()
		expect(labels(await menuFor('me'))).toContain('Unboss')
	})

	test('a spectator in an unbossed room is offered neither', async () => {
		room(false, null)
		const offered = labels(await menuFor('alice'))
		expect(offered).not.toContain('Boss')
		expect(offered).not.toContain('Unboss')
	})
})

describe('sharing an ID in a running game', () => {
	/** Us walked in on a game alice is in, in a room with `ais` AIs. */
	function room(ais: number, added: boolean, alicePlays = true) {
		setLobby(emptyLobby())
		seedSession({
			me: 'me',
			users: {
				me: user('me', { battleStatus: status({ player: false }) }),
				alice: user('alice', {
					status: { ...user('alice').status, inGame: true },
					battleStatus: status({ player: alicePlays }),
				}),
			},
			battles: {
				1: battle({
					bots: Array.from({ length: ais }, (_, i) => bot(`AI${i}`)),
				}),
			},
			myBattle: myBattle(),
			gameRunning: { id: 1, ip: '', port: 0, added, playingWith: null },
		})
	}

	async function menuFor(name: string) {
		const { container } = render(() => <PlayerMenu />)
		showPlayerMenu(name, press())
		await settle()
		return container
	}

	test('goes as a vote, the one form teiserver passes on', async () => {
		vi.mocked(invoke).mockClear()
		room(1, false)
		const container = await menuFor('alice')
		const entry = [...container.querySelectorAll('button')].find(
			(b) => b.textContent === 'joinas - share their ID',
		) as HTMLElement
		expect(entry.title, 'says what it does on hover').toMatch(/vote/)
		click(entry)
		await settle()
		expect(invoke).toHaveBeenCalledWith('say_battle', {
			text: '!cv joinas alice',
		})
	})

	test('not once SPADS has us in the game', async () => {
		room(1, true)
		expect(labels(await menuFor('alice'))).not.toContain(
			'joinas - share their ID',
		)
	})

	test('not without AIs, where teiserver makes it `joinas spec`', async () => {
		room(0, false)
		expect(labels(await menuFor('alice'))).not.toContain(
			'joinas - share their ID',
		)
	})

	test('not onto someone who is only watching', async () => {
		room(1, false, false)
		expect(labels(await menuFor('alice'))).not.toContain(
			'joinas - share their ID',
		)
	})
})
