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

const { plan } = await import('./alerts')

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
