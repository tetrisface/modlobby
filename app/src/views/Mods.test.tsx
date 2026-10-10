import { MemoryRouter, Route, createMemoryHistory } from '@solidjs/router'
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { MutatorView } from '../ipc/bindings/MutatorView'
import type { Settings } from '../ipc/bindings/Settings'
import { newServer } from '../lib/servers'
import { forgetSet, modSets, rememberSet, setDraft } from '../store/mods'
import { setSettingsSignal } from '../store/settings'
import { emptyLobby, setLobby } from '../store/lobby'
import { seedSession } from '../store/testing'
import { reconcile } from 'solid-js/store'
import { Mods } from './Mods'
import { RoomProvider } from './room/model'
import {
	type Calls,
	fakeRoom,
	myBattle,
	recordingIo,
	status,
	user,
} from './room/fixture'

vi.mock('@tauri-apps/api/core', async () => ({ invoke: vi.fn() }))

const SHA = '9108a17078f79d09925edc305ec83bc06c3a7cb3'
const SPHERE = `github:dev/sphere@${SHA}`

const tags = {
	'game/mutatoroffer0': 'sphere-spawner',
	'game/mutatoroffer0source': SPHERE,
	'game/mutatoroffer1': 'tiny maps v1',
}

const sphere: MutatorView = {
	name: 'github-dev-sphere-9108a17078f7.sdd',
	title: 'sphere spawner mod v1.0.0',
	description: null,
	here: true,
	check: null,
	source: SPHERE,
	date: null,
}

beforeEach(() => {
	setDraft(1, null)
	for (const set of modSets()) forgetSet(set.at)
})
afterEach(() => {
	cleanup()
	setSettingsSignal(null)
	setLobby(reconcile(emptyLobby()))
	vi.mocked(invoke).mockReset()
})

/**
 * The pane in a room whose host runs no mods, on a route, since the way to
 * the list navigates. The mods server is among the settings' servers.
 */
function intro() {
	setSettingsSignal({
		servers: [
			{ ...newServer('server.pve.bar'), builtin: 'mods', name: 'modserver' },
		],
		account: { rememberPassword: false, autoLogin: false },
		chat: { muted: [] },
		// Kept from last time: Empty off would hide a spare mods autohost;
		// PvE would not, and is the reader's to keep.
		battleList: { sort: 'relevance', mode: 'pve', showEmpty: false },
	} as unknown as Settings)
	const room = fakeRoom({
		my: () => myBattle({ scriptTags: {} }),
		check: () => ({ game: null, map: null, mutators: [] }),
	})
	const history = createMemoryHistory()
	const { container } = render(() => (
		<MemoryRouter history={history}>
			<Route
				path='/*'
				component={() => (
					<RoomProvider value={room}>
						<Mods />
					</RoomProvider>
				)}
			/>
		</MemoryRouter>
	))
	return { container, history }
}

function pane(options: {
	boss?: string
	player?: boolean
	loaded?: MutatorView[]
}) {
	const calls: Calls = []
	const room = fakeRoom({
		my: () => myBattle({ scriptTags: tags, boss: options.boss ?? null }),
		users: () => ({
			me: user('me', {
				battleStatus: status({ player: options.player ?? true }),
			}),
		}),
		check: () => ({ game: null, map: null, mutators: options.loaded ?? [] }),
		io: recordingIo(calls),
	})
	const { container } = render(() => (
		<RoomProvider value={room}>
			<Mods />
		</RoomProvider>
	))
	const rows = () =>
		[...container.querySelectorAll('.mod-rows.loaded .mod-row')].map((row) => ({
			name: row.querySelector('.mod-name')?.textContent,
			source: row.querySelector('.mod-source')?.textContent ?? null,
			row,
		}))
	const offers = () => [...container.querySelectorAll('.mod-offers button')]
	/** What was said to the host, in order. */
	const said = () => calls.map(([, [line]]) => line)
	function paste(text: string) {
		const field = container.querySelector<HTMLInputElement>('.mod-add input')!
		fireEvent.input(field, { target: { value: text } })
		fireEvent.keyDown(field, { key: 'Enter' })
	}
	return { container, calls, rows, offers, said, paste }
}

