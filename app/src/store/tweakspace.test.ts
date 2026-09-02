import { createSignal } from 'solid-js'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { Prepared } from '../ipc/bindings/Prepared'
import type { TweakView } from '../ipc/bindings/TweakView'
import { draftId, slotId } from '../lib/tweakspace'
import { createTweakspace, type TweakIo } from './tweakspace'

/** A base64 that is its own Lua: the fake decodes by prefixing a header. */
function fakeIo() {
  const files = new Map<string, string>()
  const tweakPrepare = vi.fn(async (lua: string): Promise<Prepared> => ({
    minified: lua,
    blob: 'b',
    command: '!bSet x b',
    gauge: {
      raw: lua.length,
      minified: lua.length,
      blob: 1,
      command: 9,
      cap: 16385,
      fits: true,
    },
  }))
  const io = {
    tweakDecode: vi.fn(async (blob: string): Promise<TweakView> => ({
      lua: `lua of ${blob}`,
      formatted: `-- ${blob}\nlua of ${blob}\n`,
      name: blob,
      summary: `${blob.length}:hash`,
      diagnostics: [],
    })),
    tweakFormat: vi.fn(async (lua: string) => `${lua.trim()}\n`),
    tweakPrepare,
    tweakCheck: vi.fn(async () => ({ problems: [], outline: [] })),
    tweakDiffText: vi.fn(async () => ({
      added: 0,
      removed: 0,
      unified: '',
      hunks: [],
    })),
    tweakSend: vi.fn(async (lua: string) => tweakPrepare(lua)),
    tweakClear: vi.fn(async () => {}),
    listDrafts: vi.fn(async () => [...files.keys()].sort()),
    readDraft: vi.fn(async (name: string) => files.get(name) ?? ''),
    saveDraft: vi.fn(async (name: string, lua: string) => {
      files.set(name, lua)
    }),
    deleteDraft: vi.fn(async (name: string) => {
      files.delete(name)
    }),
  } satisfies TweakIo
  return { io, files }
}

/** Lets the store's awaited decodes land; the clock is fake in these tests. */
const flush = () => vi.advanceTimersByTimeAsync(0)

describe('the workspace', () => {
  beforeEach(() => vi.useFakeTimers())
  afterEach(() => vi.useRealTimers())

  test('a slot shows the room value, decoded once per blob', async () => {
    const { io } = fakeIo()
    const [room, setRoom] = createSignal<Record<string, string>>({
      tweakdefs1: 'AAA',
    })
    const space = createTweakspace(io, room, slotId('tweakdefs1'))
    await flush()
    expect(space.active()).toMatchObject({
      loaded: true,
      buffer: '-- AAA\nlua of AAA\n',
      name: 'AAA',
    })
    expect(io.tweakDecode).toHaveBeenCalledTimes(1)

    // The same value again -- a resend, a rejoin -- is not decoded again.
    setRoom({ tweakdefs1: 'AAA' })
    await flush()
    expect(io.tweakDecode).toHaveBeenCalledTimes(1)

    setRoom({ tweakdefs1: 'BBB' })
    await flush()
    expect(space.active().buffer).toBe('-- BBB\nlua of BBB\n')
    expect(io.tweakDecode).toHaveBeenCalledTimes(2)
    space.dispose()
  })

  test('an edit survives the room changing under it, marked stale', async () => {
    const { io } = fakeIo()
    const [room, setRoom] = createSignal<Record<string, string>>({
      tweakdefs1: 'AAA',
    })
    const space = createTweakspace(io, room, slotId('tweakdefs1'))
    await flush()
    space.edit(slotId('tweakdefs1'), 'mine')
    setRoom({ tweakdefs1: 'BBB' })
    await flush()
    expect(space.active()).toMatchObject({
      buffer: 'mine',
      original: '-- BBB\nlua of BBB\n',
      stale: true,
    })
    expect(space.modified()).toBe(1)
    space.reset(slotId('tweakdefs1'))
    expect(space.active()).toMatchObject({
      buffer: '-- BBB\nlua of BBB\n',
      stale: false,
    })
    space.dispose()
  })

  test('the gauge follows the buffer once typing pauses', async () => {
    const { io } = fakeIo()
    const space = createTweakspace(io, () => ({}), slotId('tweakdefs1'))
    space.edit(slotId('tweakdefs1'), 'local a')
    space.edit(slotId('tweakdefs1'), 'local a = 1')
    await vi.advanceTimersByTimeAsync(300)
    expect(io.tweakPrepare).toHaveBeenCalledTimes(1)
    expect(io.tweakPrepare).toHaveBeenLastCalledWith(
      'local a = 1',
      { kind: 'defs', index: 1 },
      true,
    )
    expect(space.prepared()?.gauge.fits).toBe(true)

    io.tweakPrepare.mockRejectedValueOnce(new Error('Lua: unexpected token'))
    space.edit(slotId('tweakdefs1'), 'local a = ')
    await vi.advanceTimersByTimeAsync(300)
    expect(space.prepared()).toBeNull()
    expect(space.problem()).toBe('Lua: unexpected token')
    space.dispose()
  })

  test('drafts come off disk, are saved from any document, and go again', async () => {
    const { io, files } = fakeIo()
    files.set('walls', '{ armwall = {} }')
    const space = createTweakspace(io, () => ({}), slotId('tweakunits2'))
    await space.refreshDrafts()
    expect(space.ws.docs[draftId('walls')]).toMatchObject({
      kind: 'units',
      buffer: '{ armwall = {} }',
    })

    space.edit(slotId('tweakunits2'), '{ armcom = {} }')
    await space.saveDraft('com')
    expect(files.get('com')).toBe('{ armcom = {} }')
    expect(space.ws.docs[draftId('com')]).toMatchObject({
      kind: 'units',
      original: '{ armcom = {} }',
    })
    // The slot is still not what the room holds.
    expect(space.modified()).toBe(1)

    space.open(draftId('com'))
    expect(space.ws.target).toBe('tweakunits1')
    await space.deleteDraft('com')
    expect(space.ws.docs[draftId('com')]).toBeUndefined()
    expect(space.ws.active).toBe(slotId('tweakdefs'))
    space.dispose()
  })

  test('a direct send makes a slot clean; a vote does not', async () => {
    const { io } = fakeIo()
    const space = createTweakspace(io, () => ({}), slotId('tweakdefs1'))
    space.edit(slotId('tweakdefs1'), 'local a = 1')
    await space.send(false)
    expect(space.modified()).toBe(1)
    await space.send(true)
    expect(space.modified()).toBe(0)
    expect(io.tweakSend).toHaveBeenLastCalledWith(
      'local a = 1',
      { kind: 'defs', index: 1 },
      true,
    )
    space.dispose()
  })

  test('the list is searched and sorted through the filter', async () => {
    const { io } = fakeIo()
    const [room] = createSignal<Record<string, string>>({ tweakunits2: 'XYZ' })
    const space = createTweakspace(io, room)
    await flush()
    space.setFilter({ query: 'xyz' })
    expect(space.items().map((item) => item.title)).toEqual(['tweakunits2'])
    space.dispose()
  })
})
