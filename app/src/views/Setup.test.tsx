import { MemoryRouter, Route, createMemoryHistory } from '@solidjs/router'
import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { createSignal } from 'solid-js'
import { afterEach, describe, expect, test, vi } from 'vitest'
import type { ModOption } from '../ipc/bindings/ModOption'
import { REST_MS } from '../lib/inspecting'
import { fixtureOptions } from '../lib/setup.fixture'
import { fakeRoom, myBattle } from './room/fixture'
import { RoomProvider, type RoomModel } from './room/model'
import { Setup } from './Setup'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({
	listen: vi.fn(async () => () => {}),
}))

// The editor is Monaco, which the test DOM cannot draw; that it opens is enough.
vi.mock('./tweaks/Tweaks', () => ({ Tweaks: () => <div>the editor</div> }))

afterEach(() => {
	cleanup()
	vi.mocked(invoke).mockReset()
	vi.useRealTimers()
})

/** A workspace lives as long as its room; each test gets a room of its own. */
let rooms = 0
const ownLog = () => `#setup-${rooms++}`

async function settle() {
	for (let turn = 0; turn < 8; turn++) await Promise.resolve()
}

/** The pane in `room`, on a route since its Mods face navigates. */
function pane(room: RoomModel) {
	// The tables the pane and its tweak editor read at mount, all empty.
	vi.mocked(invoke).mockImplementation(async (command: string) =>
		command === 'engine_def_tags' ? { weapon: [] } : [],
	)
	const { container } = render(() => (
		<MemoryRouter history={createMemoryHistory()}>
			<Route
				path='/*'
				component={() => (
					<RoomProvider value={room}>
						<Setup />
					</RoomProvider>
				)}
			/>
		</MemoryRouter>
	))
	return () => container.querySelector('.pane-tab.on')?.textContent
}

describe('which face the pane opens on', () => {
	test('the options, in a room whose host runs no mods', () => {
		expect(pane(fakeRoom())()).toBe('Options')
	})

	test('the mods, once the host says it runs them', () => {
		const [tags, setTags] = createSignal<Record<string, string>>({})
		const on = pane(fakeRoom({ my: () => myBattle({ scriptTags: tags() }) }))
		expect(on()).toBe('Options')

		// The tags land after the room does.
		setTags({ 'game/mutatorhost': '1' })
		expect(on()).toBe('Mods')
	})

	test('four faces, the settings of games already played last', async () => {
		const on = pane(fakeRoom())
		vi.mocked(invoke).mockImplementation(async (command: string) => {
			if (command === 'game_history' || command === 'game_history_scan')
				return { settings: [], usual: {}, blobs: {}, ignoredPrefixes: [] }
			if (command === 'game_history_values') return { values: {}, blobs: {} }
			return command === 'engine_def_tags' ? { weapon: [] } : []
		})
		const faces = [...document.querySelectorAll('.pane-tab')]
		expect(faces.map((face) => face.textContent)).toEqual([
			'Options',
			'Presets',
			'Mods',
			'History',
		])

		fireEvent.click(faces[3] as Element)
		await settle()
		expect(on()).toBe('History')
		expect(document.querySelector('.past-games')).not.toBeNull()
	})
})

/** Every setting on the All tab, changed first, under the tabs' names. */
async function list(room: RoomModel, options = fixtureOptions()) {
	vi.mocked(invoke).mockImplementation(async (command: string) => {
		if (command === 'game_modoptions') return options
		if (command === 'engine_def_tags') return { weapon: [] }
		return []
	})
	const { container } = render(() => (
		<MemoryRouter history={createMemoryHistory()}>
			<Route
				path='/*'
				component={() => (
					<RoomProvider value={room}>
						<Setup />
					</RoomProvider>
				)}
			/>
		</MemoryRouter>
	))
	await settle()
	// Show: All. The storage a choice is kept in is not there under test.
	fireEvent.click(
		[...container.querySelectorAll('.choice[aria-label="Show"] button')].find(
			(button) => button.textContent === 'All',
		)!,
	)
	await settle()
	return container
}

const headings = (container: HTMLElement) =>
	[...container.querySelectorAll<HTMLElement>('[data-section]')].map(
		(section) => section.dataset.section,
	)

