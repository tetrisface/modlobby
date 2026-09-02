import {
  For,
  Show,
  createMemo,
  createSignal,
  onCleanup,
  onMount,
} from 'solid-js'
import { dropModel } from '../../editor/monaco'
import { unknownUnits } from '../../lib/assist'
import { describeError } from '../../ipc/client'
import {
  defaultCompare,
  draftId,
  draftNameFor,
  resolveSide,
  sideOptions,
  slotKey,
  targetOf,
  type Side,
} from '../../lib/tweakspace'
import { pushNotice } from '../../store/chat'
import { lobby } from '../../store/lobby'
import { tweakspace as space } from '../../store/tweakspaceInstance'
import { VoteDiff } from '../VoteDiff'
import { ComparePane, type SideText } from './ComparePane'
import { DocList } from './DocList'
import { EditorHost, type Goto } from './EditorHost'
import { Outline } from './Outline'
import { Problems } from './Problems'
import { Toolbar, type Copyable } from './Toolbar'

const COPIED: Record<Copyable, string> = {
  lua: 'Lua',
  minified: 'Minified Lua',
  blob: 'base64url',
  command: '!bSet command',
}

/** Monaco widgets that take Escape for themselves before the window may. */
const MONACO_WANTS_ESCAPE =
  '.tweak-full .monaco-editor :is(.suggest-widget, .find-widget, .parameter-hints-widget, .rename-box).visible'

/**
 * The workspace, composed: the list, the bar, the editor, and under it what
 * the room is doing to the open slot. Reads the store; everything below it
 * gets props.
 */
