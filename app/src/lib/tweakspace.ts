/**
 * The tweak workspace, as data.
 *
 * Twenty room slots and however many drafts, each a document with the text it
 * was loaded with and the text it holds now. Nothing here talks to Rust or
 * to Monaco: the store (`store/tweakspace.ts`) does the asking and the
 * editor does the drawing, and this is what both of them agree on -- which
 * is why every rule about dirty, stale and sent lives here, where a test can
 * reach it.
 */

import type { Diagnostic } from '../ipc/bindings/Diagnostic'
import type { Kind } from '../ipc/bindings/Kind'
import type { OptionChangeView } from '../ipc/bindings/OptionChangeView'
import type { Slot } from '../ipc/bindings/Slot'
import { TWEAK_SLOTS } from './setup'

export type DocId = `slot:${string}` | `draft:${string}`
export type Origin = 'slot' | 'draft'

export type Doc = {
  id: DocId
  origin: Origin
  /** The slot key, or the draft's file name. */
  title: string
  kind: Kind
  /** The text as loaded: the room's value formatted, or the draft file. */
  original: string
  /** What the editor holds. Dirty means it differs from `original`. */
  buffer: string
  /** The room's stored value, for a slot; a draft has none. */
  blob: string | null
  /** The leading `--` comment, which is how BAR names a tweak. */
  name: string | null
  /** Chobby's `<length>:<hash>`, so a slot can be told from another at a glance. */
  summary: string | null
  /** The room moved under an unsaved edit: `original` is new, `buffer` is not. */
  stale: boolean
  loaded: boolean
  /** What decoding had to say about the room's value; see [`noteOf`]. */
  notes: string[]
}

export type Sort = 'order' | 'name' | 'kind'
export type Segment = 'slots' | 'drafts'
export type Filter = { query: string; sort: Sort; segment: Segment }

/**
 * One side of a comparison: a document's text as edited or as loaded, one
 * end of a change the room saw this session, or what a vote proposes.
 */
export type Side =
  | { doc: DocId; text: 'buffer' | 'original' }
  | { history: number; which: 'from' | 'to' }
  | { vote: true }

export type Compare = { left: Side; right: Side }

export type Workspace = {
  docs: Record<string, Doc>
  active: DocId
  filter: Filter
  /** The slot a draft is sent to; a slot document is sent to itself. */
  target: string
  fullscreen: boolean
  /** What is being compared, in place of the editor, when something is. */
  compare: Compare | null
}

export const slotId = (key: string): DocId => `slot:${key}`
export const draftId = (name: string): DocId => `draft:${name}`
export const isSlotId = (id: DocId): boolean => id.startsWith('slot:')
export const titleOf = (id: DocId): string => id.slice(id.indexOf(':') + 1)

export function kindOf(key: string): Kind {
  return key.startsWith('tweakunits') ? 'units' : 'defs'
}

/** `tweakdefs` is index 0, `tweakdefs1` index 1, and so on to 9. */
export function slotOf(key: string): Slot | null {
  const match = /^tweak(defs|units)([1-9]?)$/.exec(key)
  if (!match) return null
  return {
    kind: match[1] as Kind,
    index: match[2] === '' ? 0 : Number(match[2]),
  }
}

export function slotKey(slot: Slot): string {
  return `tweak${slot.kind}${slot.index === 0 ? '' : slot.index}`
}

/** Where a draft goes unless told otherwise: the first numbered slot of its kind. */
export function defaultTarget(kind: Kind): string {
  return `tweak${kind}1`
}

/**
 * A tweakunits payload is a bare table constructor; anything else is code.
 * Decides what a draft is, since the file carries no kind of its own.
 */
export function guessKind(lua: string): Kind {
  const body = lua.replace(/^(\s*--[^\n]*\n?)*\s*/, '')
  return body.startsWith('{') ? 'units' : 'defs'
}

function slotDoc(key: string): Doc {
  return {
    id: slotId(key),
    origin: 'slot',
    title: key,
    kind: kindOf(key),
    original: '',
    buffer: '',
    blob: null,
    name: null,
    summary: null,
    stale: false,
    loaded: false,
    notes: [],
  }
}

export function draftDoc(name: string, lua: string): Doc {
  return {
    id: draftId(name),
    origin: 'draft',
    title: name,
    kind: guessKind(lua),
    original: lua,
    buffer: lua,
    blob: null,
    name: firstComment(lua),
    summary: null,
    stale: false,
    loaded: true,
    notes: [],
  }
}

/** The leading `--` line, trimmed, the way `tweaks::name` reads it. */
export function firstComment(lua: string): string | null {
  const match = /^\s*--\s*([^\n]*)/.exec(lua)
  const text = match?.[1]?.trim()
  return text ? text : null
}

