import { For, Show, createMemo, createResource, createSignal } from 'solid-js'
import { api, describeError } from '../ipc/client'
import { mapNames } from '../lib/maps'
import { pushNotice } from '../store/chat'

/** As many as the list draws before searching is the faster way to find one. */
const SHOWN = 400

/**
 * The maps on this machine, to pick one to play.
 *
 * Installed maps are archive file names — lowercased and underscored — and a
 * room names its map the way the engine does. Nothing on disk records the
 * capitalisation, so it comes from BAR's published index, the same one the
 * minimaps come from. Offline the file name is offered as-is and says so,
 * which is what the old skirmish form did and is still the best guess there
 * is.
 */
export function MapPicker(props: {
  /** The room's map, so the one already chosen is marked. */
  current: string
  onPick: (springName: string) => void
  onClose: () => void
}) {
  const [options] = createResource(() =>
    api.skirmishOptions().catch(() => null),
  )
  const [names] = createResource(mapNames)
  const [search, setSearch] = createSignal('')

  const maps = createMemo(() => {
    const files = options()?.maps ?? []
    const index = names() ?? {}
    return files.map((file) => ({
      file,
      spring: index[file] ?? null,
      label: index[file] ?? file,
    }))
  })

  const shown = createMemo(() => {
    const needle = search().trim().toLowerCase()
    const all = maps()
    const matching = needle
      ? all.filter((entry) => entry.label.toLowerCase().includes(needle))
      : all
    return matching.slice(0, SHOWN)
  })

  const unknown = createMemo(() => maps().some((entry) => !entry.spring))

  return (
    <div class='sheet' onMouseDown={props.onClose}>
      <div
        class='sheet-card map-picker'
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header class='ed-head'>
          <h2>Map</h2>
          <input
            class='search'
            placeholder={`Search ${maps().length} installed maps`}
            value={search()}
            onInput={(event) => setSearch(event.currentTarget.value)}
          />
          <button type='button' onClick={props.onClose}>
            Close
          </button>
        </header>

        <Show when={unknown()}>
          <p class='muted setup-note'>
            The map index could not be reached, so some of these are listed by
            file name. Picking one still works; the engine may not find it.
          </p>
        </Show>

        <div class='map-list'>
          <For
            each={shown()}
            fallback={<p class='muted setup-empty'>No map matches.</p>}
          >
            {(entry) => (
              <button
                class='room-tab'
                classList={{
                  on: props.current === (entry.spring ?? entry.file),
                }}
                onClick={() => props.onPick(entry.spring ?? entry.file)}
              >
                <span class='room-name'>{entry.label}</span>
              </button>
            )}
          </For>
        </div>
      </div>
    </div>
  )
}

/** Picks it, saying why if the room would not take it. */
export async function pickMap(
  set: (name: string) => Promise<void>,
  name: string,
): Promise<void> {
  try {
    await set(name)
  } catch (error) {
    pushNotice('warning', `map: ${describeError(error)}`)
  }
}
