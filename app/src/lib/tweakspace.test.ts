import { describe, expect, test } from 'vitest'
import { TWEAK_SLOTS } from './setup'
import {
  defaultCompare,
  draftDoc,
  draftId,
  edit,
  emptyWorkspace,
  firstComment,
  guessKind,
  isDirty,
  kindOf,
  listItems,
  loaded,
  modifiedCount,
  parseSide,
  reset,
  resolveSide,
  savedAs,
  sent,
  sideKey,
  sideOptions,
  slotId,
  slotKey,
  slotOf,
  targetOf,
  titleOf,
  type Doc,
  type Side,
} from './tweakspace'

const DEFS = slotId('tweakdefs1')

function slot(over: Partial<Doc> = {}): Doc {
  return { ...emptyWorkspace().docs[DEFS]!, ...over }
}

const arrived = (blob: string, text: string) => ({
  blob,
  text,
  name: firstComment(text),
  summary: `${blob.length}:abcd`,
  notes: [],
})

describe('slots', () => {
  test('every one of the twenty keys parses and prints back', () => {
    for (const key of TWEAK_SLOTS) {
      const parsed = slotOf(key)
      expect(parsed).not.toBeNull()
      expect(slotKey(parsed!)).toBe(key)
      expect(parsed!.kind).toBe(kindOf(key))
    }
    expect(slotOf('tweakdefs')).toEqual({ kind: 'defs', index: 0 })
    expect(slotOf('tweakunits9')).toEqual({ kind: 'units', index: 9 })
    expect(slotOf('tweakunits10')).toBeNull()
    expect(slotOf('startmetal')).toBeNull()
  })

  test('ids carry their title', () => {
    expect(titleOf(slotId('tweakunits2'))).toBe('tweakunits2')
    expect(titleOf(draftId('my:draft'))).toBe('my:draft')
  })
})

describe('guessKind', () => {
  test('a table is units, code is defs, and a header does not decide', () => {
    expect(guessKind('{ armcom = { metalcost = 1 } }')).toBe('units')
    expect(guessKind('-- Named\n-- by me\n{ armcom = {} }')).toBe('units')
    expect(guessKind('local x = 1')).toBe('defs')
    expect(guessKind('-- Named\nfor name, ud in pairs(UnitDefs) do end')).toBe(
      'defs',
    )
    expect(guessKind('')).toBe('defs')
  })
})

describe('loading a slot', () => {
  test('a clean document becomes the room value', () => {
    const doc = loaded(slot(), arrived('YQ==', '-- Nutty\nlocal a = 1\n'))
    expect(doc.loaded).toBe(true)
    expect(doc.buffer).toBe('-- Nutty\nlocal a = 1\n')
    expect(doc.original).toBe(doc.buffer)
    expect(doc.name).toBe('Nutty')
    expect(isDirty(doc)).toBe(false)
    expect(doc.stale).toBe(false)
  })

  test('a dirty document keeps its edit and is stale once the room moves', () => {
    const before = loaded(slot(), arrived('YQ==', 'local a = 1\n'))
    const typed = edit(before, 'local a = 2\n')
    expect(isDirty(typed)).toBe(true)

    // The same blob decoded again: nothing moved.
    const same = loaded(typed, arrived('YQ==', 'local a = 1\n'))
    expect(same.buffer).toBe('local a = 2\n')
    expect(same.stale).toBe(false)

    const moved = loaded(typed, arrived('Yg==', 'local b = 1\n'))
    expect(moved.buffer).toBe('local a = 2\n')
    expect(moved.original).toBe('local b = 1\n')
    expect(moved.stale).toBe(true)
  })

  test('reset returns to the room and clears stale; sent adopts the buffer', () => {
    const doc = edit(loaded(slot(), arrived('YQ==', 'local a = 1\n')), 'x')
    const stale = loaded(doc, arrived('Yg==', 'local b = 1\n'))
    expect(reset(stale)).toMatchObject({
      buffer: 'local b = 1\n',
      stale: false,
    })
    const gone = sent(edit(stale, '-- Mine\nlocal c = 1\n'))
    expect(gone).toMatchObject({
      original: '-- Mine\nlocal c = 1\n',
      stale: false,
      name: 'Mine',
    })
  })

  test('typing what is already there is not an edit', () => {
    const doc = loaded(slot(), arrived('YQ==', 'a'))
    expect(edit(doc, 'a')).toBe(doc)
  })
})

describe('drafts', () => {
  test('a draft is loaded, named by its header, and typed by its shape', () => {
    const draft = draftDoc('walls', '-- T3 walls\n{ armwall = {} }')
    expect(draft).toMatchObject({
      id: 'draft:walls',
      origin: 'draft',
      kind: 'units',
      name: 'T3 walls',
      loaded: true,
    })
    expect(isDirty(draft)).toBe(false)
  })

  test('saving a slot as a draft keeps the slot kind even for an empty buffer', () => {
    const doc = slot({ kind: 'units', buffer: '' })
    expect(savedAs(doc, 'blank')).toMatchObject({
      id: 'draft:blank',
      kind: 'units',
    })
  })
})

