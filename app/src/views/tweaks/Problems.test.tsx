import { fireEvent, render } from '@solidjs/testing-library'
import { describe, expect, test, vi } from 'vitest'
import { Outline } from './Outline'
import { Problems } from './Problems'

describe('Problems', () => {
  test('is absent with nothing to say, and jumps to a problem when there is', () => {
    const onGoto = vi.fn()
    const quiet = render(() => (
      <Problems problems={[]} warnings={[]} notes={[]} onGoto={onGoto} />
    ))
    expect(quiet.container.textContent).toBe('')
    quiet.unmount()

    const loud = render(() => (
      <Problems
        problems={[
          {
            line: 3,
            column: 15,
            endLine: 3,
            endColumn: 16,
            message: 'unexpected token `,`',
          },
        ]}
        warnings={[{ line: 30, message: 'no unit named armcomm in this game' }]}
        notes={['This payload contains 2 `_`, which the game reads as `=`.']}
        onGoto={onGoto}
      />
    ))
    expect(loud.getByText('Problems · 3')).toBeTruthy()
    fireEvent.click(loud.getByText('30'))
    expect(onGoto).toHaveBeenCalledWith(30, 1)
    expect(loud.getByText(/unexpected token/)).toBeTruthy()
    expect(loud.getByText(/reads as/)).toBeTruthy()
    fireEvent.click(loud.getByText('3:15'))
    expect(onGoto).toHaveBeenCalledWith(3, 15)
  })
})

describe('Outline', () => {
  const symbols = [
    { name: 'armcom', line: 2 },
    { name: 'corgolt4', line: 40 },
    { name: 'corak', line: 88 },
  ]

  test('lists every name and jumps to its line', () => {
    const onGoto = vi.fn()
    const { getByText } = render(() => (
      <Outline symbols={symbols} onGoto={onGoto} />
    ))
    expect(getByText('Outline · 3')).toBeTruthy()
    fireEvent.click(getByText('corgolt4'))
    expect(onGoto).toHaveBeenCalledWith(40)
  })

  test('is filtered by a name, either case', () => {
    const { getByLabelText, queryByText } = render(() => (
      <Outline symbols={symbols} onGoto={() => {}} />
    ))
    fireEvent.input(getByLabelText('Find a name'), { target: { value: 'COR' } })
    expect(queryByText('armcom')).toBeNull()
    expect(queryByText('corgolt4')).not.toBeNull()
    expect(queryByText('corak')).not.toBeNull()
  })
})
