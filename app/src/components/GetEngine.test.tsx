import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { VersionView } from '../ipc/bindings/VersionView'
import { setBuild } from '../store/build'
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

/** What the shell says about this machine, which is what `auto` waits for. */
const BUILD = (why: string | null = null): VersionView => ({
  version: '0.0.0',
  commit: 'abc1234',
  playsOnline: true,
  noPublishedEngine: why,
})

beforeEach(() => {
  vi.mocked(invoke).mockResolvedValue(null)
  // A machine with an engine to fetch, which is every machine but one. The
  // signal is module state shared across this file, hence the reset below.
  setBuild(BUILD())
})

afterEach(() => {
  cleanup()
  setBuild(null)
  vi.clearAllMocks()
})

describe('getting the first engine onto a machine', () => {
  test('a version it was given is fetched once, when it may be', async () => {
    render(() => <GetEngine version='2026.07.04' auto />)
    await settle()
    expect(sent('download_engine')).toEqual([{ version: '2026.07.04' }])
  })

  test('nothing is asked for before the shell has answered', async () => {
    // A reload straight into a room mounts this in the same tick as the
    // window, one round trip ahead of knowing whether there is an engine to
    // fetch at all. It used to ask anyway.
    setBuild(null)
    render(() => <GetEngine version='2026.07.04' auto />)
    await settle()
    expect(sent('download_engine')).toEqual([])

    setBuild(BUILD())
    await settle()
    expect(sent('download_engine')).toEqual([{ version: '2026.07.04' }])
  })

  test('a machine nothing is published for is never asked on its own', async () => {
    setBuild(BUILD('Beyond All Reason publishes no engine for this machine.'))
    const { getByText } = render(() => <GetEngine version='2026.07.04' auto />)
    await settle()
    expect(sent('download_engine')).toEqual([])

    // The button stays live, and deliberately: a click is a question somebody
    // meant to ask, and Rust answers it with the whole instruction rather than
    // with silence.
    fireEvent.click(getByText('Download the engine'))
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
