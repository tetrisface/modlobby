import { Select } from '../components/Select'
import {
  For,
  Match,
  createEffect,
  Show,
  Switch,
  createMemo,
  createResource,
  createSignal,
  type Accessor,
} from 'solid-js'
import { ActionCell, CellButton } from '../components/ActionCell'
import { ResizeHandle } from '../components/ResizeHandle'
import { api, describeError } from '../ipc/client'
import { BOX_OVERRIDE } from '../lib/boxes'
import {
  clamp,
  dragWidth,
  localStore,
  readWidth,
  writeWidth,
} from '../lib/resize'
import {
  ALL_TAB,
  GENERAL_GROUP,
  MAP_TAB,
  MODDING_TAB,
  TWEAK_SLOTS,
  changedByTab,
  changedCount,
  defaultText,
  displayText,
  isCleared,
  isOn,
  isMapOption,
  isTweakSlot,
  readModOptions,
  label,
  rowsByGroup,
  rowsByTab,
  rowsOf,
  searchRows,
  tabs,
  type Changed,
  type Row,
  type Section,
  type Tab,
} from '../lib/setup'
import { pushNotice } from '../store/chat'
import { PasteBanner } from './PasteBanner'
import { Presets } from './Presets'
import { useRoom, type RoomModel } from './room/model'
import { Tweaks } from './tweaks/Tweaks'

const TWEAK_GROUP = 'Tweak slots'

/** Shown before the game is installed, when there is no table to read. */
const NO_TABS: Tab = { key: '', name: '', desc: '', groups: [] }

/** The tab across all tabs; `groups` is empty because it draws its own body. */
const ALL: Tab = {
  key: ALL_TAB,
  name: 'All',
  desc: "Every setting that differs from BAR's default, whichever tab it is in.",
  groups: [],
}

/** The All tab's sections: one per tab, headed by the tab's name. */
const byTabName = (entries: Changed[]): Section[] =>
  entries.map(({ tab, rows }) => ({ name: tab.name, rows }))

const WIDTH_KEY = 'modlobby.setupWidth'
const NARROWEST = 420
/** What the rosters and the chat keep, however wide the pane is dragged. */
const ROOM_KEEPS = 480
/** `.setup-tabs`' side padding and the gap between tabs, as the stylesheet has them. */
const STRIP_PADDING = 14
const TAB_GAP = 2

/**
 * The room's settings, in BAR's own tabs and groups.
 *
 * Opens on All -- what differs from BAR's default, across every tab -- and
 * keeps the rest one click away: 221 options ship across these tabs, and the
 * four somebody changed are the four that decide how the game plays.
 * Everything is read-only until we hold a seat — SPADS grants `bSet` as
 * `battle,pv:player:stopped`, so a spectator can neither set a value nor call
 * a vote on one.
 *
 * The pane carries its own width: dragged by the grip on its left edge and
 * remembered, or, the first time, measured so that its tabs sit on one row.
 *
 * Presets share the pane as a second face of it: they are read from and
 * written to this room, so beside the settings is where they belong.
 */
