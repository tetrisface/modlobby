import { MemoryRouter, Route } from '@solidjs/router'
import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { reconcile } from 'solid-js/store'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { BotView } from '../../ipc/bindings/BotView'
import type { ModOption } from '../../ipc/bindings/ModOption'
import { emptyLobby, setLobby } from '../../store/lobby'
import { seedSession } from '../../store/testing'
import { PlayerMenu } from '../../components/PlayerMenu'
import { Room } from '../Room'
import {
	ALONE,
	SERVED,
	battle,
	bot,
	fakeRoom,
	myBattle,
	recordingIo,
	user,
	type Calls,
} from './fixture'
import { status } from './fixture'
import { RoomProvider, type RoomModel } from './model'

/**
 * The room, drawn with nothing behind it.
 *
 * This is what the seam is for: the whole battle room -- teams, seats, the
 * minimap, BAR's settings table -- rendered from plain values and a recording
 * `io`, with no session, no `lobby` store and no Tauri answering anything the
 * room actually asks of its room. What is mocked below is only the calls that
 * take what they need as arguments and would give the same answer either way.
 */
vi.mock('@tauri-apps/api/core', () => ({
	invoke: vi.fn(),
	convertFileSrc: (path: string, scheme: string) => `${scheme}://${path}`,
}))

const OPTIONS: ModOption[] = [
	{ key: 'options', name: 'Options', desc: '', type: 'section', weight: 1 },
	{
		key: 'ranked_game',
		name: 'Ranked',
		desc: 'Counts towards rating',
		type: 'bool',
		section: 'options',
		def: true,
	},
	// Left on its default, so a tab has something unchanged to reveal.
	{
		key: 'deathmode',
		name: 'Deathmode',
		desc: '',
		type: 'bool',
		section: 'options',
		def: false,
	},
]

/** The names on the player rows inside one part of the page. */
function named(container: HTMLElement, within: string): string[] {
	return [...container.querySelectorAll(`${within} .pname`)].map(
		(cell) => cell.textContent ?? '',
	)
}

/** The labels of the buttons inside one part of the page. */
function buttons(container: HTMLElement, within: string): string[] {
	const host = container.querySelector(within)
	if (host === null) return []
	return [...host.querySelectorAll('button')].map((b) => b.textContent ?? '')
}

/** Lets awaited answers reach the component and the DOM. */
async function settle() {
	for (let turn = 0; turn < 8; turn++) await Promise.resolve()
}

beforeEach(() => {
	vi.mocked(invoke).mockImplementation(async (command: string) => {
		switch (command) {
			case 'game_modoptions':
				return OPTIONS
			case 'game_ais':
				return [{ name: 'BARb', desc: '' }]
			case 'game_unit_names':
				return []
			case 'engine_def_tags':
				return { weapon: [] }
			default:
				return null
		}
	})
})

afterEach(() => {
	cleanup()
	vi.clearAllMocks()
	// The engine tests below write the mirrored state; nothing else here does.
	setLobby(reconcile(emptyLobby()))
})

async function open(model: RoomModel) {
	const result = render(() => (
		<MemoryRouter
			root={(props) => (
				<RoomProvider value={model}>{props.children}</RoomProvider>
			)}
		>
			<Route path='/' component={Room} />
		</MemoryRouter>
	))
	await settle()
	return result
}

/** A room of one's own: one human, one AI on the other side, nobody in charge. */
function alone(calls: Calls): RoomModel {
	return fakeRoom({
		caps: ALONE,
		log: '#skirmish',
		battle: () =>
			battle({ title: 'Skirmish', founder: 'me', bots: [bot('BARb')] }),
		my: () => myBattle({ scriptTags: { 'game/modoptions/ranked_game': '0' } }),
		users: () => ({ me: user('me') }),
		io: recordingIo(calls),
	})
}

