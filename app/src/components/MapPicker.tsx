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

  const maps = createMemo(() => {
    const files = options()?.maps ?? []
    const index = names() ?? {}
    return files.map((file) => ({
      value: index[file] ?? file,
      label: index[file] ?? file,
      known: index[file] !== undefined,
    }))
  })

  return (
    <Picker
      title='Map'
      current={props.current}
      items={maps()}
      note={
        maps().some((entry) => !entry.known)
          ? 'The map index could not be reached, so some of these are listed by file name. Picking one still works; the engine may not find it.'
          : null
      }
      onPick={props.onPick}
      onClose={props.onClose}
    />
  )
}

/**
 * The games or engines this machine has.
 *
 * Newest first, which is what rapid's own ordering makes them and what
 * somebody looking for "the current one" wants at the top.
 */
export function VersionPicker(props: {
  what: 'Game' | 'Engine'
  current: string
  onPick: (version: string) => void
  onClose: () => void
}) {
  const [options] = createResource(() =>
    api.skirmishOptions().catch(() => null),
  )
  const versions = createMemo(() => {
    const held = props.what === 'Game' ? options()?.games : options()?.engines
    return (held ?? []).map((value) => ({ value, label: value, known: true }))
  })

  return (
    <Picker
      title={props.what}
      current={props.current}
      items={versions()}
      note={
        versions().length === 0
          ? `Nothing is installed to play ${props.what === 'Game' ? 'with' : 'on'}. The room offers a download for what it needs.`
          : null
      }
      onPick={props.onPick}
      onClose={props.onClose}
    />
  )
}

type Item = { value: string; label: string; known: boolean }

/** One list, searchable, over the room. */
function Picker(props: {
  title: string
  current: string
  items: Item[]
  note: string | null
  onPick: (value: string) => void
  onClose: () => void
}) {
  const [search, setSearch] = createSignal('')
  const shown = createMemo(() => {
    const needle = search().trim().toLowerCase()
    const matching = needle
      ? props.items.filter((entry) =>
          entry.label.toLowerCase().includes(needle),
        )
      : props.items
    return matching.slice(0, SHOWN)
  })

  return (
    <div class='sheet' onMouseDown={props.onClose}>
      <div
        class='sheet-card map-picker'
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header class='ed-head'>
          <h2>{props.title}</h2>
          <input
            class='search'
            placeholder={`Search ${props.items.length}`}
            value={search()}
            onInput={(event) => setSearch(event.currentTarget.value)}
          />
          <button type='button' onClick={props.onClose}>
            Close
          </button>
        </header>

        <Show when={props.note}>
          {(note) => <p class='muted setup-note'>{note()}</p>}
        </Show>

        <div class='map-list'>
          <For
            each={shown()}
            fallback={<p class='muted setup-empty'>Nothing matches.</p>}
          >
            {(entry) => (
              <button
                class='room-tab'
                classList={{ on: props.current === entry.value }}
                onClick={() => props.onPick(entry.value)}
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
export async function picked(
  set: (name: string) => Promise<void>,
  what: string,
  name: string,
): Promise<void> {
  try {
    await set(name)
  } catch (error) {
    pushNotice('warning', `${what}: ${describeError(error)}`)
  }
}