describe('the row under the pointer', () => {
	test('keeps its place while it changes, and the list settles once the pointer moves off', async () => {
		const [tags, setTags] = createSignal<Record<string, string>>({})
		const container = await list(
			fakeRoom({ my: () => myBattle({ scriptTags: tags() }) }),
		)
		expect(headings(container)[0]).toBe('Main')

		// Resting on a row in Cheats, which is far down the list.
		const list_ = container.querySelector('.setup-detail')!
		const row = container.querySelector('[data-key="startmetal"]')!
		fireEvent.mouseOver(row)
		setTags({ 'game/modoptions/startmetal': '2000' })
		await settle()
		// Changed first would lead with Cheats; under the hand it stays put.
		expect(headings(container)[0]).toBe('Main')
		expect(row.classList.contains('changed')).toBe(true)

		fireEvent.mouseLeave(list_)
		expect(headings(container)[0]).toBe('Cheats')
	})
})

describe('the strip under the list', () => {
	const strip = (container: HTMLElement) =>
		container.querySelector('.inspector-name')?.textContent

	test('says what a row is once the pointer has rested on it', async () => {
		const container = await list(fakeRoom({ log: ownLog() }))
		expect(container.querySelector('.inspector-empty')).not.toBeNull()

		vi.useFakeTimers()
		fireEvent.mouseOver(container.querySelector('[data-key="startmetal"]')!)
		expect(strip(container)).toBeUndefined()
		vi.advanceTimersByTime(REST_MS)
		expect(strip(container)).toBe('startmetal')
	})

	test('keeps the row while the pointer goes down to it', async () => {
		const container = await list(fakeRoom({ log: ownLog() }))
		vi.useFakeTimers()
		fireEvent.mouseOver(container.querySelector('[data-key="startmetal"]')!)
		vi.advanceTimersByTime(REST_MS)

		fireEvent.mouseOver(container.querySelector('[data-key="startenergy"]')!)
		fireEvent.mouseLeave(container.querySelector('.setup-detail')!)
		vi.advanceTimersByTime(REST_MS)
		expect(strip(container)).toBe('startmetal')
	})

	test('a row pressed is pinned and marked, and stays once the pointer has gone', async () => {
		const container = await list(fakeRoom({ log: ownLog() }))
		const metal = container.querySelector('[data-key="startmetal"]')!
		fireEvent.click(metal)
		expect(strip(container)).toBe('startmetal')
		expect(metal.classList.contains('on')).toBe(true)

		vi.useFakeTimers()
		fireEvent.mouseOver(container.querySelector('[data-key="startenergy"]')!)
		vi.advanceTimersByTime(REST_MS)
		expect(strip(container)).toBe('startenergy')

		fireEvent.mouseLeave(container.querySelector('.setup-list')!)
		expect(strip(container)).toBe('startmetal')
	})

	test('a control taking the focus picks its row', async () => {
		const container = await list(
			fakeRoom({ log: ownLog(), my: () => myBattle({ boss: 'me' }) }),
		)
		const input = container.querySelector('[data-key="startmetal"] input')!
		fireEvent.focusIn(input)
		expect(strip(container)).toBe('startmetal')
	})

	test('a row that goes out of the list is no longer described', async () => {
		const [tags, setTags] = createSignal<Record<string, string>>({
			'game/modoptions/startmetal': '2000',
		})
		const container = await list(
			fakeRoom({ log: ownLog(), my: () => myBattle({ scriptTags: tags() }) }),
		)
		// Show: Changed, then the one change undone by somebody.
		fireEvent.click(
			[...container.querySelectorAll('.choice[aria-label="Show"] button')].find(
				(button) => button.textContent === 'Changed',
			)!,
		)
		fireEvent.click(container.querySelector('[data-key="startmetal"]')!)
		expect(strip(container)).toBe('startmetal')

		setTags({})
		expect(strip(container)).toBeUndefined()
		expect(container.querySelector('.inspector-empty')).not.toBeNull()
	})

	test('says who changed the row and lights it for a moment', async () => {
		const now = Math.floor(Date.now() / 1000)
		const container = await list(
			fakeRoom({
				log: ownLog(),
				my: () =>
					myBattle({
						scriptTags: { 'game/modoptions/startmetal': '2000' },
						history: [
							{
								seq: 1,
								key: 'startmetal',
								from: '1000',
								to: '2000',
								by: 'Alice',
								at: now,
							},
						],
					}),
			}),
		)
		const metal = container.querySelector('[data-key="startmetal"]')!
		expect(metal.classList.contains('recent')).toBe(true)
		fireEvent.click(metal)
		expect(container.querySelector('.inspector-trace')?.textContent).toBe(
			'Set by Alice just now, was 1000',
		)
	})

	test('a vote on a row is on the row and in the strip', async () => {
		const container = await list(
			fakeRoom({
				log: ownLog(),
				my: () =>
					myBattle({
						vote: {
							command: 'bSet startmetal 5000',
							by: 'Bob',
							proposal: { type: 'setOption', key: 'startmetal', value: '5000' },
							yes: 1,
							yesNeeded: 2,
							no: 0,
							noNeeded: 2,
							remainingSecs: 30,
						},
					}),
			}),
		)
		const metal = container.querySelector('[data-key="startmetal"]')!
		expect(metal.querySelector('.opt-vote')?.textContent).toBe('vote 5000')
		expect(
			container.querySelector('[data-key="startenergy"] .opt-vote'),
		).toBeNull()
		fireEvent.click(metal)
		expect(container.querySelector('.inspector-vote')?.textContent).toContain(
			'Bob called a vote to set 5000',
		)
	})

	test('an option another holds off stays, saying by what', async () => {
		const options: ModOption[] = [
			...fixtureOptions(),
			{
				key: 'holder',
				name: 'Holder',
				type: 'bool',
				def: false,
				section: 'options_main',
				unlock: ['startmetal'],
			},
		]
		const container = await list(fakeRoom({ log: ownLog() }), options)
		const metal = container.querySelector('[data-key="startmetal"]')!
		expect(metal.classList.contains('off')).toBe(true)
		fireEvent.click(metal)
		expect(container.querySelector('.inspector-off')?.textContent).toBe(
			'Not in force while Holder is off',
		)
	})

	test('makes way while a slot is open', async () => {
		const container = await list(fakeRoom({ log: ownLog() }))
		expect(container.querySelector('.inspector')).not.toBeNull()
		fireEvent.click(
			container.querySelector('[data-key="tweakunits"] .slot-open')!,
		)
		expect(container.querySelector('.inspector')).toBeNull()
	})
})

