import {
  createEffect,
  createMemo,
  createRoot,
  createSignal,
  on,
  onCleanup,
  untrack,
  type Accessor,
} from 'solid-js'
import { createStore, produce } from 'solid-js/store'
import type { Check } from '../ipc/bindings/Check'
import type { Kind } from '../ipc/bindings/Kind'
import type { Prepared } from '../ipc/bindings/Prepared'
import type { TweakView } from '../ipc/bindings/TweakView'
import type { api } from '../ipc/client'
import { NO_ASSIST, type Assist } from '../lib/assist'
import { TWEAK_SLOTS } from '../lib/setup'
import {
  defaultTarget,
  draftDoc,
  draftId,
  edit as editDoc,
  emptyWorkspace,
  isDirty,
  kindOf,
  listItems,
  loaded,
  modifiedCount,
  noteOf,
  reset as resetDoc,
  savedAs,
  sent,
  slotId,
  targetOf,
  type Compare,
  type DocId,
  type Filter,
  type Loaded,
  type Workspace,
} from '../lib/tweakspace'

/**
 * The workspace's side: what it asks Rust for.
 *
 * Handed in rather than imported so a test can stand in for the whole of
 * it -- and so the store never learns that there is such a thing as Tauri.
 */
export type TweakIo = Pick<
  typeof api,
  | 'tweakDecode'
  | 'tweakFormat'
  | 'tweakPrepare'
  | 'tweakCheck'
  | 'tweakDiffText'
  | 'tweakSend'
  | 'tweakClear'
  | 'listDrafts'
  | 'readDraft'
  | 'saveDraft'
  | 'deleteDraft'
>

/** The room's modoptions, keyed without their prefix; what the slots watch. */
export type RoomValues = Accessor<Record<string, string>>

export type Tweakspace = ReturnType<typeof createTweakspace>

/** How long typing has to pause before the payload is minified and measured. */
const SETTLE_MS = 250

/** Distinct blobs decoded this session, so a list of twenty costs twenty calls once. */
const DECODED_MOST = 200

function message(error: unknown): string {
  if (typeof error === 'object' && error !== null && 'message' in error)
    return String((error as { message: unknown }).message)
  return String(error)
}

/**
 * The tweak workspace: every slot in the room and every draft on disk, with
 * whatever has been typed into them, alive for as long as the app is.
 *
 * Made under its own root because it outlives any view. The editor pane is
 * mounted and unmounted as the reader moves about the room; what they were
 * writing must not be.
 */
