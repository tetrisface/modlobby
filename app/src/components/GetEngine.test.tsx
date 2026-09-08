import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { VersionView } from '../ipc/bindings/VersionView'
import { setBuild } from '../store/build'
import { GetEngine, forgetAskedEngines } from './GetEngine'

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
  // What `auto` has already asked for outlives a mount on purpose, so it has
  // to be forgotten between tests the way the build signal is.
  forgetAskedEngines()
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

  test('a room that remounts does not ask the index again', async () => {
    // A reload, a tab away and back, a list rebuilding its rows: each of those
    // used to be another trip to BAR's index for a version it may well have no
    // build for, and each 404 arrived as a red notice about the network.
    const first = render(() => <GetEngine version='2026.07.04' auto />)
    await settle()
    first.unmount()

    const second = render(() => <GetEngine version='2026.07.04' auto />)
    await settle()
    expect(sent('download_engine')).toEqual([{ version: '2026.07.04' }])
    second.unmount()

    // A version nobody has asked about yet is still a new question.
    render(() => <GetEngine version='2026.09.01' auto />)
    await settle()
    expect(sent('download_engine')).toEqual([
      { version: '2026.07.04' },
      { version: '2026.09.01' },
    ])
  })

  test('a click asks again for a version already tried on its own', async () => {
    // The standing question is only about asking unprompted. Somebody who
    // clicks after a failure means it, and gets the whole answer back.
    const { getByText } = render(() => <GetEngine version='2026.07.04' auto />)
    await settle()
    expect(sent('download_engine')).toHaveLength(1)

    fireEvent.click(getByText('Download the engine'))
    await settle()
    expect(sent('download_engine')).toHaveLength(2)
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
