import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { afterEach, describe, expect, test, vi } from 'vitest'
import type { MapFacts } from '../ipc/bindings/MapFacts'
import { MapPicker } from './MapPicker'

vi.mock('@tauri-apps/api/core', () => ({
	invoke: vi.fn(),
	convertFileSrc: (path: string) => path,
}))

/** What this machine has, as `skirmishOptions` reports it: archive stems. */
vi.mock('../ipc/client', () => ({
	api: {
		skirmishOptions: () =>
			Promise.resolve({ maps: ['bigsteppe_1.2', 'homemade 0.1'] }),
	},
	describeError: (error: unknown) => String(error),
}))

function about(some: Partial<MapFacts>): MapFacts {
	return {
		displayName: '',
		author: '',
		width: 0,
		height: 0,
		playersMin: 0,
		playersMax: 0,
		certified: false,
		terrain: [],
		tags: [],
		...some,
	}
}

// The pictures and the index are Tauri's to answer; everything pure —
// `mapNameFromFile` above all — is the real thing, so a mock cannot go on
// agreeing with a rule the app has since changed.
vi.mock('../lib/maps', async (actual) => ({
	...(await actual<typeof import('../lib/maps')>()),
	CARD_TILE: { width: 180, height: 180 },
	ROW_TILE: { width: 44, height: 28 },
	mapThumb: () => null,
	mapPicture: () => null,
	mapNames: () =>
		Promise.resolve({
			'bigsteppe_1.2': 'Big Steppe 1.2',
			'tinyisle_1.0': 'Tiny Isle 1.0',
		}),
	mapFacts: () =>
		Promise.resolve({
			'Big Steppe 1.2': about({
				displayName: 'Big Steppe',
				author: 'Ann',
				width: 24,
				height: 24,
				playersMin: 8,
				playersMax: 32,
			}),
			'Tiny Isle 1.0': about({
				displayName: 'Tiny Isle',
				width: 8,
				height: 8,
				playersMin: 2,
				playersMax: 4,
			}),
			// Longer on one side than Big Steppe is on either, and less than
			// half the ground: what tells a product apart from a side.
			'Long Valley 1.0': about({
				displayName: 'Long Valley',
				author: 'Bo',
				width: 8,
				height: 40,
				playersMin: 4,
				playersMax: 16,
			}),
		}),
}))

afterEach(cleanup)

const shown = (container: HTMLElement) =>
	[...container.querySelectorAll('.map-card-name')].map(
		(name) => name.textContent,
	)

async function settle() {
	for (let turn = 0; turn < 8; turn++) await Promise.resolve()
}

function open() {
	const picked: [string, boolean][] = []
	const { container } = render(() => (
		<MapPicker
			current=''
			onPick={(name, installed) => picked.push([name, installed])}
			onClose={() => {}}
		/>
	))
	return { container, picked }
}

describe('the map picker', () => {
	test('lists what is published and what is only on disk, and says which', async () => {
		const { container, picked } = open()
		await settle()
		// Published maps by their author's name for them; a map on the disk the
		// index has never heard of by its file name, because nothing else
		// would list it at all -- with the underscores read back as the spaces
		// they were, since `frostycove_v1.13` is a name the engine resolves to
		// nothing and stops the game over.
		expect(shown(container)).toEqual([
			'Big Steppe',
			'homemade 0.1',
			'Long Valley',
			'Tiny Isle',
		])

		const cards = [...container.querySelectorAll('.map-card')]
		const tiny = cards.find((card) => card.textContent?.includes('Tiny Isle'))
		expect(tiny?.classList.contains('absent'), 'not on this machine').toBe(true)
		expect(tiny?.textContent).toContain('not on disk')
		expect(tiny?.textContent).toContain('8 × 8')
		expect(tiny?.textContent).toContain('2–4p')

		// Picking sends the spring name, not the name on the card -- and
		// whether it is here, which is what lets the room fetch what is not.
		fireEvent.click(tiny as HTMLElement)
		const big = cards.find((card) => card.textContent?.includes('Big Steppe'))
		fireEvent.click(big as HTMLElement)
		expect(picked).toEqual([
			['Tiny Isle 1.0', false],
			['Big Steppe 1.2', true],
		])
	})

	test('sorts by size, and On disk hides what we would have to fetch', async () => {
		const { container } = open()
		await settle()

		const sort = container.querySelector('select') as HTMLSelectElement
		fireEvent.change(sort, { target: { value: 'size' } })
		await settle()
		// By area, not by a side: Long Valley is longer than Big Steppe is on
		// either axis and still has less than half the ground.
		// A map the index says nothing about has no size at all, so it sorts
		// last rather than first.
		expect(shown(container)).toEqual([
			'Big Steppe',
			'Long Valley',
			'Tiny Isle',
			'homemade 0.1',
		])

		const onDisk = container.querySelector(
			'.map-held input',
		) as HTMLInputElement
		fireEvent.click(onDisk)
		await settle()
		expect(shown(container)).toEqual(['Big Steppe', 'homemade 0.1'])
	})

	test('the list sorts by a clicked column, and turns round on a second click', async () => {
		const { container } = open()
		await settle()
		fireEvent.click(
			[...container.querySelectorAll('button')].find(
				(b) => b.textContent === 'List',
			) as HTMLElement,
		)
		await settle()

		const header = (label: string) =>
			[...container.querySelectorAll('.map-col')].find((col) =>
				col.textContent?.startsWith(label),
			) as HTMLElement
		const rows = () =>
			[...container.querySelectorAll('.map-cell.name')].map(
				(c) => c.textContent,
			)

		// A number column opens on its largest, which is what it is asked for.
		fireEvent.click(header('Size'))
		await settle()
		expect(rows()).toEqual([
			'Big Steppe',
			'Long Valley',
			'Tiny Isle',
			'homemade 0.1',
		])
		expect(header('Size').getAttribute('aria-sort')).toBe('descending')

		// Clicking it again turns it round -- but the map with no size known
		// stays at the bottom, since unknown is not "smallest".
		fireEvent.click(header('Size'))
		await settle()
		expect(rows()).toEqual([
			'Tiny Isle',
			'Long Valley',
			'Big Steppe',
			'homemade 0.1',
		])
		expect(header('Size').getAttribute('aria-sort')).toBe('ascending')

		// The cells say the sides; the order said the product.
		const size = [...container.querySelectorAll('.map-cell.size')].map(
			(cell) => cell.textContent,
		)
		expect(size).toEqual(['8 × 8', '8 × 40', '24 × 24', ''])
	})

	test('the search reaches the author, not just the name', async () => {
		const { container } = open()
		await settle()
		const search = container.querySelector('.search') as HTMLInputElement
		fireEvent.input(search, { target: { value: 'ann' } })
		await settle()
		expect(shown(container)).toEqual(['Big Steppe'])
	})
})