export function emptyWorkspace(
  active: DocId = slotId(TWEAK_SLOTS[0]!),
): Workspace {
  const docs: Record<string, Doc> = {}
  for (const key of TWEAK_SLOTS) docs[slotId(key)] = slotDoc(key)
  return {
    docs,
    active,
    filter: { query: '', sort: 'order', segment: 'slots' },
    target: defaultTarget('defs'),
    fullscreen: false,
    compare: null,
  }
}

export const isDirty = (doc: Doc): boolean => doc.buffer !== doc.original

export type Loaded = {
  blob: string
  text: string
  name: string | null
  summary: string | null
  notes: string[]
}

/** A decoder's finding, as a sentence. */
export function noteOf(diagnostic: Diagnostic): string {
  switch (diagnostic.type) {
    case 'underscoreCorruption':
      return `This payload contains ${diagnostic.count} \`_\`, which the game reads as \`=\` -- what it loads is not what is stored here.`
  }
}

/**
 * The room's value for a slot has arrived, or changed.
 *
 * A clean document simply becomes the new text. A dirty one keeps what was
 * typed and is marked stale when the room's value is not the one it started
 * from -- losing an edit to somebody else's `!bSet` is the one thing this
 * must never do.
 */
export function loaded(doc: Doc, from: Loaded): Doc {
  const base = {
    ...doc,
    blob: from.blob,
    name: from.name,
    summary: from.summary,
    notes: from.notes,
    loaded: true,
  }
  if (!isDirty(doc)) {
    return { ...base, original: from.text, buffer: from.text, stale: false }
  }
  return {
    ...base,
    original: from.text,
    stale: doc.stale || doc.blob !== from.blob,
  }
}

export function edit(doc: Doc, text: string): Doc {
  return text === doc.buffer ? doc : { ...doc, buffer: text }
}

export function reset(doc: Doc): Doc {
  return { ...doc, buffer: doc.original, stale: false }
}

/** Sent as a direct `!bSet`: the buffer is now what the room will hold. */
export function sent(doc: Doc): Doc {
  return {
    ...doc,
    original: doc.buffer,
    stale: false,
    name: firstComment(doc.buffer),
  }
}

/** The draft name to use when none is typed: its own, else its header, else its slot. */
export function draftNameFor(doc: Doc): string {
  return (doc.origin === 'draft' ? doc.title : doc.name) || doc.title
}

/** Saved to a draft under this name: that draft now holds the buffer. */
export function savedAs(doc: Doc, name: string): Doc {
  return { ...draftDoc(name, doc.buffer), kind: doc.kind }
}

export type Item = {
  id: DocId
  title: string
  kind: Kind
  name: string | null
  dirty: boolean
  stale: boolean
  empty: boolean
  size: number
  /** What `size` counts: the room's blob, or the draft's Lua. */
  unit: 'blob' | 'lua'
}

function itemOf(doc: Doc): Item {
  const slot = doc.origin === 'slot'
  return {
    id: doc.id,
    title: doc.title,
    kind: doc.kind,
    name: doc.name,
    dirty: isDirty(doc),
    stale: doc.stale,
    empty: slot ? (doc.blob ?? '') === '' : doc.buffer === '',
    size: slot ? (doc.blob ?? '').length : doc.buffer.length,
    unit: slot ? 'blob' : 'lua',
  }
}

const order = (item: Item) => {
  const at = TWEAK_SLOTS.indexOf(item.title)
  return at === -1 ? TWEAK_SLOTS.length : at
}

const BY: Record<Sort, (a: Item, b: Item) => number> = {
  order: (a, b) => order(a) - order(b) || a.title.localeCompare(b.title),
  name: (a, b) => (a.name ?? a.title).localeCompare(b.name ?? b.title),
  kind: (a, b) =>
    a.kind.localeCompare(b.kind) ||
    order(a) - order(b) ||
    a.title.localeCompare(b.title),
}

/** The list on the left: one segment, searched and sorted. */
export function listItems(ws: Workspace, filter: Filter = ws.filter): Item[] {
  const origin: Origin = filter.segment === 'slots' ? 'slot' : 'draft'
  const needle = filter.query.trim().toLowerCase()
  return Object.values(ws.docs)
    .filter((doc) => doc.origin === origin)
    .map(itemOf)
    .filter(
      (item) =>
        needle === '' ||
        item.title.toLowerCase().includes(needle) ||
        (item.name ?? '').toLowerCase().includes(needle),
    )
    .sort(BY[filter.sort])
}

export function modifiedCount(ws: Workspace): number {
  return Object.values(ws.docs).filter(isDirty).length
}

