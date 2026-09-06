import { afterEach, describe, expect, test, vi } from 'vitest'
import { resetScale, setScale, uiScale } from './settings'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: () => Promise.reject(new Error('no backend under test')),
}))

/** Pretend we are looking at a given display. */
function on(width: number, height: number) {
  vi.stubGlobal('screen', { width, height })
}

const drawnAt = () =>
  document.documentElement.style.getPropertyValue('--ui-scale')

afterEach(() => {
  vi.unstubAllGlobals()
  document.documentElement.style.removeProperty('--ui-scale')
})

describe('sizing the interface', () => {
  test('the root carries the scale, since every length hangs off it', () => {
    on(1920, 1080)
    setScale(150)
    expect(drawnAt()).toBe('1.5')
    expect(uiScale()).toBe(150)
  })

  test('full size is a plain 1, not a rounding of one', () => {
    on(1920, 1080)
    setScale(100)
    expect(drawnAt()).toBe('1')
  })

  test('a size the screen cannot carry is held back to what it can', () => {
    on(1920, 1080)
    setScale(400)
    // 200% is this screen's ceiling.
    expect(uiScale()).toBe(200)
    expect(drawnAt()).toBe('2')
  })

  test('and a bigger screen carries more', () => {
    on(3840, 2160)
    setScale(400)
    expect(uiScale()).toBe(400)
  })

  test('reset goes to what the screen suits, not to full size', () => {
    on(2560, 1440)
    setScale(200)
    resetScale()
    expect(uiScale()).toBe(110)
  })

  test('which on an ordinary screen is full size', () => {
    on(1920, 1080)
    setScale(180)
    resetScale()
    expect(uiScale()).toBe(100)
  })
})