/** Me on the first team, an AI on the third: the second is an empty gap. */
function gapped(): RoomModel {
	return fakeRoom({
		caps: ALONE,
		battle: () =>
			battle({
				bots: [
					bot('BARb', {
						status: status({ allyTeam: 2, team: 1, sync: 'bot' }),
					}),
				],
			}),
		users: () => ({
			me: user('me', { battleStatus: status({ allyTeam: 0 }) }),
		}),
	})
}

describe('a room whose game cannot be played here', () => {
	/** What macOS gets for a room on the server: watch and talk, nothing else. */
	function watching(): RoomModel {
		return fakeRoom({
			caps: { ...SERVED, plays: false },
			battle: () => battle({ bots: [bot('BARb')] }),
			users: () => ({ me: user('me') }),
		})
	}

	test('offers no seat at all, rather than one that fails', async () => {
		const { container, getByText } = await open(watching())

		expect(container.querySelector('.seat select')).toBeNull()
		expect(getByText(/Spectating/)).toBeTruthy()
	})

	test('and the room itself is still there to read', async () => {
		const { container } = await open(watching())

		// The point of allowing this at all: the teams, the map and the chat are
		// what a lobby is for, and none of them start an engine.
		expect(container.querySelectorAll('.team').length).toBeGreaterThan(0)
		expect(named(container, '.team')).toContain('BARb')
	})
})

describe('choosing a team', () => {
	test('an empty team between two full ones can still be taken', async () => {
		const { container } = await open(gapped())

		const picker = container.querySelector<HTMLSelectElement>('.seat select')
		const offered = [...(picker?.options ?? [])].map((o) => o.textContent)
		// Team 2 is empty and sits below an occupied team 3. It used to be
		// missing entirely: the list was the teams in use plus one past the top.
		// The roster already draws it, so it is joined, not new; only the team
		// past everything drawn is.
		expect(offered).toContain('Join team 2')
		expect(offered).toContain('Join team 3')
		expect(offered.filter((label) => label?.startsWith('New'))).toEqual([
			'New team 4',
		])
	})

	test('the team you are on reads as yours, not as one to join', async () => {
		const { container } = await open(gapped())

		const picker = container.querySelector<HTMLSelectElement>('.seat select')
		const offered = [...(picker?.options ?? [])].map((o) => o.textContent)
		expect(offered[0]).toBe('Team 1')
	})

	/** Me watching; alice and bob on team 1, carol on team 2. */
	function watchingThree(calls: Calls, bots: BotView[]): RoomModel {
		return fakeRoom({
			caps: SERVED,
			battle: () => battle({ members: ['me', 'alice', 'bob', 'carol'], bots }),
			users: () => ({
				me: user('me', { battleStatus: status({ player: false }) }),
				alice: user('alice', { battleStatus: status({ allyTeam: 0 }) }),
				bob: user('bob', { battleStatus: status({ allyTeam: 0, team: 1 }) }),
				carol: user('carol', {
					battleStatus: status({ allyTeam: 1, team: 2 }),
				}),
			}),
			io: recordingIo(calls),
		})
	}

	async function join(container: HTMLElement) {
		const button = [...container.querySelectorAll('.seat button')].find(
			(b) => b.textContent === 'Join',
		)
		expect(button).toBeTruthy()
		fireEvent.click(button as HTMLButtonElement)
		await settle()
	}

	test('Join takes the emptiest side against people', async () => {
		const calls: Calls = []
		const { container } = await open(watchingThree(calls, []))
		await join(container)
		const seat = calls.find(([name]) => name === 'takeSeat')
		expect(seat?.[1][1]).toBe(1)
	})

	test('and the fullest side against AIs, which is where the people are', async () => {
		const calls: Calls = []
		const raptors = bot('RaptorsAI', {
			status: status({ allyTeam: 1, team: 3, sync: 'bot' }),
		})
		const { container } = await open(watchingThree(calls, [raptors]))
		await join(container)
		const seat = calls.find(([name]) => name === 'takeSeat')
		expect(seat?.[1][1]).toBe(0)
	})

	test('in the queue, Join is Leave queue and says where you stand', async () => {
		const calls: Calls = []
		const model = watchingThree(calls, [])
		const { container } = await open(
			fakeRoom({
				...model,
				battle: () =>
					battle({
						members: ['me', 'alice', 'bob', 'carol', 'dave'],
						queue: ['dave', 'me'],
					}),
			}),
		)
		const buttons = [...container.querySelectorAll('.seat button')]
		expect(buttons.some((b) => b.textContent === 'Join')).toBe(false)
		expect(container.querySelector('.seat .muted')?.textContent).toBe(
			'queued 2 of 2',
		)
		const leave = buttons.find((b) => b.textContent === 'Leave queue')
		fireEvent.click(leave as HTMLButtonElement)
		await settle()
		expect(calls).toContainEqual(['sayBattle', ['$leaveq']])
	})

	test('and, with nobody seated yet, the side the AIs are not on', async () => {
		const calls: Calls = []
		const { container } = await open(
			fakeRoom({
				caps: SERVED,
				battle: () => battle({ bots: [bot('ScavengersAI')] }),
				users: () => ({
					me: user('me', { battleStatus: status({ player: false }) }),
				}),
				io: recordingIo(calls),
			}),
		)
		await join(container)
		const seat = calls.find(([name]) => name === 'takeSeat')
		// The AI sits on team 2 (the fixture's default), so team 1 it is.
		expect(seat?.[1][1]).toBe(0)
	})
})