export function Setup() {
  const room = useRoom()
  const [pane, setPane] = createSignal<'setup' | 'presets'>('setup')

  /**
   * The game's own option table, read from the copy installed on this machine
   * rather than shipped with the app — see `lib/setup`. Re-read when the room
   * changes game, which is also what keeps it matching the version in play.
   */
  const [catalogue] = createResource(
    () => room.battle()?.gameName,
    (game) => api.gameModOptions(game).catch(() => []),
  )
  const TABS = createMemo(() => tabs(catalogue() ?? []))

  /**
   * The chosen tab is held as a key rather than as the tab itself: the table
   * is re-read when the room changes game, and a held object would then be a
   * tab from the previous catalogue.
   */
  const [tabKey, setTabKey] = createSignal<string>(ALL_TAB)
  const tab = (): Tab => {
    if (tabKey() === ALL_TAB) return ALL
    return (
      TABS().find((entry) => entry.key === tabKey()) ?? TABS()[0] ?? NO_TABS
    )
  }
  const [group, setGroup] = createSignal<string | null>(null)

  /** The slot being edited, or nothing; the pane is the editor while one is. */
  const [editing, setEditing] = createSignal<{ slot?: string } | null>(null)

  const values = createMemo(() => readModOptions(room.my()?.scriptTags))

  /**
   * SPADS refuses `bSet` from a spectator outright and auto-converts it into a
   * vote for a player below level 100, so holding a seat is the honest gate.
   * What happens after the send is the host's call, not ours. Where no host is
   * being asked, there is nobody to refuse and a spectator may still set up
   * the game they are about to watch.
   */
  const editable = createMemo(() => {
    if (!room.caps.spads) return true
    const me = room.me()
    return me !== null && room.users()[me]?.battleStatus?.player === true
  })

  const everywhere = createMemo(() => changedByTab(TABS(), values()))
  const total = createMemo(() =>
    everywhere().reduce((sum, entry) => sum + entry.rows.length, 0),
  )

  const shown = createMemo(() => {
    const name = group()
    const found = tab().groups.find((entry) => entry.name === name)
    return found ? rowsOf(found, values()) : []
  })

  /**
   * A search across every tab. While it holds something the body is the
   * matches, under the tab each lives in; the tab and group that were open
   * are untouched, so clearing it lands where you were.
   */
  const [needle, setNeedle] = createSignal('')
  const searching = () => needle().trim() !== ''
  const found = createMemo(() => searchRows(TABS(), values(), needle()))

  /**
   * Whether the Changed view is showing everything else as well. Reset with
   * the tab: opening one means landing on what is changed in it.
   */
  const [unchanged, setUnchanged] = createSignal(false)

  function open(next: Tab) {
    setTabKey(next.key)
    setGroup(null)
    setUnchanged(false)
  }

  const [width, setWidth] = createSignal(readWidth(localStore(), WIDTH_KEY))
  /** Once a width has been chosen by hand, the tabs stop deciding it. */
  let chosen = width() !== null
  let host: HTMLElement | undefined
  let strip: HTMLDivElement | undefined
  let search: HTMLInputElement | undefined
  const bounds = () => ({
    min: NARROWEST,
    max: Math.max(NARROWEST, window.innerWidth - ROOM_KEEPS),
  })

  /**
   * As wide as it takes for the tabs and the search to sit on one row: their
   * widths and the gaps between them, the strip's padding and the pane's
   * border. Summed from the tabs themselves, so the answer is the same
   * whether they are currently on one row or wrapped onto two.
   */
  function fit() {
    if (chosen || !strip) return
    const tabs = [...strip.children] as HTMLElement[]
    if (tabs.length === 0) return
    const wanted =
      tabs.reduce((sum, tab) => sum + tab.offsetWidth, 0) +
      TAB_GAP * (tabs.length - 1) +
      STRIP_PADDING * 2 +
      1
    setWidth(clamp(Math.min(wanted, window.innerWidth / 2), bounds()))
  }

  // Whenever the tabs or their badges change -- and once the display font
  // is in, since a tab measured in the fallback face comes out narrower.
  createEffect(() => {
    TABS()
    total()
    if (chosen) return
    fit()
    void document.fonts?.ready.then(fit)
  })

  return (
    <aside
      class='setup'
      ref={host}
      style={{
        '--setup-width': width() === null ? undefined : `${width()}px`,
      }}
    >
      <ResizeHandle
        label='Resize the setup pane'
        onStart={() => width() ?? host?.getBoundingClientRect().width ?? 0}
        onMove={(start, x0, x) => setWidth(dragWidth(start, x0, x, bounds()))}
        onEnd={() => {
          const now = width()
          if (now === null) return
          chosen = true
          writeWidth(localStore(), WIDTH_KEY, now)
        }}
      />

      <div class='setup-head'>
        <button
          class='pane-tab'
          classList={{ on: pane() === 'setup' }}
          onClick={() => setPane('setup')}
        >
          Setup
        </button>
        <button
          class='pane-tab'
          classList={{ on: pane() === 'presets' }}
          onClick={() => setPane('presets')}
        >
          Presets
        </button>
        <Show when={pane() === 'setup'}>
          <span class='note'>{noteOfPane(room, editable())}</span>
          <Show when={editing()}>
            <button class='setup-back' onClick={() => setEditing(null)}>
              Settings
            </button>
          </Show>
        </Show>
      </div>

      {/* The paste banner is the chat throttle showing through; nothing is
          throttled where nothing is said. Above the pane switch, because
          loading a preset is a paste too -- and it is the pane you are on
          while you wait for one. */}
      <Show when={room.caps.spads}>
        <PasteBanner />
      </Show>

      <Show when={pane() === 'setup'} fallback={<Presets />}>
        <div class='setup-tabs' ref={strip}>
          <button
            class='setup-tab'
            classList={{ on: !searching() && tab().key === ALL_TAB }}
            title={ALL.desc}
            onClick={() => {
              setNeedle('')
              open(ALL)
            }}
          >
            {ALL.name}
            <Show when={total() > 0}>
              <span class='badge'>{total()}</span>
            </Show>
          </button>
          <For each={TABS()}>
            {(entry) => {
              const count = createMemo(() => changedCount(entry, values()))
              return (
                <button
                  class='setup-tab'
                  classList={{
                    on: !searching() && tab().key === entry.key,
                    ours: entry.key === MODDING_TAB || entry.key === MAP_TAB,
                  }}
                  title={entry.desc}
                  onClick={() => {
                    setNeedle('')
                    open(entry)
                  }}
                >
                  {entry.name}
                  <Show when={count() > 0}>
                    <span class='badge'>{count()}</span>
                  </Show>
                </button>
              )
            }}
          </For>
          <Show when={!editing()}>
            <div class='setup-search'>
              <input
                ref={search}
                placeholder='Search settings'
                aria-label='Search settings'
                value={needle()}
                onInput={(event) => setNeedle(event.currentTarget.value)}
                onKeyDown={(event) => {
                  if (event.key === 'Escape') setNeedle('')
                }}
              />
              <Show when={searching()}>
                <button
                  type='button'
                  class='clear'
                  title='Clear search'
                  aria-label='Clear search'
                  onClick={() => {
                    setNeedle('')
                    search?.focus()
                  }}
                >
                  ×
                </button>
              </Show>
            </div>
          </Show>
        </div>

        <Show when={!editing()} fallback={<Tweaks initial={editing()?.slot} />}>
          <Show
            when={!searching()}
            fallback={
              <div class='setup-detail setup-found'>
                <Found
                  found={found}
                  needle={needle()}
                  editable={editable()}
                  onEdit={(slot) => setEditing({ slot })}
                />
              </div>
            }
          >
            <div class='setup-body'>
              <nav class='groups'>
                <Show
                  when={tab().key !== ALL_TAB}
                  fallback={
                    <For each={everywhere()}>
                      {(entry) => (
                        <button class='group' onClick={() => open(entry.tab)}>
                          {entry.tab.name}
                          <span class='c'>{entry.rows.length}</span>
                        </button>
                      )}
                    </For>
                  }
                >
                  <button
                    class='group'
                    classList={{ on: group() === null }}
                    onClick={() => setGroup(null)}
                  >
                    Changed
                    <span class='c'>{changedCount(tab(), values())}</span>
                  </button>
                  <For each={tab().groups}>
                    {(entry) => (
                      <button
                        class='group'
                        classList={{ on: group() === entry.name }}
                        onClick={() => setGroup(entry.name)}
                      >
                        {entry.name || GENERAL_GROUP}
                        <span class='c'>{entry.options.length}</span>
                      </button>
                    )}
                  </For>
                </Show>
              </nav>

              <div class='setup-detail'>
                <Switch>
                  <Match when={tab().key === ALL_TAB}>
                    <Changes
                      changed={() => byTabName(everywhere())}
                      all={() => byTabName(rowsByTab(TABS(), values(), false))}
                      empty="Every setting is on BAR's default."
                      unchanged={unchanged()}
                      onToggle={() => setUnchanged(!unchanged())}
                      editable={editable()}
                      onEdit={(slot) => setEditing({ slot })}
                    />
                  </Match>
                  <Match when={group() === null}>
                    <Changes
                      changed={() => rowsByGroup(tab(), values(), true)}
                      all={() => rowsByGroup(tab(), values(), false)}
                      empty="Every setting in this tab is on BAR's default."
                      unchanged={unchanged()}
                      onToggle={() => setUnchanged(!unchanged())}
                      editable={editable()}
                      onEdit={(slot) => setEditing({ slot })}
                    />
                  </Match>
                  <Match
                    when={tab().key === MODDING_TAB && group() === TWEAK_GROUP}
                  >
                    <TweakSlots
                      values={values()}
                      onEdit={(slot) => setEditing({ slot })}
                    />
                  </Match>
                  <Match when={true}>
                    <Rows
                      rows={shown()}
                      editable={editable()}
                      onEdit={(slot) => setEditing({ slot })}
                    />
                  </Match>
                </Switch>
              </div>
            </div>
          </Show>
        </Show>
      </Show>
    </aside>
  )
}