describe('the mods pane', () => {
	test('in a room whose host runs no mods, says where they are and opens the way in', () => {
		// The form behind it reaches for the app; nothing answers here.
		vi.mocked(invoke).mockResolvedValue(null as never)
		const { container } = intro()

		expect(container.querySelector('.mods-intro')?.textContent).toContain(
			"This lobby's host loads none",
		)
		// How a mod is made shows before any login.
		expect(container.querySelector('.mod-tree')).not.toBeNull()
		// No account there yet: the way in is to make one, right here.
		expect(
			container.querySelector('input[autocomplete="username"]'),
		).not.toBeNull()
		expect(container.querySelector('input[type="email"]')).not.toBeNull()
		expect(container.querySelector('.mod-rows')).toBeNull()
	})

	test('logged in to the mods server, offers a room of your own and the list', async () => {
		const asked = vi.mocked(invoke)
		asked.mockImplementation(async (command: string, args?: unknown) => {
			if (command === 'update_settings')
				return (args as { settings: unknown }).settings
			if (command === 'host_public') return 7
			return null
		})
		seedSession({ phase: 'ready', me: 'tetrisface2' }, 'server.pve.bar')
		const { container, history } = intro()

		const way = (text: string) =>
			[...container.querySelectorAll('.mods-ways button')].find((b) =>
				b.textContent?.includes(text),
			) as HTMLButtonElement
		expect(container.textContent).toContain('as tetrisface2')
		expect(container.querySelector('input[type="email"]')).toBeNull()

		// How a mod is made: one to read, and the way to it.
		expect(container.querySelector('.mod-tree')?.textContent).toContain(
			'modinfo.lua',
		)
		fireEvent.click(container.querySelector('.mod-tree .mod-link')!)
		await waitFor(() =>
			expect(asked).toHaveBeenCalledWith('open_url', {
				url: 'https://github.com/tetrisface/sphere-spawner',
			}),
		)

		fireEvent.click(way('Host a lobby'))
		await waitFor(() =>
			expect(asked).toHaveBeenCalledWith('host_public', {
				server: 'server.pve.bar',
			}),
		)
		// Still here: the room view follows the room, this tab does not navigate.
		expect(history.get()).toBe('/')

		fireEvent.click(way('Find a lobby'))
		await waitFor(() => expect(history.get()).toBe('/battles'))
		const saved = asked.mock.calls.find(
			([c]) => c === 'update_settings',
		)?.[1] as { settings: { battleList: object } }
		// The sort, and Empty opened so the spare mods rooms are in the list.
		expect(saved.settings.battleList).toEqual({
			sort: 'modded',
			mode: 'pve',
			showEmpty: true,
		})
	})

	test('shows what is loaded, where it comes from, and what the host offers', () => {
		const { rows, offers, said } = pane({ loaded: [sphere] })
		expect(rows().map((row) => [row.name, row.source])).toEqual([
			['sphere spawner mod v1.0.0', 'dev/sphere9108a17'],
		])
		expect(offers().map((chip) => chip.textContent)).toEqual([
			'sphere-spawner',
			'tiny maps v1',
		])
		expect(offers()[0]?.getAttribute('aria-pressed')).toBe('true')
		expect(said()).toEqual([])
	})

	test('a used-recently chip toggles: clicked while in the list, it comes out', () => {
		const { rows, offers, said } = pane({ boss: 'me', loaded: [sphere] })
		fireEvent.click(offers()[0]!)
		expect(said()).toEqual(['!mutator clear'])
		expect(rows()).toHaveLength(0)
		expect(offers()[0]?.getAttribute('aria-pressed')).toBe('false')
	})

	test('an offer clicked, a page pasted, and a row removed each go to the host', () => {
		const { rows, offers, said, paste } = pane({
			boss: 'me',
			loaded: [sphere],
		})
		fireEvent.click(offers()[1]!)
		paste('https://github.com/dev/tanks/tree/wip')
		// Each builds on what was sent before, though the host has loaded none.
		expect(rows().map((row) => row.name)).toEqual([
			'sphere spawner mod v1.0.0',
			'tiny maps v1',
			'tanks',
		])
		fireEvent.click(rows()[0]!.row.querySelector('button.danger')!)
		expect(said()).toEqual([
			'!mutator set sphere-spawner, tiny maps v1',
			'!mutator set sphere-spawner, tiny maps v1, dev/tanks@wip',
			'!mutator set tiny maps v1, dev/tanks@wip',
		])
	})

	test('what is neither an offer nor a repository is refused in place', () => {
		const { paste, container, said } = pane({ loaded: [sphere] })
		paste('rogue archive')
		expect(container.querySelector('.mod-problem')?.textContent).toContain(
			'Not a GitHub repository',
		)
		expect(said()).toEqual([])
	})

	/** Lays the loaded rows out 40px apart, top to bottom, as jsdom will not. */
	function laidOut(container: HTMLElement) {
		const list = container.querySelector('.mod-rows.loaded')!
		for (const [index, row] of [...list.children].entries())
			row.getBoundingClientRect = () =>
				({ top: index * 40, height: 40, left: 0, width: 300 }) as DOMRect
	}

	test('a row is dragged into load order, the others making room', () => {
		const { rows, offers, said, container } = pane({
			boss: 'me',
			loaded: [sphere],
		})
		fireEvent.click(offers()[1]!)
		laidOut(container)
		fireEvent.pointerDown(rows()[1]!.row, { clientX: 10, clientY: 60 })
		fireEvent.pointerMove(window, { clientX: 10, clientY: 10 })
		expect(rows().map((row) => row.name)).toEqual([
			'tiny maps v1',
			'sphere spawner mod v1.0.0',
		])
		expect(document.querySelector('.drag-ghost')).not.toBeNull()
		expect(said()).toEqual(['!mutator set sphere-spawner, tiny maps v1'])
		fireEvent.pointerUp(window, { clientX: 10, clientY: 10 })
		expect(document.querySelector('.drag-ghost')).toBeNull()
		expect(said().at(-1)).toBe('!mutator set tiny maps v1, sphere-spawner')
	})

	test('a drag let go of, or a press on a control, moves nothing', () => {
		const { rows, offers, said, container } = pane({
			boss: 'me',
			loaded: [sphere],
		})
		fireEvent.click(offers()[1]!)
		laidOut(container)
		fireEvent.pointerDown(rows()[1]!.row, { clientX: 10, clientY: 60 })
		fireEvent.pointerMove(window, { clientX: 10, clientY: 10 })
		fireEvent.pointerCancel(window)
		expect(rows().map((row) => row.name)).toEqual([
			'sphere spawner mod v1.0.0',
			'tiny maps v1',
		])
		fireEvent.pointerDown(rows()[1]!.row.querySelector('button')!, {
			clientX: 10,
			clientY: 60,
		})
		fireEvent.pointerMove(window, { clientX: 10, clientY: 10 })
		fireEvent.pointerUp(window, { clientX: 10, clientY: 10 })
		expect(said()).toEqual(['!mutator set sphere-spawner, tiny maps v1'])
	})

	test('a mod from GitHub links to the commit the room loads, and a press there is no drag', async () => {
		const opened: string[] = []
		vi.mocked(invoke).mockImplementation(async (command, args) => {
			if (command === 'open_url') opened.push((args as { url: string }).url)
		})
		const { rows, offers, said, container } = pane({
			boss: 'me',
			loaded: [sphere],
		})
		fireEvent.click(offers()[1]!)
		laidOut(container)
		const links =
			rows()[0]!.row.querySelectorAll<HTMLButtonElement>('.mod-link')
		const link = links[1]!
		expect(link.title).toContain(SHA)
		fireEvent.pointerDown(link, { clientX: 10, clientY: 20 })
		fireEvent.pointerMove(window, { clientX: 10, clientY: 70 })
		fireEvent.pointerUp(window, { clientX: 10, clientY: 70 })
		expect(said()).toEqual(['!mutator set sphere-spawner, tiny maps v1'])
		fireEvent.click(link)
		fireEvent.click(links[0]!)
		await waitFor(() =>
			expect(opened).toEqual([
				`https://github.com/dev/sphere/tree/${SHA}`,
				'https://github.com/dev/sphere',
			]),
		)
	})

	/** GitHub, as the app asks it: the newest commit is `sha`. */
	function newestIs(sha: string) {
		vi.mocked(invoke).mockImplementation(async (command) => {
			if (command === 'newest_commit')
				return { sha, date: '2025-11-01T00:00:00Z' }
		})
	}
	const NEWER = 'c'.repeat(40)

	test('a click opens a mod to what it says it does; a drag does not', () => {
		const described = {
			...sphere,
			description: 'spawns aggressive space spheres',
		}
		const { rows, container } = pane({
			loaded: [described, { ...sphere, name: 'x', title: 'x', source: null }],
		})
		const description = () =>
			container.querySelector('.mod-description')?.textContent
		laidOut(container)
		const row = rows()[0]!.row
		fireEvent.pointerDown(row, { clientX: 10, clientY: 20 })
		fireEvent.pointerUp(window, { clientX: 10, clientY: 20 })
		expect(description()).toBe('spawns aggressive space spheres')
		const chevron = row.querySelector<HTMLButtonElement>('.mod-expand')!
		expect(chevron.getAttribute('aria-expanded')).toBe('true')
		fireEvent.click(chevron)
		expect(description()).toBeUndefined()
		fireEvent.pointerDown(row, { clientX: 10, clientY: 20 })
		fireEvent.pointerMove(window, { clientX: 10, clientY: 70 })
		fireEvent.pointerUp(window, { clientX: 10, clientY: 70 })
		expect(description()).toBeUndefined()
		const quiet = rows().find((entry) => entry.name === 'x')
		expect(quiet?.row.querySelector('.mod-expand')).toBeNull()
	})

	test('a row the host has not loaded yet says how it changes', async () => {
		newestIs(NEWER)
		const { rows, offers } = pane({ boss: 'me', loaded: [sphere] })
		fireEvent.click(offers()[1]!)
		fireEvent.click(rows()[0]!.row.querySelector('[aria-label^="Update"]')!)
		await waitFor(() =>
			expect(rows()[0]?.row.classList.contains('moving')).toBe(true),
		)
		expect(rows()[1]?.row.classList.contains('added')).toBe(true)
		expect(rows()[0]?.row.getAttribute('title')).toContain(
			'Goes to another commit once the host loads it',
		)
		expect(rows()[1]?.row.getAttribute('title')).toContain(
			'Added; waiting for the host to load it',
		)
	})

	test('an update that GitHub says changes nothing leaves the room as it is', async () => {
		newestIs(SHA)
		const { rows, said } = pane({ boss: 'me', loaded: [sphere] })
		const update = rows()[0]!.row.querySelector<HTMLButtonElement>(
			'[aria-label^="Update"]',
		)!
		fireEvent.click(update)
		expect(update.disabled).toBe(true)
		await waitFor(() =>
			expect(rows()[0]?.source).toBe('dev/sphere9108a17· newest'),
		)
		// Nothing to move: the row says so, and the button stays off.
		expect(update.disabled).toBe(true)
		expect(update.title).toBe('At the newest commit already')
		expect(said()).toEqual([])
		expect(rows()[0]?.row.classList.contains('moving')).toBe(false)
	})

	test('a mod from GitHub is moved to the newest commit, or pointed elsewhere', async () => {
		newestIs(NEWER)
		const { rows, said, container } = pane({ boss: 'me', loaded: [sphere] })
		fireEvent.click(rows()[0]!.row.querySelector('[aria-label^="Update"]')!)
		await waitFor(() => expect(said()).toEqual(['!mutator set dev/sphere']))
		expect(rows()[0]?.source).toMatch(/^dev\/sphere9108a17→newest · .+ ago$/)
		expect(rows()[0]?.row.classList.contains('moving')).toBe(true)
		fireEvent.click(rows()[0]!.row.querySelector('[aria-label^="Edit"]')!)
		const field = container.querySelector<HTMLInputElement>('.mod-source-edit')!
		fireEvent.input(field, { target: { value: 'dev/sphere@main' } })
		fireEvent.keyDown(field, { key: 'Enter' })
		expect(rows()[0]?.source).toBe('dev/sphere9108a17→newest of main')
		expect(said().at(-1)).toBe('!mutator set dev/sphere@main')
	})

	test('a boss is taken at their word, and anyone else is the host’s to answer', () => {
		const ADDED = '!mutator set sphere-spawner, tiny maps v1'
		const boss = pane({ boss: 'me', loaded: [sphere] })
		fireEvent.click(boss.offers()[1]!)
		expect(boss.said()).toEqual([ADDED])
		expect(boss.rows()).toHaveLength(2)
		cleanup()
		setDraft(1, null)
		const player = pane({ boss: 'someone', loaded: [sphere] })
		fireEvent.click(player.offers()[1]!)
		expect(player.said()).toEqual([ADDED])
		// A vote may fail: the list stays the room's until the host says otherwise.
		expect(player.rows()).toHaveLength(1)
		cleanup()
		// Likely refused, but a host may know a spectator by name: it is asked.
		const watcher = pane({ player: false, loaded: [sphere] })
		expect(watcher.container.querySelector('.toolbar .note')?.textContent).toBe(
			'Join as a player to change mods',
		)
		fireEvent.click(watcher.rows()[0]!.row.querySelector('button.danger')!)
		expect(watcher.said()).toEqual(['!mutator clear'])
	})

	test('what was sent shows until the host has loaded it', () => {
		const first = pane({ boss: 'me', loaded: [sphere] })
		fireEvent.click(first.offers()[1]!)
		expect(first.rows()[1]?.row.classList.contains('added')).toBe(true)
		cleanup()
		const tiny: MutatorView = {
			...sphere,
			name: 'tiny maps v1',
			title: 'tiny maps v1',
			description: null,
			source: null,
		}
		const after = pane({ boss: 'me', loaded: [sphere, tiny] })
		expect(after.rows().map((row) => row.name)).toEqual([
			'sphere spawner mod v1.0.0',
			'tiny maps v1',
		])
		expect(after.rows()[1]?.row.classList.contains('added')).toBe(false)
	})

	test('a set a room loaded before is loaded again', () => {
		rememberSet([
			sphere,
			{ ...sphere, name: 'tiny maps v1', title: 'tiny maps v1', source: null },
		])
		const { container, said, rows } = pane({ boss: 'me', loaded: [] })
		const set = container.querySelector('.mod-row.set')!
		expect(set.querySelector('.mod-name')?.textContent).toBe(
			'sphere spawner mod v1.0.0 + tiny maps v1',
		)
		fireEvent.click(set.querySelector('[aria-label^="Use"]')!)
		expect(rows().map((row) => row.name)).toEqual([
			'sphere spawner mod v1.0.0',
			'tiny maps v1',
		])
		expect(said()).toEqual(['!mutator set sphere-spawner, tiny maps v1'])
		fireEvent.click(set.querySelector('button.danger')!)
		expect(container.querySelector('.mod-row.set')).toBeNull()
	})
})