export function Workspace() {
  const [busy, setBusy] = createSignal(false)
  const [goto, setGoto] = createSignal<Goto | null>(null)
  const doc = space.active
  const jump = (line: number, column = 1) =>
    setGoto({ line, column, at: Date.now() })

  /** SPADS takes `bSet` only from a player; see `Setup`. */
  const seated = createMemo(() => {
    const me = lobby.me
    return me !== null && lobby.users[me]?.battleStatus?.player === true
  })

  /** The vote in progress, when it proposes the open slot. */
  const proposal = createMemo(() => {
    const vote = lobby.myBattle?.vote
    const open = doc()
    if (vote?.proposal.type !== 'setOption' || open.origin !== 'slot')
      return null
    return vote.proposal.key === open.title ? vote.proposal.value : null
  })

  const history = () => lobby.myBattle?.history ?? []

  /** Unit keys this game does not have -- only meaningful in a units table. */
  const warnings = createMemo(() =>
    doc().kind === 'units'
      ? unknownUnits(space.check()?.outline ?? [], space.assist().units)
      : [],
  )
  const changes = createMemo(() => {
    const open = doc()
    if (open.origin !== 'slot') return []
    return history()
      .filter((change) => change.key === open.title)
      .reverse()
  })

  /** A side's text, decoding a blob on the way. */
  async function resolve(side: Side): Promise<SideText | null> {
    const found = resolveSide(space.ws, side, history(), proposal())
    if (!found) return null
    if ('lua' in found)
      return { label: found.label, kind: found.kind, text: found.lua }
    const view = await space.decode(found.blob, found.kind).catch(() => null)
    return {
      label: found.label,
      kind: found.kind,
      text: view?.formatted ?? found.blob,
    }
  }

  const toggleCompare = () =>
    space.setCompare(space.ws.compare ? null : defaultCompare(space.ws))

  async function act(what: string, run: () => Promise<void>) {
    setBusy(true)
    try {
      await run()
    } catch (error) {
      pushNotice('warning', `${what}: ${describeError(error)}`)
    } finally {
      setBusy(false)
    }
  }

  const copy = (what: Copyable) =>
    act('copy', async () => {
      const ready = space.prepared()
      const text = {
        lua: doc().buffer,
        minified: ready?.minified,
        blob: ready?.blob,
        command: ready?.command,
      }[what]
      if (text === undefined) return
      await navigator.clipboard.writeText(text)
      pushNotice('info', `${COPIED[what]} copied`)
    })

  const save = (name: string) =>
    act('save draft', async () => {
      await space.saveDraft(name)
      pushNotice('info', `saved draft "${name}"`)
    })

  const send = (direct: boolean) =>
    act('send', async () => {
      const slot = targetOf(space.ws)
      const out = await space.send(direct)
      if (out && slot)
        pushNotice(
          'info',
          `sent ${out.gauge.command} chars to ${slotKey(slot)}`,
        )
    })

  const remove = () =>
    act('delete draft', async () => {
      const name = doc().title
      await space.deleteDraft(name)
      dropModel(draftId(name))
    })

  // Escape leaves fullscreen. Caught before the overlay's own Escape handler
  // and left alone when a Monaco widget is open and wants it.
  onMount(() => {
    const keys = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || !space.ws.fullscreen) return
      if (event.defaultPrevented || document.querySelector(MONACO_WANTS_ESCAPE))
        return
      event.preventDefault()
      space.setFullscreen(false)
    }
    window.addEventListener('keydown', keys, true)
    onCleanup(() => window.removeEventListener('keydown', keys, true))
  })

  return (
    <section class='tweaks'>
      <DocList
        items={space.items()}
        active={space.ws.active}
        filter={space.ws.filter}
        modified={space.modified()}
        onSelect={space.open}
        onFilter={space.setFilter}
      />

      <div class='tweak-main'>
        <Toolbar
          doc={doc()}
          prepared={space.prepared()}
          problem={space.problem()}
          busy={busy()}
          fullscreen={space.ws.fullscreen}
          comparing={space.ws.compare !== null}
          seated={seated()}
          target={space.ws.target}
          onFormat={() => void act('format', () => space.format(doc().id))}
          onReset={() => space.reset(doc().id)}
          onSave={(name) => void save(name)}
          onDelete={doc().origin === 'draft' ? () => void remove() : undefined}
          onFullscreen={space.setFullscreen}
          onCompare={toggleCompare}
          onCopy={(what) => void copy(what)}
          onTarget={space.setTarget}
          onSend={(direct) => void send(direct)}
          onClear={() => void act('clear', () => space.clear())}
        />

        <Show
          when={space.ws.compare}
          fallback={
            <EditorHost
              doc={doc()}
              problems={space.check()?.problems ?? []}
              warnings={warnings()}
              assist={space.assist()}
              goto={goto()}
              onEdit={space.edit}
              onSave={() => void save(draftNameFor(doc()))}
            />
          }
        >
          {(compare) => (
            <ComparePane
              compare={compare()}
              options={sideOptions(space.ws, history(), proposal())}
              resolve={resolve}
              diff={space.diffText}
              onChange={space.setCompare}
              onClose={() => space.setCompare(null)}
            />
          )}
        </Show>

        <Problems
          problems={space.check()?.problems ?? []}
          warnings={warnings()}
          notes={doc().notes}
          onGoto={jump}
        />

        <Outline symbols={space.check()?.outline ?? []} onGoto={jump} />

        <Show when={space.prepared()}>
          {(ready) => (
            <div class='gauge' classList={{ over: !ready().gauge.fits }}>
              <span>raw {ready().gauge.raw} B</span>
              <span>minified {ready().gauge.minified} B</span>
              <span>blob {ready().gauge.blob}</span>
              <span>
                command {ready().gauge.command} / {ready().gauge.cap}
              </span>
            </div>
          )}
        </Show>

        <Show when={proposal()}>
          {(value) => (
            <div class='tweak-extra'>
              <VoteDiff
                kind={doc().kind}
                current={doc().blob ?? ''}
                proposed={value()}
                title='A vote proposes this slot'
              />
              <button
                class='link'
                onClick={() =>
                  space.setCompare({
                    left: { vote: true },
                    right: { doc: doc().id, text: 'buffer' },
                  })
                }
              >
                Compare it with your edit
              </button>
            </div>
          )}
        </Show>

        <Show when={changes().length > 0}>
          <details class='tweak-extra history'>
            <summary>Changes this session · {changes().length}</summary>
            <For each={changes()}>
              {(change) => (
                <div class='history-row'>
                  <span>
                    #{change.seq} {change.by ?? 'someone'} ·{' '}
                    {change.from.length} → {change.to.length} chars
                  </span>
                  <button
                    class='link'
                    onClick={() =>
                      space.setCompare({
                        left: { history: change.seq, which: 'from' },
                        right: { history: change.seq, which: 'to' },
                      })
                    }
                  >
                    Compare
                  </button>
                </div>
              )}
            </For>
          </details>
        </Show>
      </div>
    </section>
  )
}