describe('copying an AI', () => {
	test('brings its bonus along, said to the host like any bonus', async () => {
		const calls: Calls = []
		const barb = bot('BARb', { status: status({ allyTeam: 1, handicap: 25 }) })
		const { container } = await open(
			fakeRoom({
				caps: SERVED,
				battle: () => battle({ bots: [barb] }),
				users: () => ({ me: user('me') }),
				io: recordingIo(calls),
			}),
		)
		fireEvent.click(container.querySelector('.bot-clone') as HTMLElement)
		await settle()

		const added = calls.find(([name]) => name === 'addBot')
		expect(added?.[1][1]).toBe('BARb')
		expect(added?.[1][3]).toBe(1)
		const said = calls.find(([name]) => name === 'sayBattle')
		expect(said?.[1][0]).toMatch(/^!force %\S+ bonus 25$/)
	})

	test('and says nothing more when there is no bonus to bring', async () => {
		const calls: Calls = []
		const { container } = await open(
			fakeRoom({
				caps: SERVED,
				battle: () => battle({ bots: [bot('BARb')] }),
				users: () => ({ me: user('me') }),
				io: recordingIo(calls),
			}),
		)
		fireEvent.click(container.querySelector('.bot-clone') as HTMLElement)
		await settle()
		expect(calls.some(([name]) => name === 'addBot')).toBe(true)
		expect(calls.some(([name]) => name === 'sayBattle')).toBe(false)
	})
})

