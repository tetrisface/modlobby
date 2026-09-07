import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import { GetEngine } from './GetEngine'

/**
 * The one download modlobby does itself, and the one it must not.
 *
 * A fresh machine opens its room with no engine version at all, and this
 * component used to fire on mount regardless: `find?category=engine_linux64&
 * springname=` answers 404, whose text reads like the network is broken, and
 * the button offered to ask again.
 */
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
}))

async function settle() {
  for (let turn = 0; turn < 8; turn++) await Promise.resolve()
}

/** The commands that reached Tauri, with what they carried. */
function sent(command: string) {
  return vi
    .mocked(invoke)
    .mock.calls.filter(([name]) => name === command)
    .map(([, args]) => args)
}

beforeEach(() => {
  vi.mocked(invoke).mockResolvedValue(null)
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('getting the first engine onto a machine', () => {
  test('a version it was given is fetched once, on mount', async () => {
    render(() => <GetEngine version='2026.07.04' auto />)
    await settle()
    expect(sent('download_engine')).toEqual([{ version: '2026.07.04' }])
  })

  test('an engine with no version is never asked for', async () => {
    const { getByText } = render(() => <GetEngine version='' auto />)
    await settle()
    expect(sent('download_engine')).toEqual([])

    // Nor by hand: the same question would get the same 404.
    fireEvent.click(getByText('Download the engine'))
    await settle()
    expect(sent('download_engine')).toEqual([])
  })

  test('the heading names the version when there is one', async () => {
    const named = render(() => <GetEngine version='2026.07.04' />)
    await settle()
    expect(named.getByText('Engine 2026.07.04 is not installed.')).toBeTruthy()
    named.unmount()

    // Rather than "Engine  is not installed.", with the gap where it went.
    const bare = render(() => <GetEngine version='' />)
    await settle()
    expect(bare.getByText('The engine is not installed.')).toBeTruthy()
  })
})
