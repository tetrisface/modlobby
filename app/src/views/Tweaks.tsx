import {
  For,
  Show,
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
} from 'solid-js'
import type { Prepared } from '../ipc/bindings/Prepared'
import type { Slot } from '../ipc/bindings/Slot'
import type { TweakView } from '../ipc/bindings/TweakView'
import { SLOTS, api, describeError } from '../ipc/client'
import { createEditor, monaco, seed } from '../editor/monaco'
import { pushNotice } from '../store/chat'
import { lobby } from '../store/lobby'
import { settings } from '../store/settings'
import { VoteDiff } from './VoteDiff'

const MODOPTION = 'game/modoptions/'

export function Tweaks() {
  const [selected, setSelected] = createSignal<string>(
    settings()?.tweaks.defaultSlot ?? 'tweakdefs1',
  )
  const [view, setView] = createSignal<TweakView | null>(null)
  const [buffer, setBuffer] = createSignal('')
  const [dirty, setDirty] = createSignal(false)
  const [prepared, setPrepared] = createSignal<Prepared | null>(null)
  const [busy, setBusy] = createSignal(false)
  const [drafts, setDrafts] = createSignal<string[]>([])
  const [draftName, setDraftName] = createSignal('')
  let host: HTMLDivElement | undefined
  let editor: monaco.editor.IStandaloneCodeEditor | undefined
  let seeding = false

  const entry = createMemo(() => SLOTS.find((s) => s.key === selected()))
  const value = (key: string) =>
    lobby.myBattle?.scriptTags[`${MODOPTION}${key}`] ?? ''
  const current = createMemo(() => value(selected()))

  /** The vote in progress, when it proposes the slot being edited. */
  const proposal = createMemo(() => {
    const vote = lobby.myBattle?.vote
    if (vote?.proposal.type !== 'setOption') return null
    return vote.proposal.key === selected() ? vote.proposal.value : null
  })

  const changes = createMemo(() =>
    (lobby.myBattle?.history ?? [])
      .filter((h) => h.key === selected())
      .reverse(),
  )

  createEffect(() => {
    if (!host || editor) return
    editor = createEditor(host, '')
    editor.onDidChangeModelContent(() => {
      setBuffer(editor?.getValue() ?? '')
      if (!seeding) setDirty(true)
    })
    editor.addCommand(
      monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS,
      () => void save(),
    )
  })
  onCleanup(() => editor?.dispose())

  // Reload the slot whenever the selection or the room's value changes, unless
  // there is unsaved work in the buffer.
  createEffect(() => {
    const blob = current()
    const kind = entry()?.kind
    if (!kind) return
    if (dirty()) return
    void (async () => {
      if (!blob) {
        setView(null)
        replace('')
        return
      }
      try {
        const decoded = await api.tweakDecode(blob, kind)
        setView(decoded)
        replace(decoded.formatted)
      } catch (error) {
        setView(null)
        pushNotice('warning', `${selected()}: ${describeError(error)}`)
      }
    })()
  })

  // The gauge follows the buffer, debounced: every keystroke would minify and
  // encode the whole payload otherwise.
  createEffect(() => {
    const lua = buffer()
    const slot = entry()?.slot
    if (!slot || !lua.trim()) {
      setPrepared(null)
      return
    }
    const timer = setTimeout(() => {
      api
        .tweakPrepare(lua, slot, true)
        .then(setPrepared)
        .catch(() => setPrepared(null))
    }, 250)
    onCleanup(() => clearTimeout(timer))
  })

  createEffect(() => {
    void api
      .listDrafts()
      .then(setDrafts)
      .catch(() => setDrafts([]))
  })

  function replace(text: string) {
    seeding = true
    if (editor) seed(editor, text)
    else setBuffer(text)
    setBuffer(text)
    setDirty(false)
    seeding = false
  }

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

  const format = () =>
    act('format', async () => {
      const kind = entry()?.kind
      if (!kind) return
      const formatted = await api.tweakFormat(buffer(), kind)
      const wasDirty = dirty()
      replace(formatted)
      setDirty(wasDirty)
    })

  const send = (direct: boolean) =>
    act('send', async () => {
      const slot = entry()?.slot
      if (!slot) return
      const sent = await api.tweakSend(buffer(), slot, direct)
      pushNotice('info', `sent ${sent.gauge.command} chars to ${selected()}`)
      setDirty(false)
    })

  const clear = () =>
    act('clear', async () => {
      const slot = entry()?.slot
      if (slot) await api.tweakClear(slot)
    })

  const save = () =>
    act('save draft', async () => {
      const name = draftName().trim() || view()?.name || selected()
      await api.saveDraft(name, buffer())
      setDrafts(await api.listDrafts())
      setDraftName(name)
      pushNotice('info', `saved draft "${name}"`)
    })

  const openDraft = (name: string) =>
    act('open draft', async () => {
      replace(await api.readDraft(name))
      setDraftName(name)
      setDirty(true)
    })

  const copy = (label: string, text: string) =>
    act('copy', async () => {
      await navigator.clipboard.writeText(text)
      pushNotice('info', `${label} copied`)
    })

  return (
    <section class='tweaks'>
      <aside class='slots'>
        <h2>Slots</h2>
        <For each={SLOTS}>
          {(s) => {
            const blob = () => value(s.key)
            return (
              <button
                class='slot'
                classList={{
                  active: s.key === selected(),
                  filled: blob().length > 0,
                }}
                onClick={() => {
                  setDirty(false)
                  setSelected(s.key)
                }}
              >
                <span class='slot-key'>{s.key}</span>
                <Show when={blob()} fallback={<span class='muted'>empty</span>}>
                  <span class='slot-len'>{blob().length}</span>
                </Show>
              </button>
            )
          }}
        </For>
      </aside>

      <div class='tweak-main'>
        <header class='tweak-header'>
          <div>
            <h1>{selected()}</h1>
            <p class='muted'>
              <Show when={view()} fallback='empty slot'>
                {(v) => (
                  <>
                    {v().name ?? 'unnamed'} · {v().summary}
                  </>
                )}
              </Show>
              <Show when={dirty()}> · unsaved</Show>
            </p>
          </div>
          <div class='tweak-actions'>
            <button onClick={format} disabled={busy()}>
              Format
            </button>
            <input
              class='draft-name'
              placeholder='draft name'
              value={draftName()}
              onInput={(e) => setDraftName(e.currentTarget.value)}
            />
            <button onClick={save} disabled={busy()}>
              Save draft
            </button>
          </div>
        </header>

        <Show when={view()?.diagnostics.length}>
          <For each={view()?.diagnostics}>
            {(d) => (
              <p class='error'>
                This payload contains {d.count} `_`, which the game reads as `=`
                — what it loads is not what is stored here.
              </p>
            )}
          </For>
        </Show>

        <div class='tweak-editor' ref={host} />

        <Show when={prepared()}>
          {(p) => (
            <div class='gauge' classList={{ over: !p().gauge.fits }}>
              <span>raw {p().gauge.raw} B</span>
              <span>minified {p().gauge.minified} B</span>
              <span>blob {p().gauge.blob}</span>
              <span>
                command {p().gauge.command} / {p().gauge.cap}
              </span>
              <span class='spacer' />
              <button onClick={() => copy('Minified Lua', p().minified)}>
                Copy minified
              </button>
              <button onClick={() => copy('base64url', p().blob)}>
                Copy base64url
              </button>
              <button onClick={() => copy('!bSet command', p().command)}>
                Copy !bSet
              </button>
            </div>
          )}
        </Show>

        <div class='tweak-send'>
          <button
            class='primary'
            disabled={busy() || !prepared()?.gauge.fits}
            onClick={() => send(true)}
          >
            Send !bSet
          </button>
          <button
            disabled={busy() || !prepared()?.gauge.fits}
            onClick={() => send(false)}
          >
            Call a vote
          </button>
          <button disabled={busy() || !current()} onClick={clear}>
            Clear slot
          </button>
          <span class='muted'>
            SPADS only accepts these from a player at trust level 100, or a boss
            — as a spectator expect a refusal; copy the command instead.
          </span>
        </div>

        <Show when={proposal()}>
          {(value) => (
            <VoteDiff
              kind={entry()?.kind ?? 'defs'}
              current={current()}
              proposed={value()}
              title='A vote proposes this slot'
            />
          )}
        </Show>

        <Show when={changes().length}>
          <section class='history'>
            <h2>Changes this session</h2>
            <For each={changes()}>
              {(change) => (
                <details>
                  <summary>
                    #{change.seq} {change.by ?? 'someone'} ·{' '}
                    {change.from.length} → {change.to.length} chars
                  </summary>
                  <VoteDiff
                    kind={entry()?.kind ?? 'defs'}
                    current={change.from}
                    proposed={change.to}
                  />
                </details>
              )}
            </For>
          </section>
        </Show>

        <Show when={drafts().length}>
          <div class='drafts'>
            <span class='muted'>Drafts:</span>
            <For each={drafts()}>
              {(name) => (
                <span class='draft'>
                  <button onClick={() => openDraft(name)}>{name}</button>
                  <button
                    class='drop'
                    title='delete'
                    onClick={() =>
                      act('delete draft', async () => {
                        await api.deleteDraft(name)
                        setDrafts(await api.listDrafts())
                      })
                    }
                  >
                    ×
                  </button>
                </span>
              )}
            </For>
          </div>
        </Show>
      </div>
    </section>
  )
}