describe('the bonus, from a row to the host', () => {
	/** Me bossing a hosted room, with alice and my BARb in it. */
	function bossed(calls: Calls): RoomModel {
		return fakeRoom({
			caps: SERVED,
			battle: () => battle({ members: ['me', 'alice'], bots: [bot('BARb')] }),
			my: () => myBattle({ boss: 'me' }),
			users: () => ({
				me: user('me'),
				alice: user('alice', {
					battleStatus: status({ allyTeam: 1, team: 2 }),
				}),
			}),
			io: recordingIo(calls),
		})
	}

	/** The menu lives beside the page, as in the app; what it knows of people
	 *  it reads off the lobby store. */
	async function openWithMenu(model: RoomModel) {
		seedSession({
			me: 'me',
			myBattle: myBattle({ boss: 'me' }),
			users: { alice: user('alice') },
		})
		const result = render(() => (
			<MemoryRouter
				root={(props) => (
					<RoomProvider value={model}>
						{props.children}
						<PlayerMenu />
					</RoomProvider>
				)}
			>
				<Route path='/' component={Room} />
			</MemoryRouter>
		))
		await settle()
		return result
	}

	async function setBonusOf(container: HTMLElement, name: string) {
		const cell = [...container.querySelectorAll('.pname')].find(
			(c) => c.textContent === name,
		) as HTMLElement
		expect(cell, `a row for ${name}`).toBeTruthy()
		fireEvent.contextMenu(cell, { clientX: 5, clientY: 5 })
		await settle()
		const entry = [...container.querySelectorAll('.player-menu button')].find(
			(b) => b.textContent === 'Bonus',
		) as HTMLElement
		expect(entry, `a Bonus entry for ${name}`).toBeTruthy()
		fireEvent.mouseDown(entry)
		fireEvent.click(entry)
		await settle()
		const panel = container.querySelector('.bonus-pick') as HTMLFormElement
		expect(panel, 'the panel').toBeTruthy()
		fireEvent.input(panel.querySelector('input[type=number]') as HTMLElement, {
			target: { value: '40' },
		})
		fireEvent.submit(panel)
		await settle()
	}

	test('an AI of ours: one command, said to the host', async () => {
		const calls: Calls = []
		const { container } = await openWithMenu(bossed(calls))
		await setBonusOf(container, 'BARb')
		expect(calls.filter(([name]) => name === 'sayBattle')).toEqual([
			['sayBattle', ['!force %BARb bonus 40']],
		])
	})

	test('a person, by name, as the boss may', async () => {
		const calls: Calls = []
		const { container } = await openWithMenu(bossed(calls))
		await setBonusOf(container, 'alice')
		expect(calls.filter(([name]) => name === 'sayBattle')).toEqual([
			['sayBattle', ['!force alice bonus 40']],
		])
	})
})