/**
 * What the pane says a change will do, which is not the same sentence when
 * there is a host to persuade and when there is not.
 */
function noteOfPane(room: RoomModel, editable: boolean): string {
  if (!editable) return 'Read-only · spectator'
  if (room.caps.spads) return 'A change is proposed to the host'
  return 'A change takes effect here'
}

/**
 * What is changed, under the heading it belongs to -- and, on request,
 * everything else beside it. On the All tab the headings are tabs; inside a
 * tab they are its groups.
 *
 * The toggle reveals rows in place. It used to open the tab's first group
 * instead, which lost the changed rows it had been showing and, on Modding,
 * landed on the slot grid rather than the rows it had promised.
 */
function Changes(props: {
  changed: Accessor<Section[]>
  all: Accessor<Section[]>
  /** What to say when nothing here is changed. */
  empty: string
  unchanged: boolean
  onToggle: () => void
  editable: boolean
  onEdit: (slot: string) => void
}) {
  const shown = () => (props.unchanged ? props.all() : props.changed())
  return (
    <>
      <Show
        when={shown().length > 0}
        fallback={<p class='muted setup-empty'>{props.empty}</p>}
      >
        <For each={shown()}>
          {(entry) => (
            <>
              <div class='setup-section'>
                <span>{entry.name}</span>
                <span class='count'>{entry.rows.length}</span>
              </div>
              <Rows
                rows={entry.rows}
                editable={props.editable}
                onEdit={props.onEdit}
              />
            </>
          )}
        </For>
      </Show>
      <button class='setup-reveal' onClick={props.onToggle}>
        {props.unchanged ? 'Hide unchanged' : 'Show unchanged'}
      </button>
    </>
  )
}

