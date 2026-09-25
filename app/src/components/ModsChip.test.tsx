import { cleanup, render } from '@solidjs/testing-library'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { MutatorView } from '../ipc/bindings/MutatorView'
import { ModsChip } from './ModsChip'

beforeEach(() => {
	vi.useFakeTimers({ toFake: ['Date'] })
	vi.setSystemTime(new Date('2025-10-10T12:00:00Z'))
})
afterEach(() => {
	cleanup()
	vi.useRealTimers()
})

const sphere: MutatorView = {
	name: 'github-dev-sphere-9108a17078f7.sdd',
	title: 'sphere spawner mod v1.0.0',
	description: null,
	here: true,
	check: null,
	source: 'github:dev/sphere@9108a17078f79d09925edc305ec83bc06c3a7cb3',
	date: '2025-10-05T13:32:07Z',
}

describe('the mods chip', () => {
	const chip = (mods: MutatorView[]) =>
		render(() => <ModsChip mods={mods} />).container.querySelector('.chip')

	test('one mod reads with how recent it is, the commit kept for the tooltip', () => {
		const shown = chip([sphere])
		expect(shown?.textContent).toBe('Mods: sphere spawner mod v1.0.0 · 4d ago')
		expect(shown?.getAttribute('title')).toBe(
			'sphere spawner mod v1.0.0 · 4d ago · dev/sphere @ 9108a17 · committed ' +
				new Date('2025-10-05T13:32:07Z').toLocaleString(),
		)
	})

	test('several read as a count and names, each on its own line in the tooltip', () => {
		const tiny = { ...sphere, title: 'tiny maps v1', source: null, date: null }
		const shown = chip([sphere, tiny, { ...tiny, title: 'fast units v2' }])
		expect(shown?.textContent).toBe(
			'3 mods: sphere spawner mod v1.0.0, tiny maps v1, fast units v2',
		)
		expect(shown?.classList.contains('mods-chip')).toBe(true)
		expect(shown?.getAttribute('title')?.split('\n')).toEqual([
			expect.stringContaining('dev/sphere @ 9108a17'),
			'tiny maps v1',
			'fast units v2',
		])
		expect(shown?.textContent).not.toContain('9108a17')
	})

	test('is not there when the room loads none', () => {
		const shown = render(() => <ModsChip mods={[]} />).container
		expect(shown.querySelector('.chip')).toBeNull()
	})
})
