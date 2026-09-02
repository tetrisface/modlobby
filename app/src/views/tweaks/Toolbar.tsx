import { For, Show, createSignal } from 'solid-js'
import type { Prepared } from '../../ipc/bindings/Prepared'
import { TWEAK_SLOTS } from '../../lib/setup'
import { draftNameFor, isDirty, type Doc } from '../../lib/tweakspace'

export type Copyable = 'lua' | 'minified' | 'blob' | 'command'

/**
 * What can be done to the open document, and what state it is in.
 *
 * Everything here is a prop: the bar knows nothing about the store or Rust,
 * which is what lets a test press each button and see the right call.
 */
export function Toolbar(props: {
  doc: Doc
  prepared: Prepared | null
  /** Why the buffer could not be measured -- a syntax error, most days. */
  problem: string | null
  busy: boolean
  fullscreen: boolean
  /** Whether the main area is the comparison rather than the editor. */
  comparing: boolean
  /** Whether we hold a seat, which is what `!bSet` and a vote need. */
  seated: boolean
  /** The slot a draft is sent to. */
  target: string
  onFormat: () => void
  onReset: () => void
  onSave: (name: string) => void
  /** Present for a draft, which is the one kind of document that can go. */
  onDelete?: () => void
  onFullscreen: (on: boolean) => void
  onCompare: () => void
  onCopy: (what: Copyable) => void
  onTarget: (key: string) => void
  onSend: (direct: boolean) => void
  onClear: () => void
}) {
  const [draftName, setDraftName] = createSignal('')
  const dirty = () => isDirty(props.doc)
  const fits = () => props.prepared?.gauge.fits === true
  const slots = () =>
    TWEAK_SLOTS.filter((key) => key.startsWith(`tweak${props.doc.kind}`))

  function save() {
    props.onSave(draftName().trim() || draftNameFor(props.doc))
    setDraftName('')
  }

  return (
    <header class='tweak-bar'>
      <div class='tweak-bar-row'>
        <span class='doc-kind'>{props.doc.kind}</span>
        <h1>{props.doc.title}</h1>
        <Show when={props.doc.name}>
          {(name) => <span class='tweak-name'>{name()}</span>}
        </Show>
        <Show when={dirty()}>
          <span class='doc-tag dirty'>edited</span>
        </Show>
        <Show when={props.doc.stale}>
          <span
            class='doc-tag'
            title='Somebody set this slot while you were editing it'
          >
            room moved
          </span>
        </Show>
        <span class='spacer' />
        <Show when={props.prepared}>
          {(ready) => (
            <span
              class='gauge-badge'
              classList={{ over: !ready().gauge.fits }}
              title='Characters of the !bSet command, against what the server keeps'
            >
              {ready().gauge.command} / {ready().gauge.cap}
            </span>
          )}
        </Show>
        <Show when={props.problem}>
          {(text) => (
            <span class='gauge-badge over' title={text()}>
              will not load
            </span>
          )}
        </Show>
        <button
          class='tweak-tool'
          classList={{ on: props.comparing }}
          title='Any two of: a slot, a draft, your edit, a change, the vote'
          onClick={props.onCompare}
        >
          {props.comparing ? 'Editor' : 'Compare'}
        </button>
        <button
          class='tweak-tool'
          title={
            props.fullscreen ? 'Back into the pane (Esc)' : 'Fill the window'
          }
          onClick={() => props.onFullscreen(!props.fullscreen)}
        >
          {props.fullscreen ? 'Exit fullscreen' : 'Fullscreen'}
        </button>
      </div>

      <div class='tweak-bar-row actions'>
        <button
          class='tweak-tool'
          onClick={props.onFormat}
          disabled={props.busy}
        >
          Format
        </button>
        <button
          class='tweak-tool'
          onClick={props.onReset}
          disabled={props.busy || !dirty()}
          title='Back to what the room holds, or what the draft file says'
        >
          Reset
        </button>
        <span class='tweak-save'>
          <input
            class='draft-name'
            placeholder={
              props.doc.origin === 'draft' ? props.doc.title : 'draft name'
            }
            aria-label='Draft name'
            value={draftName()}
            onInput={(event) => setDraftName(event.currentTarget.value)}
            onKeyDown={(event) => event.key === 'Enter' && save()}
          />
          <button class='tweak-tool' onClick={save} disabled={props.busy}>
            Save draft
          </button>
        </span>
        <span class='tweak-copy'>
          <span class='muted'>Copy</span>
          <button class='link' onClick={() => props.onCopy('lua')}>
            Lua
          </button>
          <button
            class='link'
            disabled={!props.prepared}
            onClick={() => props.onCopy('minified')}
          >
            minified
          </button>
          <button
            class='link'
            disabled={!props.prepared}
            onClick={() => props.onCopy('blob')}
          >
            base64url
          </button>
          <button
            class='link'
            disabled={!props.prepared}
            onClick={() => props.onCopy('command')}
          >
            !bSet
          </button>
        </span>
        <Show when={props.onDelete}>
          {(remove) => (
            <button class='link drop' onClick={() => remove()()}>
              Delete draft
            </button>
          )}
        </Show>
        <span class='spacer' />
        <Show when={props.doc.origin === 'draft'}>
          <label class='tweak-target'>
            <span class='muted'>to</span>
            <select
              value={props.target}
              onChange={(event) => props.onTarget(event.currentTarget.value)}
            >
              <For each={slots()}>
                {(key) => <option value={key}>{key}</option>}
              </For>
            </select>
          </label>
        </Show>
        <button
          class='primary'
          disabled={props.busy || !fits() || !props.seated}
          title={
            props.seated
              ? 'Set the slot now'
              : 'SPADS only takes this from a player; copy the command instead'
          }
          onClick={() => props.onSend(true)}
        >
          Send !bSet
        </button>
        <button
          disabled={props.busy || !fits() || !props.seated}
          onClick={() => props.onSend(false)}
        >
          Call a vote
        </button>
        <button
          disabled={props.busy || !props.seated}
          title='Set the slot to nothing'
          onClick={props.onClear}
        >
          Clear slot
        </button>
      </div>
    </header>
  )
}