describe('a room behind the seam', () => {
	test('draws teams, the map and the settings with no server at all', async () => {
		const { container, getByText, getAllByText } = await open(alone([]))

		expect(getByText('Skirmish')).toBeTruthy()
		// On the card, and again in the content chip's tip.
		const map = getAllByText('Comet Catcher Remake 1.8')
		expect(map.some((shown) => !shown.closest('.content-tip'))).toBe(true)
		// The human on team 1 and the AI on team 2, from `lib/roster`'s arrange().
		expect(container.querySelectorAll('.team').length).toBe(2)
		expect(named(container, '.team')).toEqual(['me', 'BARb'])
	})

	test('a change goes to the room it was given, not to a server', async () => {
		const calls: Calls = []
		const { container } = await open(alone(calls))

		const box = container.querySelector(
			'.opt input[type=checkbox]',
		) as HTMLInputElement
		expect(box).toBeTruthy()
		fireEvent.change(box, { target: { checked: true } })
		await settle()

		expect(calls).toContainEqual(['setOption', ['ranked_game', '1']])
		// Nothing reached Tauri: `set_option` is the room's, not a bare command.
		expect(vi.mocked(invoke).mock.calls.map(([name]) => name)).not.toContain(
			'set_option',
		)
	})

	test('what only a host can offer is not offered where there is no host', async () => {
		const { container, queryByText } = await open(alone([]))

		// The host bar's commands, the vote buttons and the hosting buttons.
		expect(queryByText('Balance')).toBeNull()
		expect(queryByText('Force start')).toBeNull()
		expect(queryByText('Host a public room')).toBeNull()
		expect(queryByText('Private room')).toBeNull()
		expect(queryByText('Leave room')).toBeNull()
		// Nobody to be ready for. Asked of the seat bar rather than of the page,
		// because a player row's sync icon is titled "Not ready" as well.
		expect(buttons(container, '.seat')).not.toContain('Ready up')
	})

	test('the game is started from here when nobody else will start it', async () => {
		const calls: Calls = []
		const { container } = await open(alone(calls))

		expect(buttons(container, '.card-actions')).toEqual(['Start'])
		fireEvent.click(container.querySelector('.card-actions button')!)
		await settle()
		expect(calls).toContainEqual(['launch', []])
	})

	test('a served room asks its host to start, and the boss tells it', async () => {
		const calls: Calls = []
		const { container } = await open(
			fakeRoom({
				caps: SERVED,
				my: () => myBattle({ boss: 'me' }),
				users: () => ({
					me: user('me', { battleStatus: status({ ready: true }) }),
				}),
				io: recordingIo(calls),
			}),
		)

		// Not `launch`: the host runs the engine, we say a `!` line to it. The
		// card holds the plain start; the host bar keeps only the forced one.
		expect(buttons(container, '.card-actions')).toEqual([
			'Start the game',
			'Leave room',
		])
		fireEvent.click(cardButton(container, 'Start the game'))
		await settle()
		expect(calls).toContainEqual(['sayBattle', ['!start']])
		expect(calls).not.toContainEqual(['launch', []])
		expect(buttons(container, '.host-bar')).not.toContain('Start')
		expect(buttons(container, '.host-bar')).toContain('Force start')
		// A listed room is named for other people and run by somebody, so it says
		// whose it is and offers the pen. The skirmish test asserts neither.
		expect(container.textContent).toContain('Host')
		expect(container.querySelector('.room-title button')).toBeTruthy()
	})

	test('a player who is not the boss proposes the start as a vote', async () => {
		const calls: Calls = []
		const { container } = await open(
			fakeRoom({
				caps: SERVED,
				my: () => myBattle({ boss: 'someone' }),
				users: () => ({
					me: user('me', { battleStatus: status({ ready: true }) }),
				}),
				io: recordingIo(calls),
			}),
		)

		expect(buttons(container, '.card-actions')).toEqual([
			'Vote to start',
			'Leave room',
		])
		fireEvent.click(cardButton(container, 'Vote to start'))
		await settle()
		expect(calls).toContainEqual(['sayBattle', ['!cv start']])
	})

	test('a player who is not ready is asked to ready up before the vote is offered', async () => {
		const calls: Calls = []
		const { container } = await open(
			fakeRoom({
				caps: SERVED,
				my: () => myBattle({ boss: 'someone' }),
				io: recordingIo(calls),
			}),
		)

		expect(buttons(container, '.card-actions')).toEqual([
			'Ready up',
			'Leave room',
		])
		fireEvent.click(cardButton(container, 'Ready up'))
		await settle()
		expect(calls).toContainEqual(['setReady', [true]])
		expect(calls).not.toContainEqual(['sayBattle', ['!cv start']])
	})

	test('a spectator is offered no start, since SPADS would refuse one', async () => {
		const { container } = await open(
			fakeRoom({
				caps: SERVED,
				users: () => ({
					me: user('me', { battleStatus: status({ player: false }) }),
				}),
			}),
		)

		expect(buttons(container, '.card-actions')).toEqual(['Leave room'])
	})
})

/** One of the card's buttons, by what it says. */
function cardButton(container: HTMLElement, label: string): HTMLButtonElement {
	const found = [
		...container.querySelectorAll<HTMLButtonElement>('.card-actions button'),
	].find((button) => button.textContent === label)
	if (!found) throw new Error(`no card button reading ${label}`)
	return found
}

/** The Tauri commands that have been asked for, in order. */
const invoked = () => vi.mocked(invoke).mock.calls.map(([name]) => name)

