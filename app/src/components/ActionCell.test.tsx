import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { afterEach, describe, expect, test, vi } from 'vitest'
import { ActionCell, CellButton } from './ActionCell'

afterEach(cleanup)

describe('CellButton', () => {
  test('the tooltip names the button unless a label says more', () => {
    const { getByLabelText } = render(() => (
      <ActionCell>
        <CellButton icon='act-pen' title='Rename' onClick={() => {}} />
        <CellButton
          icon='act-trash'
          title='Delete'
          label='Delete ffa'
          onClick={() => {}}
        />
      </ActionCell>
    ))
    expect(getByLabelText('Rename').title).toBe('Rename')
    expect(getByLabelText('Delete ffa').title).toBe('Delete')
  })

  test('the glyph is drawn and a click reaches the handler', () => {
    const onClick = vi.fn()
    const { getByLabelText } = render(() => (
      <ActionCell filled>
        <CellButton icon='act-pen' title='Rename' onClick={onClick} />
      </ActionCell>
    ))
    const button = getByLabelText('Rename')
    expect(button.querySelector('use')?.getAttribute('href')).toBe('#act-pen')
    expect(button.closest('.act-cell')?.classList.contains('filled')).toBe(true)
    fireEvent.click(button)
    expect(onClick).toHaveBeenCalledTimes(1)
  })
})
