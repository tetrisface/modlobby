import { For, Show, createMemo, createSignal } from 'solid-js'
import type { Symbol } from '../../ipc/bindings/Symbol'

/**
 * What the payload names at the top level -- the unit keys of a tweakunits
 * table, the locals and functions of a tweakdefs script -- as a list that
 * jumps to the line. A twelve kilobyte tweak is a hundred units, and the one
 * you want is `corgolt4`.
 */
export function Outline(props: {
  symbols: Symbol[]
  onGoto: (line: number) => void
}) {
  const [query, setQuery] = createSignal('')
  const shown = createMemo(() => {
    const needle = query().trim().toLowerCase()
    if (needle === '') return props.symbols
    return props.symbols.filter((symbol) =>
      symbol.name.toLowerCase().includes(needle),
    )
  })

  return (
    <Show when={props.symbols.length > 0}>
      <details class='tweak-extra outline'>
        <summary>Outline · {props.symbols.length}</summary>
        <input
          class='outline-find'
          placeholder='Find a name'
          aria-label='Find a name'
          value={query()}
          onInput={(event) => setQuery(event.currentTarget.value)}
        />
        <div class='outline-list'>
          <For
            each={shown()}
            fallback={<p class='muted setup-empty'>Nothing by that name.</p>}
          >
            {(symbol) => (
              <button
                class='outline-item'
                title={`Line ${symbol.line}`}
                onClick={() => props.onGoto(symbol.line)}
              >
                <span class='outline-name'>{symbol.name}</span>
                <span class='outline-line'>{symbol.line}</span>
              </button>
            )}
          </For>
        </div>
      </details>
    </Show>
  )
}