describe('the list', () => {
  const ws = (() => {
    const base = emptyWorkspace()
    const docs = { ...base.docs }
    docs[slotId('tweakunits2')] = loaded(
      docs[slotId('tweakunits2')]!,
      arrived('e30=', '-- Golem\n{}'),
    )
    docs[DEFS] = edit(
      loaded(docs[DEFS]!, arrived('YQ==', 'local a = 1')),
      'local a = 2',
    )
    docs[draftId('b-draft')] = draftDoc('b-draft', 'local b')
    docs[draftId('a-draft')] = draftDoc('a-draft', '{ x = 1 }')
    return { ...base, docs }
  })()

  test('slots come in the order the game applies them', () => {
    const items = listItems(ws)
    expect(items.map((item) => item.title)).toEqual([...TWEAK_SLOTS])
    expect(items[1]).toMatchObject({ dirty: true, size: 4, unit: 'blob' })
    expect(items[12]).toMatchObject({
      title: 'tweakunits2',
      name: 'Golem',
      empty: false,
    })
  })

  test('drafts are their own segment, measured in Lua', () => {
    const items = listItems(ws, { query: '', sort: 'order', segment: 'drafts' })
    expect(items.map((item) => item.title)).toEqual(['a-draft', 'b-draft'])
    expect(items[0]).toMatchObject({ kind: 'units', size: 9, unit: 'lua' })
  })

  test('search matches the key or the header, either case', () => {
    expect(
      listItems(ws, { query: 'GOLEM', sort: 'order', segment: 'slots' }).map(
        (item) => item.title,
      ),
    ).toEqual(['tweakunits2'])
    expect(
      listItems(ws, { query: 'defs', sort: 'order', segment: 'slots' }),
    ).toHaveLength(10)
  })

  test('sorting by kind keeps defs first, and by name uses the header', () => {
    const byKind = listItems(ws, { query: '', sort: 'kind', segment: 'slots' })
    expect(byKind[0]!.kind).toBe('defs')
    expect(byKind[19]!.kind).toBe('units')
    const byName = listItems(ws, { query: '', sort: 'name', segment: 'slots' })
    expect(byName[0]!.title).toBe('tweakunits2')
  })

  test('counts what is modified across both segments', () => {
    expect(modifiedCount(ws)).toBe(1)
  })
})

describe('target', () => {
  test('a slot is sent to itself; a draft to the chosen slot', () => {
    const ws = emptyWorkspace(slotId('tweakunits3'))
    expect(targetOf(ws)).toEqual({ kind: 'units', index: 3 })
    const docs = { ...ws.docs, [draftId('d')]: draftDoc('d', 'x') }
    expect(
      targetOf({ ...ws, docs, active: draftId('d'), target: 'tweakdefs4' }),
    ).toEqual({ kind: 'defs', index: 4 })
  })
})

describe('comparing', () => {
  const ws = (() => {
    const base = emptyWorkspace(slotId('tweakdefs1'))
    const docs = { ...base.docs }
    docs[DEFS] = edit(
      loaded(docs[DEFS]!, arrived('YQ==', 'local a = 1')),
      'local a = 2',
    )
    docs[slotId('tweakunits2')] = loaded(
      docs[slotId('tweakunits2')]!,
      arrived('e30=', '{}'),
    )
    docs[draftId('walls')] = draftDoc('walls', '{ armwall = {} }')
    return { ...base, docs }
  })()
  const history = [
    { seq: 6, key: 'allowpausegameplay', from: '0', to: '1', by: null },
    { seq: 7, key: 'tweakdefs1', from: 'YQ==', to: 'Yg==', by: 'Lathek' },
    {
      seq: 8,
      key: 'mapmetadata_startbox_override',
      from: '',
      to: 'eJyr',
      by: 'Host',
    },
  ]

  test('every side survives the trip through a select value', () => {
    const sides: Side[] = [
      { doc: DEFS, text: 'buffer' },
      { doc: draftId('a:b'), text: 'original' },
      { history: 7, which: 'from' },
      { vote: true },
    ]
    for (const side of sides) expect(parseSide(sideKey(side))).toEqual(side)
    expect(parseSide('doc:sideways:slot:x')).toBeNull()
    expect(parseSide('history:from:seven')).toBeNull()
    expect(parseSide('nonsense')).toBeNull()
  })

  test('the menu lists held slots, edited buffers, drafts, tweak changes and the vote', () => {
    const options = sideOptions(ws, history, 'Yg==')
    expect(options.map((option) => option.label)).toEqual([
      'tweakdefs1 · room',
      'tweakdefs1 · edited',
      'tweakunits2 · room',
      'walls · file',
      '#7 tweakdefs1 before by Lathek',
      '#7 tweakdefs1 after by Lathek',
      'what the vote proposes',
    ])
    expect(options[3]!.group).toBe('Drafts')
    expect(sideOptions(ws, [], null).map((o) => o.group)).not.toContain('Vote')
  })

  test('a side resolves to Lua for a document and to a blob for the rest', () => {
    expect(
      resolveSide(ws, { doc: DEFS, text: 'buffer' }, history, null),
    ).toEqual({
      label: 'tweakdefs1 · edited',
      kind: 'defs',
      lua: 'local a = 2',
    })
    expect(resolveSide(ws, { history: 7, which: 'to' }, history, null)).toEqual(
      { label: '#7 tweakdefs1 after', kind: 'defs', blob: 'Yg==' },
    )
    expect(resolveSide(ws, { vote: true }, history, 'Yg==')).toMatchObject({
      kind: 'defs',
      blob: 'Yg==',
    })
    expect(resolveSide(ws, { vote: true }, history, null)).toBeNull()
    expect(
      resolveSide(ws, { history: 99, which: 'to' }, history, null),
    ).toBeNull()
  })

  test('opening compare shows the edit against what was loaded, or a clean document against itself', () => {
    expect(defaultCompare(ws)).toEqual({
      left: { doc: DEFS, text: 'original' },
      right: { doc: DEFS, text: 'buffer' },
    })
    const clean = { ...ws, active: slotId('tweakunits2') }
    expect(defaultCompare(clean).right).toEqual({
      doc: slotId('tweakunits2'),
      text: 'original',
    })
  })
})