export function createTweakspace(
  io: TweakIo,
  room: RoomValues,
  initial?: DocId,
) {
  return createRoot((dispose) => {
    const [ws, setWs] = createStore<Workspace>(emptyWorkspace(initial))

    const active = createMemo(() => ws.docs[ws.active]!)
    const items = createMemo(() => listItems(ws))
    const modified = createMemo(() => modifiedCount(ws))

    // ---- the room's slots ----

    const decoded = new Map<string, Promise<TweakView>>()
    function decode(blob: string, kind: Kind): Promise<TweakView> {
      const key = `${kind}:${blob}`
      const known = decoded.get(key)
      if (known) return known
      if (decoded.size >= DECODED_MOST) decoded.clear()
      const pending = io.tweakDecode(blob, kind)
      decoded.set(key, pending)
      pending.catch(() => decoded.delete(key))
      return pending
    }

    async function arrive(key: string, blob: string): Promise<Loaded> {
      if (blob === '')
        return { blob, text: '', name: null, summary: null, notes: [] }
      try {
        const view = await decode(blob, kindOf(key))
        return {
          blob,
          text: view.formatted,
          name: view.name,
          summary: view.summary,
          notes: view.diagnostics.map(noteOf),
        }
      } catch (error) {
        // Not something we can read; shown as it is rather than as nothing.
        return {
          blob,
          text: blob,
          name: null,
          summary: null,
          notes: [`This slot could not be decoded: ${message(error)}`],
        }
      }
    }

    /** The blob each slot is being decoded for, so a superseded answer is dropped. */
    const inFlight = new Map<string, string>()

    async function load(key: string, blob: string) {
      inFlight.set(key, blob)
      const from = await arrive(key, blob)
      if (inFlight.get(key) !== blob) return
      inFlight.delete(key)
      setWs('docs', slotId(key), (doc) => loaded(doc, from))
    }

    createEffect(() => {
      const values = room()
      for (const key of TWEAK_SLOTS) {
        const blob = values[key] ?? ''
        const doc = untrack(() => ws.docs[slotId(key)]!)
        if (doc.loaded && doc.blob === blob) continue
        if (inFlight.get(key) === blob) continue
        void load(key, blob)
      }
    })

    // ---- the gauge, and the check beside it ----

    const [prepared, setPrepared] = createSignal<Prepared | null>(null)
    /** Why the buffer could not be prepared: a syntax error, most days. */
    const [problem, setProblem] = createSignal<string | null>(null)
    /** Where the active buffer stops making sense, and what it names. */
    const [check, setCheck] = createSignal<Check | null>(null)

    createEffect(
      on(
        () => [active().buffer, active().kind, targetOf(ws)] as const,
        ([lua, kind, slot]) => {
          if (!slot || !lua.trim()) {
            setPrepared(null)
            setProblem(null)
            setCheck(null)
            return
          }
          const timer = setTimeout(() => {
            io.tweakPrepare(lua, slot, true)
              .then((next) => {
                setPrepared(next)
                setProblem(null)
              })
              .catch((error) => {
                setPrepared(null)
                setProblem(message(error))
              })
            io.tweakCheck(lua, kind)
              .then(setCheck)
              .catch(() => setCheck(null))
          }, SETTLE_MS)
          onCleanup(() => clearTimeout(timer))
        },
      ),
    )

    /** What the room's game and engine know; see `lib/assist`. */
    const [assist, setAssist] = createSignal<Assist>(NO_ASSIST)

    // ---- what the reader does ----

    function open(id: DocId) {
      const doc = ws.docs[id]
      if (!doc) return
      setWs('active', id)
      // What was measured was the last document; nothing is, until this one is.
      setPrepared(null)
      setProblem(null)
      setCheck(null)
      // A draft is sent to a slot of its own kind; keep the choice if it fits.
      if (doc.origin === 'draft' && kindOf(ws.target) !== doc.kind)
        setWs('target', defaultTarget(doc.kind))
    }

    function edit(id: DocId, text: string) {
      const doc = ws.docs[id]
      if (!doc || doc.buffer === text) return
      setWs('docs', id, 'buffer', text)
    }

    function reset(id: DocId) {
      setWs('docs', id, resetDoc)
    }

    async function format(id: DocId) {
      const doc = ws.docs[id]
      if (!doc) return
      edit(id, await io.tweakFormat(doc.buffer, doc.kind))
    }

    /** `!bSet`, or a vote for one. Returns what went out. */
    async function send(direct: boolean): Promise<Prepared | null> {
      const doc = active()
      const slot = targetOf(ws)
      if (!slot) return null
      const out = await io.tweakSend(doc.buffer, slot, direct)
      if (direct && doc.origin === 'slot') setWs('docs', doc.id, sent)
      return out
    }

    async function clear() {
      const slot = targetOf(ws)
      if (slot) await io.tweakClear(slot)
    }

    async function refreshDrafts() {
      const names = await io.listDrafts()
      const texts = await Promise.all(names.map((name) => io.readDraft(name)))
      setWs(
        'docs',
        produce((docs) => {
          for (const [id, doc] of Object.entries(docs)) {
            if (doc.origin !== 'draft') continue
            // A file that is gone is forgotten, unless it holds unsaved work.
            if (!names.includes(doc.title) && !isDirty(doc)) delete docs[id]
          }
          names.forEach((name, at) => {
            const id = draftId(name)
            const known = docs[id]
            // Unsaved work is never overwritten by what is on disk.
            if (known && isDirty(known)) return
            docs[id] = draftDoc(name, texts[at]!)
          })
        }),
      )
    }

    /** Saves the active buffer as a draft, which becomes clean at that name. */
    async function saveDraft(name: string) {
      const doc = active()
      await io.saveDraft(name, doc.buffer)
      setWs('docs', draftId(name), savedAs(doc, name))
    }

    async function deleteDraft(name: string) {
      await io.deleteDraft(name)
      const id = draftId(name)
      if (ws.active === id) open(slotId(TWEAK_SLOTS[0]!))
      setWs(
        'docs',
        produce((docs) => {
          delete docs[id]
        }),
      )
    }

    const setFilter = (patch: Partial<Filter>) => setWs('filter', patch)
    const setCompare = (compare: Compare | null) => setWs('compare', compare)
    const diffText = io.tweakDiffText
    const setTarget = (key: string) => setWs('target', key)
    const setFullscreen = (on: boolean) => setWs('fullscreen', on)

    return {
      ws,
      active,
      items,
      modified,
      prepared,
      problem,
      check,
      assist,
      setAssist,
      decode,
      open,
      edit,
      reset,
      format,
      send,
      clear,
      refreshDrafts,
      saveDraft,
      deleteDraft,
      setFilter,
      setCompare,
      diffText,
      setTarget,
      setFullscreen,
      dispose,
    }
  })
}