/** The slot the active document would be sent to. */
export function targetOf(ws: Workspace): Slot | null {
  const doc = ws.docs[ws.active]
  if (!doc) return null
  return slotOf(doc.origin === 'slot' ? doc.title : ws.target)
}

// ---- comparing ----

/** A side as a `<select>` value, and back. */
export function sideKey(side: Side): string {
  if ('doc' in side) return `doc:${side.text}:${side.doc}`
  if ('history' in side) return `history:${side.which}:${side.history}`
  return 'vote'
}

export function parseSide(key: string): Side | null {
  if (key === 'vote') return { vote: true }
  const match = /^(doc|history):(buffer|original|from|to):(.+)$/.exec(key)
  if (!match) return null
  const [, what, text, rest] = match
  if (what === 'doc') {
    if (text !== 'buffer' && text !== 'original') return null
    return { doc: rest as DocId, text }
  }
  const seq = Number(rest)
  if ((text !== 'from' && text !== 'to') || !Number.isInteger(seq)) return null
  return { history: seq, which: text }
}

export type SideGroup = 'Slots' | 'Drafts' | 'Changes this session' | 'Vote'
export type SideOption = {
  key: string
  label: string
  side: Side
  group: SideGroup
}

/**
 * Everything worth putting on one side of a diff, grouped for a menu.
 *
 * Of the room's history, only the tweak slots: the other modoptions are a
 * number, a switch, or -- the start boxes -- zlib inside base64, none of
 * which is Lua anyone wants to see diffed as text.
 */
export function sideOptions(
  ws: Workspace,
  history: OptionChangeView[],
  vote: string | null,
): SideOption[] {
  const out: SideOption[] = []
  for (const doc of Object.values(ws.docs)) {
    const slot = doc.origin === 'slot'
    const held = slot ? (doc.blob ?? '') !== '' : true
    const dirty = isDirty(doc)
    if (!held && !dirty) continue
    const group: SideGroup = slot ? 'Slots' : 'Drafts'
    if (held) {
      const side: Side = { doc: doc.id, text: 'original' }
      out.push({
        key: sideKey(side),
        label: `${doc.title} · ${slot ? 'room' : 'file'}`,
        side,
        group,
      })
    }
    if (dirty) {
      const side: Side = { doc: doc.id, text: 'buffer' }
      out.push({
        key: sideKey(side),
        label: `${doc.title} · edited`,
        side,
        group,
      })
    }
  }
  for (const change of history) {
    if (slotOf(change.key) === null) continue
    for (const which of ['from', 'to'] as const) {
      const side: Side = { history: change.seq, which }
      const by = change.by ? ` by ${change.by}` : ''
      out.push({
        key: sideKey(side),
        label: `#${change.seq} ${change.key} ${which === 'from' ? 'before' : 'after'}${by}`,
        side,
        group: 'Changes this session',
      })
    }
  }
  if (vote !== null) {
    out.push({
      key: 'vote',
      label: 'what the vote proposes',
      side: { vote: true },
      group: 'Vote',
    })
  }
  return out
}

/** A side, found: either Lua ready to show, or a blob still to decode. */
export type Resolved = { label: string; kind: Kind } & (
  { lua: string } | { blob: string }
)

export function resolveSide(
  ws: Workspace,
  side: Side,
  history: OptionChangeView[],
  vote: string | null,
): Resolved | null {
  if ('doc' in side) {
    const doc = ws.docs[side.doc]
    if (!doc) return null
    return {
      label: `${doc.title} · ${side.text === 'buffer' ? 'edited' : doc.origin === 'slot' ? 'room' : 'file'}`,
      kind: doc.kind,
      lua: side.text === 'buffer' ? doc.buffer : doc.original,
    }
  }
  if ('history' in side) {
    const change = history.find((entry) => entry.seq === side.history)
    if (!change) return null
    return {
      label: `#${change.seq} ${change.key} ${side.which === 'from' ? 'before' : 'after'}`,
      kind: kindOf(change.key),
      blob: side.which === 'from' ? change.from : change.to,
    }
  }
  if (vote === null) return null
  const active = ws.docs[ws.active]
  return {
    label: 'the vote proposes',
    kind: active?.kind ?? 'defs',
    blob: vote,
  }
}

/**
 * What opening Compare shows first: the open document as loaded against as
 * edited -- or against itself when nothing has been typed, so both sides
 * name something the menu has.
 */
export function defaultCompare(ws: Workspace): Compare {
  const doc = ws.docs[ws.active]
  const right = doc && isDirty(doc) ? 'buffer' : 'original'
  return {
    left: { doc: ws.active, text: 'original' },
    right: { doc: ws.active, text: right },
  }
}
