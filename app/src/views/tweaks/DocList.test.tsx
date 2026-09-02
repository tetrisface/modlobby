import { fireEvent, render } from '@solidjs/testing-library'
import { describe, expect, test, vi } from 'vitest'
import type { Item } from '../../lib/tweakspace'
import { DocList } from './DocList'

const items: Item[] = [
  {
    id: 'slot:tweakdefs1',
    title: 'tweakdefs1',
    kind: 'defs',
    name: 'NuttyB v1.52',
    dirty: true,
    stale: false,
    empty: false,
    size: 6996,
    unit: 'blob',
  },
  {
    id: 'slot:tweakunits2',
    title: 'tweakunits2',
    kind: 'units',
    name: null,
    dirty: false,
    stale: true,
    empty: false,
    size: 120,
    unit: 'blob',
  },
  {
    id: 'slot:tweakunits3',
    title: 'tweakunits3',
    kind: 'units',
    name: null,
    dirty: false,
    stale: false,
    empty: true,
    size: 0,
    unit: 'blob',
  },
]

const filter = { query: '', sort: 'order', segment: 'slots' } as const

describe('DocList', () => {
  test('a row says what it is, how big, and whether it differs from the room', () => {
    const { getAllByRole } = render(() => (
      <DocList
        items={items}
        active='slot:tweakunits2'
        filter={filter}
        modified={1}
        onSelect={() => {}}
        onFilter={() => {}}
      />
    ))
    const rows = getAllByRole('button').filter((b) =>
      b.classList.contains('doc'),
    )
    expect(rows).toHaveLength(3)
    expect(rows[0]!.textContent).toContain('tweakdefs1')
    expect(rows[0]!.textContent).toContain('6996 B')
    expect(rows[0]!.textContent).toContain('NuttyB v1.52')
    expect(rows[0]!.textContent).toContain('edited')
    expect(rows[1]!.classList.contains('on')).toBe(true)
    expect(rows[1]!.textContent).toContain('room moved')
    expect(rows[2]!.classList.contains('empty')).toBe(true)
    expect(rows[2]!.textContent).toContain('—')
  })

  test('selecting, searching and switching segment go to the caller', () => {
    const onSelect = vi.fn()
    const onFilter = vi.fn()
    const { getByText, getByLabelText, getByRole } = render(() => (
      <DocList
        items={items}
        active='slot:tweakdefs1'
        filter={filter}
        modified={0}
        onSelect={onSelect}
        onFilter={onFilter}
      />
    ))
    fireEvent.click(getByText('tweakunits2'))
    expect(onSelect).toHaveBeenCalledWith('slot:tweakunits2')

    fireEvent.input(getByLabelText('Find'), { target: { value: 'nutty' } })
    expect(onFilter).toHaveBeenCalledWith({ query: 'nutty' })

    fireEvent.click(getByRole('tab', { name: 'Drafts' }))
    expect(onFilter).toHaveBeenCalledWith({ segment: 'drafts' })

    fireEvent.change(getByLabelText('Sort'), { target: { value: 'name' } })
    expect(onFilter).toHaveBeenCalledWith({ sort: 'name' })
  })

  test('counts unsent edits in the footer, and says so only when there are some', () => {
    const { queryByText, unmount } = render(() => (
      <DocList
        items={items}
        active='slot:tweakdefs1'
        filter={filter}
        modified={2}
        onSelect={() => {}}
        onFilter={() => {}}
      />
    ))
    expect(queryByText(/2 documents with unsent edits/)).not.toBeNull()
    unmount()
    const none = render(() => (
      <DocList
        items={items}
        active='slot:tweakdefs1'
        filter={filter}
        modified={0}
        onSelect={() => {}}
        onFilter={() => {}}
      />
    ))
    expect(none.queryByText(/unsent edits/)).toBeNull()
  })
})
