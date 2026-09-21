import { fireEvent, render } from '@solidjs/testing-library'
import { describe, expect, test, vi } from 'vitest'
import type { Found } from '../../lib/tweakspace'
import { SearchPanel } from './SearchPanel'

const found: Found[] = [
	{
		id: 'slot:tweakunits',
		title: 'tweakunits',
		name: 'T3 walls',
		hits: [
			{
				line: 3,
				column: 2,
				length: 7,
				text: '\tarmwall = { health = 12000 },',
			},
		],
		more: 0,
	},
	{
		id: 'slot:tweakdefs1',
		title: 'tweakdefs1',
		name: null,
		hits: [
			{
				line: 12,
				column: 12,
				length: 7,
				text: '\t\tUnitDefs.armwall.health = 1',
			},
		],
		more: 4,
	},
]

function panel(over: Partial<Parameters<typeof SearchPanel>[0]> = {}) {
	const on = { onQuery: vi.fn(), onPick: vi.fn(), onClose: vi.fn() }
	const view = render(() => (
		<SearchPanel
			query='armwall'
			found={found}
			active='slot:tweakdefs1'
			{...on}
			{...over}
		/>
	))
	return { on, ...view }
}

describe('SearchPanel', () => {
	test('lists each tweak with its matches, the match marked on its line', () => {
		const { container, getByText } = panel()
		expect(getByText('6 in 2 tweaks')).toBeTruthy()
		expect(getByText('T3 walls')).toBeTruthy()
		expect(getByText('and 4 more')).toBeTruthy()
		const marks = [...container.querySelectorAll('mark')].map(
			(m) => m.textContent,
		)
		expect(marks).toEqual(['armwall', 'armwall'])
		// The open tweak's matches are marked as its own.
		expect(
			container.querySelector('.tweak-search-doc.on .tweak-search-key')
				?.textContent,
		).toBe('tweakdefs1')
	})

	test('a match goes to its tweak; Enter goes to the first', () => {
		const { on, getByTitle, getByLabelText } = panel()
		fireEvent.click(getByTitle('Line 12'))
		expect(on.onPick).toHaveBeenCalledWith('slot:tweakdefs1', found[1]!.hits[0])
		fireEvent.keyDown(getByLabelText('Search all tweaks'), { key: 'Enter' })
		expect(on.onPick).toHaveBeenLastCalledWith(
			'slot:tweakunits',
			found[0]!.hits[0],
		)
	})

	test('typing searches, Escape closes, and nothing found says so', () => {
		const { on, getByLabelText, getByText } = panel({ found: [] })
		expect(getByText('Nothing in any tweak')).toBeTruthy()
		const input = getByLabelText('Search all tweaks')
		fireEvent.input(input, { target: { value: 'armcom' } })
		expect(on.onQuery).toHaveBeenCalledWith('armcom')
		fireEvent.keyDown(input, { key: 'Escape' })
		expect(on.onClose).toHaveBeenCalled()
	})
})