/** What the search turned up, under the tab each row lives in. */
function Found(props: {
  found: Accessor<Changed[]>
  needle: string
  editable: boolean
  onEdit: (slot: string) => void
}) {
  return (
    <Show
      when={props.found().length > 0}
      fallback={
        <p class='muted setup-empty'>
          Nothing matches "{props.needle.trim()}".
        </p>
      }
    >
      <For each={props.found()}>
        {(entry) => (
          <>
            <div class='setup-section'>
              <span>{entry.tab.name}</span>
              <span class='count'>{entry.rows.length}</span>
            </div>
            <Rows
              rows={entry.rows}
              editable={props.editable}
              onEdit={props.onEdit}
            />
          </>
        )}
      </For>
    </Show>
  )
}

/**
 * Settings as rows. A tweak slot is not a value anyone reads: its row ends
 * in the two things to do with it, copy and open, wherever the row appears.
 *
 * Exported because an AI's own options are the same kind of table -- BAR's
 * `modoptions.lua` and an engine AI's `AIOptions.lua` are one format -- and
 * drawing them the same way is the whole reason they can be. `set` is where
 * a changed value goes; the room's own settings are the default.
 */
export function Rows(props: {
  rows: Row[]
  editable: boolean
  onEdit: (slot: string) => void
  set?: (key: string, value: string) => Promise<void>
}) {
  return (
    <div class='setup-rows'>
      <For
        each={props.rows}
        fallback={<p class='muted setup-empty'>Nothing here.</p>}
      >
        {(row) => (
          <div class='opt' classList={{ changed: row.changed }}>
            <span class='mark' />
            <span class='k' title={row.option.desc ?? ''}>
              {label(row.option)}
            </span>
            <Switch fallback={<span class='v'>{displayText(row)}</span>}>
              <Match when={isMapOption(row.option)}>
                <MapValue row={row} onEdit={props.onEdit} />
              </Match>
              <Match when={isTweakSlot(row)}>
                <SlotActions
                  slot={row.option.key}
                  blob={row.current ?? ''}
                  onEdit={props.onEdit}
                />
              </Match>
              <Match when={props.editable}>
                <Control row={row} set={props.set} />
              </Match>
            </Switch>
          </div>
        )}
      </For>
    </div>
  )
}

/**
 * A map-metadata row's value in words. The blob is base64url(zlib(json)) and
 * says nothing; Rust decodes it (`boxes::describe_map_option`) and answers
 * with what it holds. The two SPADS sets from the map's metadata are
 * read-only; the override is somebody's own, drawn on the map or, from here,
 * typed as JSON in the editor.
 */
function MapValue(props: { row: Row; onEdit: (slot: string) => void }) {
  const [words] = createResource(
    () => [props.row.option.key, props.row.current ?? ''] as const,
    ([key, raw]) => api.describeMapOption(key, raw).catch(() => null),
  )
  const text = () => (words.loading ? '…' : (words() ?? displayText(props.row)))
  return (
    <Show
      when={props.row.option.key === BOX_OVERRIDE}
      fallback={
        <span class='v' title={props.row.current ?? ''}>
          {text()}
        </span>
      }
    >
      <SlotActions
        slot={props.row.option.key}
        blob={props.row.current ?? ''}
        text={text()}
        onEdit={props.onEdit}
      />
    </Show>
  )
}

