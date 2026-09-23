import { invoke } from '@tauri-apps/api/core'
import { beforeEach, describe, expect, test, vi } from 'vitest'
import type { WidgetUsage } from '../ipc/bindings/WidgetUsage'
import type { WindowStats } from '../ipc/bindings/WindowStats'
import type { Install } from '../ipc/bindings/Install'
import type { Usage } from '../ipc/bindings/Usage'
import {
	AUDIENCE_ORDER,
	DEFAULT_AUDIENCE,
	WINDOW_ORDER,
	disabledOnly,
	isRepresentative,
	matches,
	ownerOf,
	statsFor,
	unavailableBecause,
} from './widgets'

vi.mock('@tauri-apps/api/core', async () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)

function stats(over: Partial<WindowStats> = {}): WindowStats {
	return {
		rank: 1,
		players: 100,
		players_active: 80,
		players_still_using: 70,
		retention: 0.8,
		still_using: 0.7,
		withheld: false,
		sightings: 300,
		replays: 200,
		days_covered: 30,
		coverage: 1,
		...over,
	}
}

/** Windows under the combined audience, which is what most tests mean. */
function widget(
	windows: Record<string, WindowStats>,
	over: Partial<WidgetUsage> = {},
): WidgetUsage {
	return {
		key: 'widget:gui_ping_wheel',
		resolved: true,
		id: 'gui_ping_wheel',
		name: 'Ping Wheel',
		author: 'Errrrrrr',
		description: '',
		windows: { all: windows },
		install: withheld(),
		image: '',
		images: [],
		first_published: '',
		last_updated: '',
		main: '',
		forks: [],
		...over,
	}
}

function withheld(): Install {
	return {
		kind: 'none',
		url: '',
		archive: false,
		page: '',
		license: '',
		permissive: false,
		reason: 'no known source',
		files: [],
	}
}

function downloadable(): Install {
	return {
		kind: 'github',
		url: 'https://raw.githubusercontent.com/o/r/c6fd104/gui.lua',
		archive: false,
		page: 'https://github.com/o/r',
		license: 'MIT',
		permissive: true,
		reason: '',
		files: [
			{
				path: 'gui.lua',
				content_hash: 'aGFzaA==',
				url: 'https://raw.githubusercontent.com/o/r/c6fd104/gui.lua',
				install_path: 'gui.lua',
			},
		],
	}
}

describe('widget usage store', () => {
	test('the wanted window is used when present', () => {
		const found = statsFor(
			widget({ '30d': stats({ players: 42 }) }),
			'all',
			'30d',
		)
		expect(found?.window).toBe('30d')
		expect(found?.stats.players).toBe(42)
	})

	test('a withheld window falls back to the widest available', () => {
		// The k-anonymity floor applies inside each window, so a widget can be
		// absent from the week and present in the year. Blanking the card would
		// read as "unused" rather than "withheld here".
		const found = statsFor(
			widget({ '90d': stats(), all: stats({ players: 9 }) }),
			'all',
			'7d',
		)
		expect(found?.window).toBe('all')
		expect(found?.stats.players).toBe(9)
	})

	test('the fallback does not depend on key order', () => {
		const insertedNarrowLast = widget({
			all: stats({ players: 1 }),
			'7d': stats({ players: 2 }),
		})
		expect(statsFor(insertedNarrowLast, 'all', '365d')?.window).toBe('all')
	})

	test('a widget with no windows at all has nothing to show', () => {
		expect(statsFor(widget({}), 'all', '30d')).toBeNull()
	})

	test('a partly harvested window is not presented as representative', () => {
		expect(isRepresentative(stats({ coverage: 0.04 }))).toBe(false)
		expect(isRepresentative(stats({ coverage: 1 }))).toBe(true)
	})

	test('install-and-keep is separable from install-and-forget', () => {
		expect(disabledOnly(stats({ players: 100, players_active: 80 }))).toBe(20)
		expect(disabledOnly(stats({ players: 5, players_active: 9 }))).toBe(0)
	})

	test('the window order matches what the pipeline publishes', () => {
		expect([...WINDOW_ORDER]).toEqual(['7d', '30d', '90d', '365d', 'all'])
	})

	test('the audience order matches the battles filter', () => {
		expect([...AUDIENCE_ORDER]).toEqual(['all', 'pve', 'pvp'])
		expect(DEFAULT_AUDIENCE).toBe('all')
	})

	test('an audience the document withheld falls back to the combined view', () => {
		// The split is absent while the pipeline re-reads history under new rules.
		// Blanking every row would read as "nobody plays PvE".
		const found = statsFor(
			widget({ '30d': stats({ players: 7 }) }),
			'pve',
			'30d',
		)
		expect(found?.stats.players).toBe(7)
	})

	test('search matches any field and any order of words', () => {
		const found = widget({ all: stats() })
		expect(matches(found, 'ping errrr')).toBe(true)
		expect(matches(found, 'errrrrrr wheel')).toBe(true)
		expect(matches(found, '')).toBe(true)
		expect(matches(found, 'raptor')).toBe(false)
	})

	test('search does not match on keys the reader never sees', () => {
		// `widget:gui_ping_wheel` is ours, not theirs.
		expect(matches(widget({ all: stats() }), 'gui_ping_wheel')).toBe(false)
	})

	test('a widget with no download always says why', () => {
		expect(unavailableBecause(withheld())).toBe('no known source')
		expect(
			unavailableBecause({
				...withheld(),
				kind: 'github',
				reason: 'licence does not grant redistribution',
			}),
		).toBe('licence does not grant redistribution')
	})

	test('a widget with a download has nothing to explain', () => {
		expect(unavailableBecause(downloadable())).toBeNull()
	})
})