describe('links', () => {
	const options = (): ModOption[] => [
		...fixtureOptions(),
		{
			key: 'startmetal_thread_link',
			name: 'Feedback thread',
			desc: 'Discussion about starting metal.',
			type: 'link',
			section: 'options_cheats',
			link: 'https://discord.com/channels/1/2',
		},
		{
			key: 'cheats_channel_link',
			name: 'Development Discussion',
			desc: 'Cheats discussion.',
			type: 'link',
			section: 'options_cheats',
			link: 'https://discord.com/channels/3/4',
		},
	]
	/** The two declared where BAR declares them: one right after its option. */
	const placed = () => {
		const all = options()
		const thread = all.findIndex((o) => o.key === 'startmetal_thread_link')
		const [link] = all.splice(thread, 1)
		const metal = all.findIndex((o) => o.key === 'startmetal')
		all.splice(metal + 1, 0, link!)
		return all
	}

	test('sits on the option it is about, and is no row of its own', async () => {
		const container = await list(fakeRoom({ log: ownLog() }), placed())
		expect(
			container.querySelector('[data-key="startmetal"] .opt-link'),
		).not.toBeNull()
		expect(
			container.querySelector('[data-key="startenergy"] .opt-link'),
		).toBeNull()
		expect(
			container.querySelector('[data-key="startmetal_thread_link"]'),
		).toBeNull()
	})

	test('about the whole tab, it heads the tab with its prose', async () => {
		const container = await list(fakeRoom({ log: ownLog() }), placed())
		expect(container.querySelector('.setup-intro .opt-link')).toBeNull()
		fireEvent.click(
			[...container.querySelectorAll<HTMLElement>('.setup-tab')].find((tab) =>
				tab.textContent?.startsWith('Cheats'),
			)!,
		)
		const link = container.querySelector<HTMLButtonElement>(
			'.setup-intro .opt-link',
		)
		expect(link?.title).toBe('Cheats discussion\nOpens discord.com')
	})

	test('is told of in the strip when its option is picked', async () => {
		const container = await list(fakeRoom({ log: ownLog() }), placed())
		fireEvent.click(container.querySelector('[data-key="startmetal"]')!)
		expect(container.querySelector('.inspector-link')?.textContent).toBe(
			'Discussion about starting metal · discord.com',
		)
	})
})

