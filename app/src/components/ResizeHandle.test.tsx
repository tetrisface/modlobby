import { render } from '@solidjs/testing-library'
import { describe, expect, test, vi } from 'vitest'
import { ResizeHandle } from './ResizeHandle'

function pointer(type: string, clientX: number) {
  return new MouseEvent(type, { clientX, bubbles: true, button: 0 })
}

describe('ResizeHandle', () => {
  test('reports the start width and every pointer position until release', () => {
    const onMove = vi.fn()
    const onEnd = vi.fn()
    const { getByRole } = render(() => (
      <ResizeHandle onStart={() => 556} onMove={onMove} onEnd={onEnd} />
    ))
    const grip = getByRole('separator')

    grip.dispatchEvent(pointer('pointerdown', 800))
    expect(document.body.classList.contains('resizing')).toBe(true)
    window.dispatchEvent(pointer('pointermove', 700))
    window.dispatchEvent(pointer('pointermove', 650))
    expect(onMove.mock.calls).toEqual([
      [556, 800, 700],
      [556, 800, 650],
    ])
    expect(onEnd).not.toHaveBeenCalled()

    window.dispatchEvent(pointer('pointerup', 650))
    expect(onEnd).toHaveBeenCalledTimes(1)
    expect(document.body.classList.contains('resizing')).toBe(false)

    // Released: the window's pointer is no longer ours.
    window.dispatchEvent(pointer('pointermove', 100))
    expect(onMove).toHaveBeenCalledTimes(2)
  })

  test('arrow keys move it sixteen pixels a press', () => {
    const onMove = vi.fn()
    const onEnd = vi.fn()
    const { getByRole } = render(() => (
      <ResizeHandle onStart={() => 556} onMove={onMove} onEnd={onEnd} />
    ))
    const grip = getByRole('separator')
    grip.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'ArrowLeft', bubbles: true }),
    )
    grip.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }),
    )
    expect(onMove.mock.calls).toEqual([
      [556, 0, -16],
      [556, 0, 16],
    ])
    expect(onEnd).toHaveBeenCalledTimes(2)
  })
})
