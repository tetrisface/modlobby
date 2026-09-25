import { cleanup, render } from '@solidjs/testing-library'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { MutatorView } from '../ipc/bindings/MutatorView'
import { MutatorsChip } from './MutatorsChip'

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
	here: true,
	check: null,
	source: 'github:dev/sphere@9108a17078f79d09925edc305ec83bc06c3a7cb3',
	date: '2025-10-05T13:32:07Z',
}

describe('the mutators chip', () => {
	test('names each mutator and how recent it is, and keeps the commit for the tooltip', () => {
		const tiny = { ...sphere, title: 'tiny maps v1', source: null, date: null }
		const chip = render(() => (
			<MutatorsChip mutators={[sphere, tiny]} />
		)).container.querySelector('.chip')
		expect(chip?.textContent).toBe(
			'Mutators: sphere spawner mod v1.0.0 · 4d ago, tiny maps v1',
		)
		expect(chip?.getAttribute('title')).toContain('dev/sphere @ 9108a17')
		expect(chip?.textContent).not.toContain('9108a17')
	})

	test('is not there when the room loads none', () => {
		const shown = render(() => <MutatorsChip mutators={[]} />).container
		expect(shown.querySelector('.chip')).toBeNull()
	})
})