describe("a tab's own prose", () => {
	const options = (): ModOption[] => [
		...fixtureOptions(),
		{
			key: 'sub_header',
			name: 'Warning: at your own risk!',
			type: 'subheader',
			section: 'options_cheats',
		},
		{
			key: 'sub_header',
			name: 'No guarantee they work.',
			type: 'subheader',
			section: 'options_cheats',
		},
	]
	const tabNamed = (container: HTMLElement, name: string) =>
		[...container.querySelectorAll<HTMLElement>('.setup-tab')].find((tab) =>
			tab.textContent?.startsWith(name),
		)!

	test('heads its tab, and a warning reads as one', async () => {
		const container = await list(fakeRoom({ log: ownLog() }), options())
		fireEvent.click(tabNamed(container, 'Cheats'))
		const lines = [...container.querySelectorAll('.setup-intro p')]
		expect(lines.map((line) => line.textContent)).toEqual([
			'Warning: at your own risk!',
			'No guarantee they work.',
		])
		expect(lines[0]!.classList.contains('setup-note')).toBe(true)
		expect(lines[1]!.classList.contains('setup-note')).toBe(false)
	})

	test('is not on the tab across all tabs', async () => {
		const container = await list(fakeRoom({ log: ownLog() }), options())
		expect(container.querySelector('.setup-intro')).toBeNull()
	})
})

describe('a search', () => {
	async function search(container: HTMLElement, needle: string) {
		const input = container.querySelector<HTMLInputElement>(
			'.setup-search input',
		)!
		fireEvent.input(input, { target: { value: needle } })
		await settle()
	}

	test('marks what it found, and has the strip under its list too', async () => {
		const container = await list(fakeRoom({ log: ownLog() }))
		await search(container, 'startmetal')
		expect(container.querySelector('.opt mark')?.textContent).toBe('startmetal')
		fireEvent.click(container.querySelector('[data-key="startmetal"]')!)
		expect(container.querySelector('.inspector-name mark')?.textContent).toBe(
			'startmetal',
		)
	})

	test('leaves the overview out and greys Show, saying why', async () => {
		const container = await list(fakeRoom({ log: ownLog() }))
		await search(container, 'startmetal')
		expect(container.querySelector('nav.groups')).toBeNull()
		const show = container.querySelector<HTMLElement>(
			'.choice[aria-label="Show"]',
		)!
		expect(show.title).toBe('A search looks through every setting')
		expect(show.querySelector<HTMLButtonElement>('button')?.disabled).toBe(true)
	})

	test('cleared, is back on the list it came from', async () => {
		const container = await list(fakeRoom({ log: ownLog() }))
		await search(container, 'startmetal')
		await search(container, '')
		expect(container.querySelector('nav.groups')).not.toBeNull()
		expect(container.querySelector('.opt mark')).toBeNull()
	})

	test('says so when it finds nothing', async () => {
		const container = await list(fakeRoom({ log: ownLog() }))
		await search(container, 'nothing like this')
		expect(container.querySelector('.setup-empty')?.textContent).toBe(
			'Nothing matches "nothing like this".',
		)
	})
})

describe('Sort', () => {
	const first = (container: HTMLElement) =>
		container.querySelector<HTMLButtonElement>(
			'.choice[aria-label="Sort"] button',
		)!

	test('says what Changed first comes to when only changes are shown', async () => {
		const container = await list(fakeRoom({ log: ownLog() }))
		expect(first(container).title).toContain('on top')
		fireEvent.click(
			[...container.querySelectorAll('.choice[aria-label="Show"] button')].find(
				(button) => button.textContent === 'Changed',
			)!,
		)
		expect(first(container).title).toBe(
			"Everything shown differs from BAR's default, so this is BAR's own order",
		)
		expect(first(container).disabled).toBe(false)
	})
})
