import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { afterEach, describe, expect, test, vi } from 'vitest'
import { RoomTitle } from './RoomTitle'

afterEach(cleanup)

describe('RoomTitle', () => {
  test('the pen is drawn only when renaming would be taken', () => {
    const { queryByLabelText, unmount } = render(() => (
      <RoomTitle title='Rookies' canRename={false} onRename={() => {}} />
    ))
    expect(queryByLabelText('Rename room')).toBeNull()
    unmount()

    const { getByLabelText } = render(() => (
      <RoomTitle title='Rookies' canRename onRename={() => {}} />
    ))
    expect(getByLabelText('Rename room')).toBeTruthy()
  })

  test('the pen asks for a name, starting from the current one', () => {
    const onRename = vi.fn()
    const { getByLabelText, getByText, container } = render(() => (
      <RoomTitle title='Rookies' canRename onRename={onRename} />
    ))
    fireEvent.click(getByLabelText('Rename room'))

    const field = container.querySelector(
      '.sheet-card input',
    ) as HTMLInputElement
    expect(field.value).toBe('Rookies')

    fireEvent.input(field, { target: { value: 'Veterans' } })
    fireEvent.click(getByText('Rename'))
    expect(onRename).toHaveBeenCalledWith('Veterans')
    expect(container.querySelector('.sheet')).toBeNull()
  })

  test('answering with the name it already has sends nothing', () => {
    const onRename = vi.fn()
    const { getByLabelText, getByText, container } = render(() => (
      <RoomTitle title='Rookies' canRename onRename={onRename} />
    ))
    fireEvent.click(getByLabelText('Rename room'))
    fireEvent.click(getByText('Rename'))
    expect(onRename).not.toHaveBeenCalled()
    expect(container.querySelector('.sheet')).toBeNull()
  })
})
