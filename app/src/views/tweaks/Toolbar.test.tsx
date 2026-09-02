import { fireEvent, render } from '@solidjs/testing-library'
import { describe, expect, test, vi } from 'vitest'
import type { Prepared } from '../../ipc/bindings/Prepared'
import { draftDoc, edit, emptyWorkspace, slotId } from '../../lib/tweakspace'
import { Toolbar } from './Toolbar'

const clean = emptyWorkspace().docs[slotId('tweakdefs1')]!
const typed = edit(clean, 'local a = 1')

const prepared = (fits: boolean): Prepared => ({
  minified: 'local a=1',
  blob: 'bG9jYWwgYT0x',
  command: '!bSet tweakdefs1 bG9jYWwgYT0x',
  gauge: {
    raw: 11,
    minified: 9,
    blob: 12,
    command: fits ? 30 : 20000,
    cap: 16385,
    fits,
  },
})

function handlers() {
  return {
    onFormat: vi.fn(),
    onReset: vi.fn(),
    onSave: vi.fn(),
    onFullscreen: vi.fn(),
    onCompare: vi.fn(),
    comparing: false,
    onCopy: vi.fn(),
    onTarget: vi.fn(),
    onSend: vi.fn(),
    onClear: vi.fn(),
  }
}

describe('Toolbar', () => {
  test('a clean slot cannot be reset; an edited one can, and says so', () => {
    const on = handlers()
    const first = render(() => (
      <Toolbar
        doc={clean}
        prepared={null}
        problem={null}
        busy={false}
        fullscreen={false}
        seated={true}
        target='tweakdefs1'
        {...on}
      />
    ))
    expect((first.getByText('Reset') as HTMLButtonElement).disabled).toBe(true)
    expect(first.queryByText('edited')).toBeNull()
    first.unmount()

    const second = render(() => (
      <Toolbar
        doc={typed}
        prepared={prepared(true)}
        problem={null}
        busy={false}
        fullscreen={false}
        seated={true}
        target='tweakdefs1'
        {...on}
      />
    ))
    expect(second.queryByText('edited')).not.toBeNull()
    fireEvent.click(second.getByText('Reset'))
    expect(on.onReset).toHaveBeenCalled()
    expect(second.getByText('30 / 16385')).toBeTruthy()
  })

  test('sending needs a seat and a payload that fits', () => {
    const on = handlers()
    const spectator = render(() => (
      <Toolbar
        doc={typed}
        prepared={prepared(true)}
        problem={null}
        busy={false}
        fullscreen={false}
        seated={false}
        target='tweakdefs1'
        {...on}
      />
    ))
    expect(
      (spectator.getByText('Send !bSet') as HTMLButtonElement).disabled,
    ).toBe(true)
    spectator.unmount()

    const tooBig = render(() => (
      <Toolbar
        doc={typed}
        prepared={prepared(false)}
        problem={null}
        busy={false}
        fullscreen={false}
        seated={true}
        target='tweakdefs1'
        {...on}
      />
    ))
    expect((tooBig.getByText('Send !bSet') as HTMLButtonElement).disabled).toBe(
      true,
    )
    fireEvent.click(tooBig.getByText('Clear slot'))
    expect(on.onClear).toHaveBeenCalled()
    tooBig.unmount()

    const ready = render(() => (
      <Toolbar
        doc={typed}
        prepared={prepared(true)}
        problem={null}
        busy={false}
        fullscreen={false}
        seated={true}
        target='tweakdefs1'
        {...on}
      />
    ))
    fireEvent.click(ready.getByText('Send !bSet'))
    expect(on.onSend).toHaveBeenCalledWith(true)
    fireEvent.click(ready.getByText('Call a vote'))
    expect(on.onSend).toHaveBeenCalledWith(false)
    fireEvent.click(ready.getByText('!bSet'))
    expect(on.onCopy).toHaveBeenCalledWith('command')
    fireEvent.click(ready.getByText('Fullscreen'))
    expect(on.onFullscreen).toHaveBeenCalledWith(true)
    fireEvent.click(ready.getByText('Compare'))
    expect(on.onCompare).toHaveBeenCalled()
  })

  test('a syntax error is said in the bar, and copying Lua still works', () => {
    const on = handlers()
    const { getByText, getByTitle } = render(() => (
      <Toolbar
        doc={typed}
        prepared={null}
        problem='Lua: unexpected token'
        busy={false}
        fullscreen={true}
        seated={true}
        target='tweakdefs1'
        {...on}
      />
    ))
    expect(getByTitle('Lua: unexpected token').textContent).toBe(
      'will not load',
    )
    expect((getByText('minified') as HTMLButtonElement).disabled).toBe(true)
    fireEvent.click(getByText('Lua'))
    expect(on.onCopy).toHaveBeenCalledWith('lua')
    fireEvent.click(getByText('Exit fullscreen'))
    expect(on.onFullscreen).toHaveBeenCalledWith(false)
  })

  test('a draft is saved under the typed name, its own name, or the header', () => {
    const on = handlers()
    const draft = draftDoc('walls', '-- T3 walls\n{}')
    const { getByText, getByLabelText, getByRole } = render(() => (
      <Toolbar
        doc={draft}
        prepared={null}
        problem={null}
        busy={false}
        fullscreen={false}
        seated={false}
        target='tweakunits1'
        onDelete={() => {}}
        {...on}
      />
    ))
    fireEvent.click(getByText('Save draft'))
    expect(on.onSave).toHaveBeenLastCalledWith('walls')
    fireEvent.input(getByLabelText('Draft name'), {
      target: { value: 'walls-2' },
    })
    fireEvent.click(getByText('Save draft'))
    expect(on.onSave).toHaveBeenLastCalledWith('walls-2')
    fireEvent.change(getByRole('combobox'), {
      target: { value: 'tweakunits4' },
    })
    expect(on.onTarget).toHaveBeenCalledWith('tweakunits4')
    expect(getByText('Delete draft')).toBeTruthy()

    const named = render(() => (
      <Toolbar
        doc={{ ...typed, name: 'Nutty B' }}
        prepared={null}
        problem={null}
        busy={false}
        fullscreen={false}
        seated={false}
        target='tweakdefs1'
        {...on}
      />
    ))
    fireEvent.click(named.getByText('Save draft'))
    expect(on.onSave).toHaveBeenLastCalledWith('Nutty B')
  })
})
