import { fireEvent, render } from '@solidjs/testing-library'
import { describe, expect, test, vi } from 'vitest'
import { SCRATCH, type Filter, type Item } from '../../lib/tweakspace'
import { DocList } from './DocList'

const scratch: Item = {
	id: SCRATCH,
	title: 'untitled',
	kind: 'defs',
	name: null,
	dirty: false,
	empty: true,
	size: 0,
}

const items: Item[] = [
	scratch,
	{
		id: 'draft:nutty',
		title: 'nutty',
		kind: 'defs',
		name: 'NuttyB v1.52',
		dirty: true,
		empty: false,
		size: 6996,
	},
	{
		id: 'draft:walls',
		title: 'walls',
		kind: 'units',
		name: null,
		dirty: false,
		empty: false,
		size: 120,
	},
]

const filter: Filter = { query: '', sort: 'name' }

const rows = (all: HTMLElement[]) =>
	all.filter((element) => element.classList.contains('doc'))

describe('DocList', () => {
	test('the untitled tweak comes first; a draft says what it is, how long, and whether it is edited', () => {
		const { getAllByRole } = render(() => (
			<DocList
				items={items}
				active='draft:walls'
				filter={filter}
				onSelect={() => {}}
				onFilter={() => {}}
			/>
		))
		const [first, nutty, walls] = rows(getAllByRole('button'))
		expect(first!.textContent).toContain('untitled')
		expect(first!.textContent).toContain('—')
		expect(nutty!.textContent).toContain('6996 lua')
		expect(nutty!.textContent).toContain('NuttyB v1.52')
		expect(nutty!.textContent).toContain('edited')
		expect(walls!.classList.contains('on')).toBe(true)
	})

	test('selecting, searching and sorting go to the caller', () => {
		const onSelect = vi.fn()
		const onFilter = vi.fn()
		const { getByText, getByLabelText } = render(() => (
			<DocList
				items={items}
				active={SCRATCH}
				filter={filter}
				onSelect={onSelect}
				onFilter={onFilter}
			/>
		))
		fireEvent.click(getByText('walls'))
		expect(onSelect).toHaveBeenCalledWith('draft:walls')

		fireEvent.input(getByLabelText('Find'), { target: { value: 'nutty' } })
		expect(onFilter).toHaveBeenCalledWith({ query: 'nutty' })

		fireEvent.change(getByLabelText('Sort'), { target: { value: 'kind' } })
		expect(onFilter).toHaveBeenCalledWith({ sort: 'kind' })
	})

	test('says how to make a draft when there is none', () => {
		const { queryByText, unmount } = render(() => (
			<DocList
				items={[scratch]}
				active={SCRATCH}
				filter={filter}
				onSelect={() => {}}
				onFilter={() => {}}
			/>
		))
		expect(queryByText(/No drafts yet/)).not.toBeNull()
		unmount()
		const some = render(() => (
			<DocList
				items={items}
				active={SCRATCH}
				filter={filter}
				onSelect={() => {}}
				onFilter={() => {}}
			/>
		))
		expect(some.queryByText(/No drafts yet/)).toBeNull()
	})
})