describe('while our own engine is running', () => {
	beforeEach(() => setLobby('engine', { state: 'running', pid: 1 }))

	test('a served room holds every way out, in one column', async () => {
		const { container } = await open(
			fakeRoom({
				caps: SERVED,
				my: () => myBattle({ boss: 'me' }),
				io: recordingIo([]),
			}),
		)
		expect(buttons(container, '.card-actions')).toEqual([
			'Back to game',
			'Leave room',
			'Leave game',
			'Quit',
		])
	})

	test("a room of one's own has no room to leave, and the rest still", async () => {
		const { container } = await open(alone([]))
		expect(buttons(container, '.card-actions')).toEqual([
			'Back to game',
			'Leave game',
			'Quit',
		])
	})

	test('Leave game asks before it acts', async () => {
		const { container } = await open(alone([]))

		fireEvent.click(cardButton(container, 'Leave game'))
		await settle()
		expect(cardButton(container, 'End the game?')).toBeTruthy()
		expect(invoked()).not.toContain('stop_game')

		fireEvent.click(cardButton(container, 'End the game?'))
		await settle()
		expect(invoked().filter((name) => name === 'stop_game')).toHaveLength(1)
		expect(cardButton(container, 'Leave game')).toBeTruthy()
	})

	test('arming Quit disarms Leave game', async () => {
		const { container } = await open(alone([]))

		fireEvent.click(cardButton(container, 'Leave game'))
		fireEvent.click(cardButton(container, 'Quit'))
		await settle()
		expect(cardButton(container, 'Quit everything?')).toBeTruthy()
		expect(cardButton(container, 'Leave game')).toBeTruthy()
		expect(invoked()).not.toContain('stop_game')
		expect(invoked()).not.toContain('quit_all')
	})
})

describe('the setup pane', () => {
	/** The strip's button for a tab; its text is the name and then a badge. */
	function setupTab(container: HTMLElement, name: string): HTMLButtonElement {
		const found = [
			...container.querySelectorAll<HTMLButtonElement>('.setup-tab'),
		].find((button) => button.textContent?.startsWith(name))
		if (!found) throw new Error(`no tab ${name}`)
		return found
	}
	const reveal = (container: HTMLElement) =>
		container.querySelector<HTMLButtonElement>('.setup-reveal')!
	const openGroup = (container: HTMLElement) =>
		container.querySelector('.groups .group.on')?.textContent ?? ''
	const sections = (container: HTMLElement) =>
		[
			...container.querySelectorAll(
				'.setup-detail .setup-section > span:first-child',
			),
		].map((span) => span.textContent)
	const rows = (container: HTMLElement) =>
		container.querySelectorAll('.setup-detail .opt').length

	test('showing the unchanged inside a tab stays on Changed', async () => {
		const { container } = await open(alone([]))
		fireEvent.click(setupTab(container, 'Options'))
		await settle()
		expect(openGroup(container)).toMatch(/^Changed/)
		expect(sections(container)).toEqual(['General'])
		expect(rows(container)).toBe(1)

		fireEvent.click(reveal(container))
		await settle()
		expect(reveal(container).textContent).toBe('Hide unchanged')
		expect(openGroup(container)).toMatch(/^Changed/)
		expect(rows(container)).toBe(2)
		expect(container.querySelector('.slot-grid')).toBeNull()

		fireEvent.click(reveal(container))
		await settle()
		expect(reveal(container).textContent).toBe('Show unchanged')
		expect(rows(container)).toBe(1)
	})

	test('Modding shows its slots as the same rows everywhere, with room for a new tweak', async () => {
		const slotKeys = () =>
			[...container.querySelectorAll('.setup-detail .opt.tweak .k')].map(
				(key) => key.textContent,
			)
		const { container } = await open(alone([]))
		fireEvent.click(setupTab(container, 'Modding'))
		await settle()
		// Nothing is changed, and each kind still offers the slot its next tweak goes to.
		expect(sections(container)).toEqual(['Tweak slots'])
		// In the order BAR runs them: every tweakunits before any tweakdefs.
		expect(slotKeys()).toEqual(['tweakunits', 'tweakdefs'])
		expect(setupTab(container, 'Modding').querySelector('.badge')).toBeNull()

		fireEvent.click(reveal(container))
		await settle()
		expect(sections(container)).toEqual(['Tweak slots'])
		expect(rows(container)).toBe(20)

		const group = [
			...container.querySelectorAll<HTMLButtonElement>('.groups .group'),
		].find((button) => button.textContent?.startsWith('Tweak slots'))!
		fireEvent.click(group)
		await settle()
		expect(slotKeys()).toHaveLength(20)
		expect(container.querySelector('.setup-drafts')?.textContent).toBe('Editor')
	})
})

