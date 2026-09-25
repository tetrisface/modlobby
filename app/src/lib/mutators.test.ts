import { describe, expect, test } from 'vitest'
import type { MutatorView } from '../ipc/bindings/MutatorView'
import { offers, sameMutator, sourceWords } from './mutators'

const SHA = '9108a17078f79d09925edc305ec83bc06c3a7cb3'

describe('mutators', () => {
	test('a pinned source reads as its repository and short commit', () => {
		expect(sourceWords(`github:dev/sphere@${SHA}`)).toBe('dev/sphere @ 9108a17')
		expect(sourceWords('rapid:something')).toBe('rapid:something')
	})

	test('offers are read from the room’s tags to the first gap', () => {
		expect(
			offers({
				'game/mutatoroffer0': 'sphere-spawner',
				'game/mutatoroffer0source': `github:dev/sphere@${SHA}`,
				'game/mutatoroffer0date': '2025-10-05T13:32:07Z',
				'game/mutatoroffer1': 'tiny maps v1',
				'game/mutatoroffer3': 'after a gap',
				'game/modoptions/startmetal': '1000',
			}),
		).toEqual([
			{
				name: 'sphere-spawner',
				source: `github:dev/sphere@${SHA}`,
				date: '2025-10-05T13:32:07Z',
			},
			{ name: 'tiny maps v1', source: null, date: null },
		])
		expect(offers(undefined)).toEqual([])
	})

	test('a loaded mutator is its offer by commit, or else by name', () => {
		const loaded: MutatorView = {
			name: 'github-dev-sphere-9108a17078f7.sdd',
			title: 'sphere spawner mod v1.0.0',
			description: null,
			here: true,
			check: null,
			source: `github:dev/sphere@${SHA}`,
			date: null,
		}
		const offer = { name: 'sphere-spawner', source: loaded.source, date: null }
		expect(sameMutator(loaded, offer)).toBe(true)
		expect(
			sameMutator(loaded, { ...offer, source: `github:dev/other@${SHA}` }),
		).toBe(false)
		const archive = { ...loaded, name: 'tiny maps v1', source: null }
		expect(
			sameMutator(archive, { name: 'tiny maps v1', source: null, date: null }),
		).toBe(true)
	})
})
