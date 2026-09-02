import { For, Show, createEffect, createSignal, onCleanup } from 'solid-js'
import { createDiffEditor, disposeDiff, monaco } from '../../editor/monaco'
import type { DiffView } from '../../ipc/bindings/DiffView'
import type { Kind } from '../../ipc/bindings/Kind'
import {
  parseSide,
  sideKey,
  type Compare,
  type Side,
  type SideGroup,
  type SideOption,
} from '../../lib/tweakspace'

export type SideText = { label: string; kind: Kind; text: string }

const GROUPS: SideGroup[] = ['Slots', 'Drafts', 'Changes this session', 'Vote']

/**
 * Any two things side by side. Rust formats both and counts the lines; Monaco
 * draws them. Takes the editor's place rather than squeezing in under it: two
 * tweaks abreast need the width, and the models keep the edits meanwhile.
 */
export function ComparePane(props: {
  compare: Compare
  options: SideOption[]
  resolve: (side: Side) => Promise<SideText | null>
  diff: (kind: Kind, left: string, right: string) => Promise<DiffView>
  onChange: (compare: Compare) => void
  onClose: () => void
}) {
  const [sides, setSides] = createSignal<[SideText, SideText] | null>(null)
  const [view, setView] = createSignal<DiffView | null>(null)
  const [abreast, setAbreast] = createSignal(true)
  const [missing, setMissing] = createSignal<string | null>(null)
  let host: HTMLDivElement | undefined
  let editor: monaco.editor.IStandaloneDiffEditor | undefined

  createEffect(() => {
    const { left, right } = props.compare
    let current = true
    onCleanup(() => {
      current = false
    })
    void (async () => {
      const [a, b] = await Promise.all([
        props.resolve(left),
        props.resolve(right),
      ])
      if (!current) return
      if (!a || !b) {
        setSides(null)
        setView(null)
        setMissing('One side of this is no longer there.')
        return
      }
      setMissing(null)
      setSides([a, b])
      setView(await props.diff(a.kind, a.text, b.text).catch(() => null))
    })()
  })

  createEffect(() => {
    const pair = sides()
    const sideBySide = abreast()
    if (!host || !pair) return
    if (editor) disposeDiff(editor)
    editor = createDiffEditor(host, pair[0].text, pair[1].text, {
      renderSideBySide: sideBySide,
    })
  })
  onCleanup(() => editor && disposeDiff(editor))

  const pick = (which: 'left' | 'right', key: string) => {
    const side = parseSide(key)
    if (side) props.onChange({ ...props.compare, [which]: side })
  }

  const swap = () =>
    props.onChange({ left: props.compare.right, right: props.compare.left })

  return (
    <div class='compare'>
      <div class='compare-head'>
        <SidePick
          label='Left'
          value={sideKey(props.compare.left)}
          other={sideKey(props.compare.right)}
          arrow='→'
          options={props.options}
          onPick={(key) => pick('left', key)}
        />
        <button class='tweak-tool' title='Swap sides' onClick={swap}>
          ⇄
        </button>
        <SidePick
          label='Right'
          value={sideKey(props.compare.right)}
          other={sideKey(props.compare.left)}
          arrow='←'
          options={props.options}
          onPick={(key) => pick('right', key)}
        />
        <span class='spacer' />
        <Show when={view()}>
          {(counted) => (
            <span class='gauge-badge'>
              +{counted().added} −{counted().removed}
            </span>
          )}
        </Show>
        <button
          class='tweak-tool'
          onClick={() => setAbreast(!abreast())}
          title='Side by side, or one column with the changes inline'
        >
          {abreast() ? 'Inline' : 'Side by side'}
        </button>
        <button
          class='tweak-tool'
          disabled={!view()}
          onClick={() => {
            const unified = view()?.unified
            if (unified) void navigator.clipboard.writeText(unified)
          }}
        >
          Copy patch
        </button>
        <button
          class='tweak-tool'
          onClick={props.onClose}
          title='Back to the editor'
        >
          Close
        </button>
      </div>
      <Show when={missing()}>
        {(text) => <p class='muted setup-empty'>{text()}</p>}
      </Show>
      <div class='compare-editor' ref={host} />
    </div>
  )
}

/**
 * One side's menu. The entry the other side already shows carries an arrow
 * pointing that way, so the menu says where each thing is rather than
 * leaving the reader to remember.
 */
function SidePick(props: {
  label: string
  value: string
  /** What the other side is showing. */
  other: string
  /** Which way the other side is: `→` from the left menu, `←` from the right. */
  arrow: string
  options: SideOption[]
  onPick: (key: string) => void
}) {
  return (
    <select
      aria-label={props.label}
      value={props.value}
      onChange={(event) => props.onPick(event.currentTarget.value)}
    >
      <For each={GROUPS}>
        {(group) => (
          <Show when={props.options.some((option) => option.group === group)}>
            <optgroup label={group}>
              <For
                each={props.options.filter((option) => option.group === group)}
              >
                {(option) => (
                  <option value={option.key}>
                    {option.key === props.other
                      ? `${props.arrow} ${option.label}`
                      : option.label}
                  </option>
                )}
              </For>
            </optgroup>
          </Show>
        )}
      </For>
    </select>
  )
}