describe('the faction picker', () => {
	test('each option carries its mark beside a plain label', async () => {
		const { container } = await open(
			fakeRoom({ caps: SERVED, io: recordingIo([]) }),
		)
		const options = [
			...container.querySelectorAll<HTMLOptionElement>(
				'.seat .select.rich option',
			),
		]
		expect(options.map((o) => o.textContent?.trim())).toEqual([
			'Armada',
			'Cortex',
			'Legion',
			'Random',
		])
		for (const option of options)
			expect(option.querySelector('svg.icon.side')).not.toBeNull()
	})
})

describe('readying up', () => {
	test('a ready asked for shows at once, and a second press changes the wish', async () => {
		const calls: Calls = []
		const { container } = await open(
			fakeRoom({
				caps: SERVED,
				// The server still shows us unready; our ready is on its way.
				my: () => myBattle({ readyOnItsWay: true }),
				io: recordingIo(calls),
			}),
		)

		const button = [...container.querySelectorAll('.seat button')].find(
			(b) => b.textContent === 'Ready',
		) as HTMLButtonElement
		expect(button.classList).toContain('pending')
		fireEvent.click(button)
		await settle()
		expect(calls).toContainEqual(['setReady', [false]])
		// The card's button does not ask again while it is on its way.
		expect(cardButton(container, 'Ready up').disabled).toBe(true)
	})

	test('a boss who is not ready is asked to ready up before starting', async () => {
		const calls: Calls = []
		const { container } = await open(
			fakeRoom({
				caps: SERVED,
				my: () => myBattle({ boss: 'me' }),
				io: recordingIo(calls),
			}),
		)

		// SPADS refuses `!start` while a seated player is unready, the boss too.
		expect(buttons(container, '.card-actions')[0]).toBe('Ready up')
		fireEvent.click(cardButton(container, 'Ready up'))
		await settle()
		expect(calls).toContainEqual(['setReady', [true]])
	})

	test('the last one unready is told so, and the card calls louder', async () => {
		const { container } = await open(
			fakeRoom({
				caps: SERVED,
				battle: () => battle({ members: ['me', 'alice'] }),
				users: () => ({
					me: user('me'),
					alice: user('alice', { battleStatus: status({ ready: true }) }),
				}),
			}),
		)

		expect(container.querySelector('.seat')?.textContent).toContain(
			'Everyone else is ready',
		)
		expect(cardButton(container, 'Ready up').classList).toContain('waiting')
	})

	test('queued, a ready can be given in advance for when a seat frees', async () => {
		const calls: Calls = []
		const { container } = await open(
			fakeRoom({
				caps: SERVED,
				battle: () => battle({ members: ['me'], queue: ['me'] }),
				users: () => ({
					me: user('me', { battleStatus: status({ player: false }) }),
				}),
				io: recordingIo(calls),
			}),
		)

		const box = [...container.querySelectorAll('.seat label.check')].find(
			(label) => label.textContent?.includes('Ready when seated'),
		)
		expect(box).toBeTruthy()
		fireEvent.click(box!.querySelector('input')!)
		await settle()
		expect(calls).toContainEqual(['setPreReady', [true]])
	})

	test('during a game, the ready offered is one for the next', async () => {
		const { container } = await open(
			fakeRoom({
				caps: SERVED,
				running: () => ({ id: 1, ip: '', port: 0 }),
			}),
		)

		const seat = container.querySelector('.seat')!
		expect(seat.textContent).toContain('Ready for next game')
		expect(buttons(container, '.seat')).not.toContain('Ready up')
	})
})