/**
 * The two things to do with a slot: copy the `!bSet` command that carries it
 * -- what a spectator can hand to someone with a seat -- and, the wider
 * target, open it in the editor, which is what the slot is for whether it is
 * full or still empty. `label` puts the slot's name on the button, for a grid
 * where the row has no other place for it; `text` says what it holds, where
 * the blob's size would not.
 */
function SlotActions(props: {
  slot: string
  blob: string
  label?: string
  text?: string
  onEdit: (slot: string) => void
}) {
  const empty = () => isCleared(props.blob)

  async function copy() {
    try {
      await navigator.clipboard.writeText(`!bSet ${props.slot} ${props.blob}`)
      pushNotice('info', `!bSet ${props.slot} copied`)
    } catch (error) {
      pushNotice('warning', `copy: ${describeError(error)}`)
    }
  }

  return (
    <ActionCell filled={!empty()}>
      <CellButton
        icon='act-copy'
        title={`Copy the !bSet command for ${props.slot}`}
        disabled={empty()}
        onClick={() => void copy()}
      />
      <CellButton
        class='slot-open'
        icon='act-pen'
        title={
          empty()
            ? `Write into ${props.slot}`
            : `Open ${props.slot} in the editor`
        }
        onClick={() => props.onEdit(props.slot)}
      >
        <Show when={props.label}>
          {(label) => <span class='kk'>{label()}</span>}
        </Show>
        <span class='vv'>
          {props.text ?? (empty() ? '—' : `${props.blob.length} B`)}
        </span>
      </CellButton>
    </ActionCell>
  )
}

/**
 * One editable setting. The value is sent when it is committed, never on
 * every keystroke: each send is a chat command the whole room sees.
 */
function Control(props: {
  row: Row
  set?: (key: string, value: string) => Promise<void>
}) {
  const room = useRoom()
  const value = () => props.row.current ?? defaultText(props.row.option)

  async function set(next: string) {
    if (next === value()) return
    try {
      const put = props.set ?? room.io.setOption
      await put(props.row.option.key, next)
    } catch (error) {
      pushNotice('warning', `${props.row.option.key}: ${describeError(error)}`)
    }
  }

  return (
    <Switch fallback={<span class='v'>{displayText(props.row)}</span>}>
      <Match when={props.row.option.type === 'bool'}>
        <input
          class='v-edit'
          type='checkbox'
          checked={isOn(value())}
          onChange={(event) =>
            void set(event.currentTarget.checked ? '1' : '0')
          }
        />
      </Match>
      <Match when={props.row.option.type === 'number'}>
        <input
          class='v-edit'
          type='number'
          value={value()}
          min={props.row.option.min ?? undefined}
          max={props.row.option.max ?? undefined}
          step={props.row.option.step ?? undefined}
          onChange={(event) => void set(event.currentTarget.value)}
        />
      </Match>
      <Match when={props.row.option.type === 'list'}>
        <Select
          class='v-edit'
          value={value()}
          onChange={(event) => void set(event.currentTarget.value)}
        >
          <For each={props.row.option.items ?? []}>
            {(item) => <option value={item.key}>{item.name}</option>}
          </For>
        </Select>
      </Match>
    </Switch>
  )
}

/** The twenty slots as a grid, every one of them, filled or not. */
function TweakSlots(props: {
  values: Record<string, string>
  onEdit: (slot?: string) => void
}) {
  const room = useRoom()
  const filled = createMemo(() =>
    TWEAK_SLOTS.filter((key) => (props.values[key] ?? '') !== ''),
  )

  return (
    <>
      <div class='setup-section'>
        <span>Slots</span>
        <span class='count'>{filled().length} filled</span>
      </div>

      <div class='slot-grid'>
        <For each={TWEAK_SLOTS}>
          {(key) => (
            <SlotActions
              slot={key}
              blob={props.values[key] ?? ''}
              label={key}
              onEdit={props.onEdit}
            />
          )}
        </For>
      </div>

      <Show
        when={room.caps.spads}
        fallback={
          <div class='setup-note'>
            The editor formats and diffs a tweak before it goes in. Nothing
            leaves this machine.
          </div>
        }
      >
        <div class='setup-note'>
          Spectators cannot set a modoption or call a vote on one. The editor
          formats, diffs and copies the command for someone who can.
        </div>
      </Show>
      <button onClick={() => props.onEdit()}>Open editor</button>
    </>
  )
}