function published(over: Partial<Usage> = {}): Usage {
	return {
		document_version: 4,
		generated_at: '2026-09-16T04:00:00+00:00',
		policy_version: 'pve_widget_harvest_v2_prefix_262144_audience',
		audiences: ['all'],
		windows: ['30d'],
		widgets: [widget({ '30d': stats() })],
		...over,
	}
}

/**
 * The store holds the document in module state, so each test imports its own
 * copy rather than inheriting the last one's.
 */
async function fresh() {
	vi.resetModules()
	return await import('./widgets')
}

describe('asking for the document', () => {
	beforeEach(() => {
		asked.mockReset()
	})

	test('one request, however many callers arrive together', async () => {
		asked.mockResolvedValue(published())
		const store = await fresh()

		await Promise.all([store.loadWidgetUsage(), store.loadWidgetUsage()])
		await store.loadWidgetUsage()

		expect(asked.mock.calls.length).toBe(1)
		expect(store.usage()?.widgets.length).toBe(1)
	})

	test('a failure is not kept, so opening the page again tries again', async () => {
		// Rust holds its own failure for as long as the service asked to be left
		// alone, so asking again costs a call into Rust and no request at all --
		// and the numbers turn up without the app being restarted.
		asked.mockResolvedValueOnce(null)
		const store = await fresh()

		await store.loadWidgetUsage()
		expect(store.usage()).toBeNull()
		expect(store.loaded()).toBe(false)

		asked.mockResolvedValueOnce(published())
		await store.loadWidgetUsage()

		expect(store.usage()?.widgets.length).toBe(1)
		expect(store.loaded()).toBe(true)
	})

	test('a document that did arrive is not asked for twice', async () => {
		asked.mockResolvedValue(published())
		const store = await fresh()

		await store.loadWidgetUsage()
		await store.loadWidgetUsage()

		expect(asked.mock.calls.length).toBe(1)
	})
})

describe('forks and counting', () => {
	test('a document without forks treats the row as its only version', async () => {
		const { forksOf, mainFork } = await import('./widgets')
		const row = widget({ all: stats() }, { main: '', key: 'name:abc' })
		const forks = forksOf(row)
		expect(forks).toHaveLength(1)
		expect(mainFork(row).key).toBe('name:abc')
		expect(mainFork(row).main).toBe(true)
	})

	test('still using and used once count differently, and off follows', async () => {
		const { usingCount, usingShare, notUsing } = await import('./widgets')
		const seen = stats({
			players: 100,
			players_active: 80,
			players_still_using: 60,
			retention: 0.8,
			still_using: 0.6,
		})
		expect([
			usingCount(seen, 'still'),
			usingShare(seen, 'still'),
			notUsing(seen, 'still'),
		]).toEqual([60, 0.6, 40])
		expect([
			usingCount(seen, 'once'),
			usingShare(seen, 'once'),
			notUsing(seen, 'once'),
		]).toEqual([80, 0.8, 20])
	})

	test('a fork author is searchable, so a forker finds the row their version sits under', async () => {
		const row = widget(
			{ all: stats() },
			{
				forks: [
					{
						key: 'discord:.mlov:Dont Stand in Fire',
						kind: 'lineage',
						main: false,
						id: '',
						author: '.mlov',
						description: '',
						install: withheld(),
						image: '',
						images: [],
						first_published: '',
						last_updated: '',
						windows: {},
					},
				],
			},
		)
		expect(matches(row, 'mlov')).toBe(true)
	})
})

describe('whose folder', () => {
	test('the writable one is always modlobby', () => {
		expect(ownerOf('C:/Somewhere/BeyondAllReason/assets', true)).toBe(
			'modlobby',
		)
	})

	test('an install keeping its content in assets is bar-lobby', () => {
		expect(
			ownerOf(
				'C:\\Users\\a\\AppData\\Local\\Programs\\BeyondAllReason\\assets',
				false,
			),
		).toBe('bar-lobby')
		expect(ownerOf('/home/a/.local/share/BeyondAllReason/assets/', false)).toBe(
			'bar-lobby',
		)
	})

	test('any other install is Chobby', () => {
		expect(
			ownerOf(
				'C:\\Users\\a\\AppData\\Local\\Programs\\Beyond-All-Reason\\data',
				false,
			),
		).toBe('Chobby')
		expect(
			ownerOf(
				'/Users/a/Library/Application Support/Beyond-All-Reason-mac',
				false,
			),
		).toBe('Chobby')
	})
})
