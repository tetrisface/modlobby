import { cleanup, render } from '@solidjs/testing-library'
import { afterEach, describe, expect, test } from 'vitest'
import type { ContentCheckView } from '../ipc/bindings/ContentCheckView'
import { ContentChip } from './ContentChip'

afterEach(cleanup)

const chip = (check: ContentCheckView) =>
	render(() => (
		<ContentChip
			engine='2026.09.01'
			game='SplinterFaction 0.1.86'
			map='Carrot Mountains v2.0'
			check={check}
		/>
	)).container

const rows = (container: HTMLElement) =>
	[...container.querySelectorAll('.content-tip-row')].map(
		(row) => row.textContent,
	)

describe('the content chip', () => {
	test('is ready, and its tip names each part with its checksum', () => {
		const container = chip({
			game: { verdict: 'same', hash: 1521219441 },
			map: { verdict: 'checking' },
			mutators: [],
		})
		const shown = container.querySelector('.chip.ok')
		expect(shown?.firstChild?.textContent).toBe('Content ready')
		expect(shown?.getAttribute('aria-describedby')).toBe(
			container.querySelector('[role=tooltip]')?.id,
		)
		expect(rows(container)).toEqual([
			'Engine2026.09.01installed',
			'GameSplinterFaction 0.1.86same files as the room · checksum 1521219441',
			'MapCarrot Mountains v2.0checking…',
		])
	})

	test('warns when a part is not the room’s files', () => {
		const container = chip({
			game: { verdict: 'unchecked', why: 'Spring content v1 is not here' },
			map: { verdict: 'differs', ours: 1, room: 2 },
			mutators: [],
		})
		expect(container.querySelector('.chip.warn')?.firstChild?.textContent).toBe(
			'Content differs',
		)
		expect(rows(container).slice(1)).toEqual([
			'GameSplinterFaction 0.1.86installed, not checked: Spring content v1 is not here',
			"MapCarrot Mountains v2.0different files · checksum 1, the room's 2",
		])
	})

	test('lists the mutators after the map, in load order, each held to its checksum', () => {
		const container = chip({
			game: null,
			map: null,
			mutators: [
				{ name: 'Sphere v1', here: true, check: { verdict: 'same', hash: 5 } },
				{
					name: 'Tanks v2',
					here: true,
					check: { verdict: 'differs', ours: 1, room: 2 },
				},
				{ name: 'Unannounced v1', here: true, check: null },
			],
		})
		expect(container.querySelector('.chip.warn')).not.toBeNull()
		expect(rows(container).slice(3)).toEqual([
			'MutatorSphere v1same files as the room · checksum 5',
			"MutatorTanks v2different files · checksum 1, the room's 2",
			'MutatorUnannounced v1installed',
		])
	})
})
