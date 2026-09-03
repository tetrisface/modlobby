import { beforeEach, describe, expect, it, vi } from 'vitest'

const { mapIndex } = vi.hoisted(() => ({ mapIndex: vi.fn() }))
vi.mock('../ipc/client', () => ({ api: { mapIndex: () => mapIndex() } }))
// What Tauri's own helper does on Windows; the other platforms spell the
// scheme `thumb://localhost/`, and the Rust side reads the same path either way.
vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: (path: string, protocol: string) =>
    `http://${protocol}.localhost/${encodeURIComponent(path)}`,
}))

// The shape the index publishes: the official lobby's 1024px transform.
const PUBLISHED =
  'https://maps-metadata.beyondallreason.dev/i/fit-in/1024x1024/filters:format(webp):quality(75)/rowy-1f075.appspot.com/maps/x/photo/AcidicQuarry_5.16.jpg'

const INDEX = {
  images: { 'AcidicQuarry 5.17': PUBLISHED },
  names: { 'acidicquarry_5.17': 'AcidicQuarry 5.17' },
}

/** The module keeps one load per page; each test wants a page of its own. */
async function fresh() {
  vi.resetModules()
  return import('./maps')
}

beforeEach(() => {
  mapIndex.mockReset()
})

describe('the map index, as the room sees it', () => {
  it('hands the published picture URL through untouched', async () => {
    mapIndex.mockResolvedValue(INDEX)
    const maps = await fresh()
    expect(await maps.mapImage('AcidicQuarry 5.17')).toBe(PUBLISHED)
    expect(await maps.mapImage('Nowhere 1')).toBeNull()
    expect(await maps.mapImage('')).toBeNull()
    expect((await maps.mapNames())['acidicquarry_5.17']).toBe(
      'AcidicQuarry 5.17',
    )
  })

  it('asks Rust once however many callers arrive together', async () => {
    mapIndex.mockResolvedValue(INDEX)
    const maps = await fresh()
    await Promise.all([
      maps.mapImage('AcidicQuarry 5.17'),
      maps.mapImage('Nowhere 1'),
      maps.mapNames(),
    ])
    expect(mapIndex).toHaveBeenCalledTimes(1)
  })

  it('answers nothing when Rust cannot be reached, and asks again next time', async () => {
    mapIndex.mockRejectedValueOnce(new Error('no ipc'))
    mapIndex.mockResolvedValue(INDEX)
    const maps = await fresh()
    expect(await maps.mapNames()).toEqual({})
    expect(await maps.mapImage('AcidicQuarry 5.17')).toBe(PUBLISHED)
    expect(mapIndex).toHaveBeenCalledTimes(2)
  })
})

describe('a tile-sized picture', () => {
  it('is asked of Rust in device pixels, by spring name', async () => {
    vi.stubGlobal('devicePixelRatio', 1.5)
    const maps = await fresh()
    expect(maps.mapThumb('AcidicQuarry 5.17', 50, 32)).toBe(
      'http://thumb.localhost/75x48%2FAcidicQuarry%205.17',
    )
    expect(maps.mapThumb('', 50, 32)).toBeNull()
    expect(mapIndex).not.toHaveBeenCalled()
    vi.unstubAllGlobals()
  })
})
