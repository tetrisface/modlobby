import { fireEvent, render } from '@solidjs/testing-library'
import { describe, expect, test } from 'vitest'
import { Composer } from './Composer'

/**
 * The string work behind this lives in `lib/compose` and is tested there. What
 * is tested here is the wiring: that a key reaches the right function and that
 * what comes back lands in the box.
 */
function composer(names: string[] = []) {
  const sent: string[] = []
  const { container } = render(() => (
    <Composer
      placeholder='say'
      names={() => names}
      onSend={(l) => sent.push(l)}
    />
  ))
  const input = container.querySelector('input')!
  const type = (value: string) => {
    fireEvent.input(input, { target: { value } })
  }
  return { input, sent, type }
}

const settled = () =>
  new Promise((resolve) => queueMicrotask(() => resolve(null)))

describe('the box you type a line into', () => {
  test('a line is sent and the box is cleared', () => {
    const { input, sent, type } = composer()
    type('!balance')
    fireEvent.submit(input.form!)
    expect(sent).toEqual(['!balance'])
    expect(input.value).toBe('')
  })

  test('a blank line is not worth sending', () => {
    const { input, sent, type } = composer()
    type('   ')
    fireEvent.submit(input.form!)
    expect(sent).toEqual([])
  })

  test('up walks back through what was sent, down comes forward again', async () => {
    const { input, type } = composer()
    type('first')
    fireEvent.submit(input.form!)
    type('second')
    fireEvent.submit(input.form!)

    fireEvent.keyDown(input, { key: 'ArrowUp' })
    await settled()
    expect(input.value).toBe('second')
    fireEvent.keyDown(input, { key: 'ArrowUp' })
    await settled()
    expect(input.value).toBe('first')
    fireEvent.keyDown(input, { key: 'ArrowDown' })
    await settled()
    expect(input.value).toBe('second')
  })

  test('coming forward past the newest gives back what was half-typed', async () => {
    const { input, type } = composer()
    type('sent')
    fireEvent.submit(input.form!)
    type('half typed')

    fireEvent.keyDown(input, { key: 'ArrowUp' })
    await settled()
    expect(input.value).toBe('sent')
    fireEvent.keyDown(input, { key: 'ArrowDown' })
    await settled()
    expect(input.value).toBe('half typed')
  })

  test('tab finishes a name, and again offers the next one', async () => {
    const { input, type } = composer(['Skywalker', 'sky_bot'])
    type('sky')
    input.setSelectionRange(3, 3)

    fireEvent.keyDown(input, { key: 'Tab' })
    await settled()
    expect(input.value).toBe('sky_bot: ')

    fireEvent.keyDown(input, { key: 'Tab' })
    await settled()
    expect(input.value).toBe('Skywalker: ')
  })

  test('tab with nobody to complete leaves the line alone', async () => {
    const { input, type } = composer(['Skywalker'])
    type('nobody')
    input.setSelectionRange(6, 6)
    fireEvent.keyDown(input, { key: 'Tab' })
    await settled()
    expect(input.value).toBe('nobody')
  })

  test('typing ends the walk back, so the next up starts from the newest', async () => {
    const { input, type } = composer()
    type('one')
    fireEvent.submit(input.form!)
    type('two')
    fireEvent.submit(input.form!)

    fireEvent.keyDown(input, { key: 'ArrowUp' })
    await settled()
    fireEvent.keyDown(input, { key: 'ArrowUp' })
    await settled()
    expect(input.value).toBe('one')

    type('typing again')
    fireEvent.keyDown(input, { key: 'ArrowUp' })
    await settled()
    expect(input.value).toBe('two')
  })
})
