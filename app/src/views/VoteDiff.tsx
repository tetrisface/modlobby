import { Show, createEffect, createSignal, onCleanup } from 'solid-js'
import type { DiffView } from '../ipc/bindings/DiffView'
import type { Kind } from '../ipc/bindings/Kind'
import { api, describeError } from '../ipc/client'
import { createDiffEditor, disposeDiff, monaco } from '../editor/monaco'

/**
 * Two slot values side by side, both formatted first so the diff shows real
 * changes rather than minification noise. Rust does the formatting and the
 * line diff; Monaco renders it.
 */
export function VoteDiff(props: {
  kind: Kind
  current: string
  proposed: string
  title?: string
}) {
  const [diff, setDiff] = createSignal<DiffView | null>(null)
  const [error, setError] = createSignal<string | null>(null)
  const [formatted, setFormatted] = createSignal<[string, string] | null>(null)
  let host: HTMLDivElement | undefined
  let editor: monaco.editor.IStandaloneDiffEditor | undefined

  createEffect(() => {
    const { kind, current, proposed } = props
    setError(null)
    void (async () => {
      try {
        const [view, before, after] = await Promise.all([
          api.tweakDiff(kind, current, proposed),
          decode(kind, current),
          decode(kind, proposed),
        ])
        setDiff(view)
        setFormatted([before, after])
      } catch (err) {
        setError(describeError(err))
      }
    })()
  })

  createEffect(() => {
    const pair = formatted()
    if (!host || !pair) return
    if (editor) disposeDiff(editor)
    editor = createDiffEditor(host, pair[0], pair[1])
  })
  onCleanup(() => editor && disposeDiff(editor))

  return (
    <section class='diff'>
      <header>
        <h2>{props.title ?? 'Change'}</h2>
        <Show when={diff()}>
          {(d) => (
            <span class='muted'>
              +{d().added} −{d().removed}
              <button
                onClick={() => navigator.clipboard.writeText(d().unified)}
              >
                Copy patch
              </button>
            </span>
          )}
        </Show>
      </header>
      <Show when={error()}>
        {(message) => <p class='error'>{message()}</p>}
      </Show>
      <div class='diff-editor' ref={host} />
    </section>
  )
}

async function decode(kind: Kind, blob: string): Promise<string> {
  if (!blob) return ''
  try {
    return (await api.tweakDecode(blob, kind)).formatted
  } catch {
    return blob
  }
}
