import { describe, expect, test, vi } from 'vitest'

// The module reaches for the notification plugin at import time; nothing here
// calls it, but it has to resolve.
vi.mock('@tauri-apps/plugin-notification', () => ({
  isPermissionGranted: vi.fn(async () => false),
  requestPermission: vi.fn(async () => 'denied'),
  sendNotification: vi.fn(),
}))

vi.mock('@tauri-apps/api/window', () => ({
  UserAttentionType: { Critical: 1, Informational: 2 },
  getCurrentWindow: () => ({ requestUserAttention: vi.fn(async () => {}) }),
}))

const { plan, flashTarget, keepTrying } = await import('./alerts')

describe('where an alert goes', () => {
  test('off says nothing, whoever is looking', () => {
    expect(plan('off', true)).toBe('nothing')
    expect(plan('off', false)).toBe('nothing')
  })

  test('in lobby is the corner of this window, focused or not', () => {
    expect(plan('lobby', true)).toBe('lobby')
    expect(plan('lobby', false)).toBe('lobby')
  })

  test('desktop is the desktop, and never the corner', () => {
    // The overlap that made the three choices not really three: desktop used
    // to fall back to the corner, so picking it also got you what `lobby` does.
    expect(plan('desktop', false)).toBe('desktop')
    expect(plan('desktop', true)).toBe('nothing')
  })

  test('no two choices ever do the same thing at the same moment', () => {
    for (const focused of [true, false]) {
      const done = (['off', 'lobby', 'desktop'] as const).map((where) =>
        plan(where, focused),
      )
      // `off` and one other may both be silent — that is what off is. What must
      // never happen is `lobby` and `desktop` landing in the same place.
      expect(done[1]).not.toBe(done[2])
    }
  })
})

describe('which window flashes', () => {
  test('a starting game flashes the engine or nothing; never the lobby', () => {
    expect(flashTarget('gameStarting', true)).toBe('engine')
    expect(flashTarget('gameStarting', false)).toBe('nothing')
  })

  test('a finished game flashes the engine, and the lobby once it is gone', () => {
    expect(flashTarget('gameEnded', true)).toBe('engine')
    expect(flashTarget('gameEnded', false)).toBe('lobby')
  })

  test('anything else is about the lobby, whatever the engine is doing', () => {
    expect(flashTarget('privateMessage', true)).toBe('lobby')
    expect(flashTarget('mention', false)).toBe('lobby')
  })
})

describe('waiting for the engine window', () => {
  test('keeps asking until the window is there', async () => {
    let clock = 0
    const sleep = async (ms: number) => {
      clock += ms
    }
    const answers = [false, false, true]
    const attempt = vi.fn(async () => answers.shift() ?? true)
    expect(await keepTrying(attempt, 3000, 250, () => clock, sleep)).toBe(true)
    expect(attempt).toHaveBeenCalledTimes(3)
    expect(clock).toBe(500)
  })

  test('gives up when the time is spent', async () => {
    let clock = 0
    const sleep = async (ms: number) => {
      clock += ms
    }
    const attempt = vi.fn(async () => false)
    expect(await keepTrying(attempt, 3000, 250, () => clock, sleep)).toBe(false)
    expect(clock).toBe(3000)
    expect(attempt).toHaveBeenCalledTimes(13)
  })
})
