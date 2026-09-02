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
import { Glyph } from '../components/icons'
import { ResizeHandle } from '../components/ResizeHandle'
import { api, describeError } from '../ipc/client'
import { clamp, dragWidth, readWidth, writeWidth } from '../lib/resize'
import {
  ALL_TAB,
  MODDING_TAB,
  TWEAK_SLOTS,
  changedByTab,
  changedCount,
  defaultText,
  displayText,
  isOn,
  isTweakSlot,
  readModOptions,
  rowsByTab,
  rowsOf,
  tabs,
  type Changed,
  type Row,
  type Tab,
} from '../lib/setup'
import { pushNotice } from '../store/chat'
import { lobby } from '../store/lobby'
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
 */
export function Setup() {
  /**
   * The game's own option table, read from the copy installed on this machine
   * rather than shipped with the app — see `lib/setup`. Re-read when the room
   * changes game, which is also what keeps it matching the version in play.
   */
  const [catalogue] = createResource(
    () => lobby.battles[lobby.myBattle?.id ?? -1]?.gameName,
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

  const values = createMemo(() => readModOptions(lobby.myBattle?.scriptTags))

  /**
   * SPADS refuses `bSet` from a spectator outright and auto-converts it into a
   * vote for a player below level 100, so holding a seat is the honest gate.
   * What happens after the send is the host's call, not ours.
   */
  const editable = createMemo(() => {
    const me = lobby.me
    return me !== null && lobby.users[me]?.battleStatus?.player === true
  })

  const changed = createMemo(() =>
    tab()
      .groups.flatMap((entry) => rowsOf(entry, values()))
      .filter((row) => row.changed),
  )

  const everywhere = createMemo(() => changedByTab(TABS(), values()))
  const total = createMemo(() =>
    everywhere().reduce((sum, entry) => sum + entry.rows.length, 0),
  )

  const shown = createMemo(() => {
    const name = group()
    const found = tab().groups.find((entry) => entry.name === name)
    return found ? rowsOf(found, values()) : []
  })

  function open(next: Tab) {
    setTabKey(next.key)
    setGroup(null)
  }

  const [width, setWidth] = createSignal(readWidth(storage(), WIDTH_KEY))
  /** Once a width has been chosen by hand, the tabs stop deciding it. */
  let chosen = width() !== null
  let host: HTMLElement | undefined
  let strip: HTMLDivElement | undefined
  const bounds = () => ({
    min: NARROWEST,
    max: Math.max(NARROWEST, window.innerWidth - ROOM_KEEPS),
  })

  /**
   * As wide as it takes for the tabs to sit on one row: their widths and the
   * gaps between them, the strip's padding and the pane's border. Summed
   * from the tabs themselves, so the answer is the same whether they are
   * currently on one row or wrapped onto two.
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
          writeWidth(storage(), WIDTH_KEY, now)
        }}
      />

      <div class='setup-head'>
        <span class='t'>Setup</span>
        <span class='note'>
          {editable()
            ? 'a change is proposed to the host'
            : 'read-only · spectator'}
        </span>
        <Show when={editing()}>
          <button class='setup-back' onClick={() => setEditing(null)}>
            Settings
          </button>
        </Show>
      </div>

      <div class='setup-tabs' ref={strip}>
        <button
          class='setup-tab'
          classList={{ on: tab().key === ALL_TAB }}
          title={ALL.desc}
          onClick={() => open(ALL)}
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
                  on: tab().key === entry.key,
                  ours: entry.key === MODDING_TAB,
                }}
                title={entry.desc}
                onClick={() => open(entry)}
              >
                {entry.name}
                <Show when={count() > 0}>
                  <span class='badge'>{count()}</span>
                </Show>
              </button>
            )
          }}
        </For>
      </div>

      <Show when={!editing()} fallback={<Tweaks initial={editing()?.slot} />}>
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
                Changed<span class='c'>{changed().length}</span>
              </button>
              <For each={tab().groups}>
                {(entry) => (
                  <button
                    class='group'
                    classList={{ on: group() === entry.name }}
                    onClick={() => setGroup(entry.name)}
                  >
                    {entry.name || 'General'}
                    <span class='c'>{entry.options.length}</span>
                  </button>
                )}
              </For>
            </Show>
          </nav>

          <div class='setup-detail'>
            <Switch>
              <Match when={tab().key === ALL_TAB}>
                <Everywhere
                  changed={everywhere}
                  all={() => rowsByTab(TABS(), values(), false)}
                  editable={editable()}
                  onEdit={(slot) => setEditing({ slot })}
                />
              </Match>
              <Match when={group() === null}>
                <ChangedHere
                  rows={changed}
                  editable={editable()}
                  onShowAll={() => setGroup(firstGroup())}
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
    </aside>
  )

  function firstGroup() {
    return tab().groups[0]?.name ?? null
  }
}

/** `localStorage`, when the webview lets us at it. */
function storage(): Storage | null {
  try {
    return window.localStorage
  } catch {
    return null
  }
}

/**
 * The All tab: what is changed anywhere, under the tab it belongs to -- and,
 * on request, everything else beside it.
 */
function Everywhere(props: {
  changed: Accessor<Changed[]>
  all: Accessor<Changed[]>
  editable: boolean
  onEdit: (slot: string) => void
}) {
  const [unchanged, setUnchanged] = createSignal(false)
  const shown = () => (unchanged() ? props.all() : props.changed())
  return (
    <>
      <Show
        when={shown().length > 0}
        fallback={
          <p class='muted setup-empty'>Every setting is on BAR's default.</p>
        }
      >
        <For each={shown()}>
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
      <button class='setup-reveal' onClick={() => setUnchanged(!unchanged())}>
        {unchanged() ? 'Hide unchanged' : 'Show unchanged'}
      </button>
    </>
  )
}

function ChangedHere(props: {
  rows: Accessor<Row[]>
  editable: boolean
  onShowAll: () => void
  onEdit: (slot: string) => void
}) {
  return (
    <>
      <div class='setup-section'>
        <span>Changed from default</span>
        <span class='count'>{props.rows().length}</span>
      </div>
      <Show
        when={props.rows().length > 0}
        fallback={
          <p class='muted setup-empty'>
            Every setting in this tab is on BAR's default.
          </p>
        }
      >
        <Rows
          rows={props.rows()}
          editable={props.editable}
          onEdit={props.onEdit}
        />
      </Show>
      <button class='setup-reveal' onClick={props.onShowAll}>
        Show unchanged
      </button>
    </>
  )
}

/**
 * Settings as rows. A tweak slot is not a value anyone reads: its row ends
 * in the two things to do with it, copy and open, wherever the row appears.
 */
function Rows(props: {
  rows: Row[]
  editable: boolean
  onEdit: (slot: string) => void
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
              {row.option.name || row.option.key}
            </span>
            <Switch fallback={<span class='v'>{displayText(row)}</span>}>
              <Match when={isTweakSlot(row)}>
                <SlotActions
                  slot={row.option.key}
                  blob={row.current ?? ''}
                  onEdit={props.onEdit}
                />
              </Match>
              <Match when={props.editable}>
                <Control row={row} />
              </Match>
            </Switch>
          </div>
        )}
      </For>
    </div>
  )
}

/**
 * The two things to do with a tweak slot: copy the `!bSet` command that
 * carries it -- what a spectator can hand to someone with a seat -- and, the
 * wider target, open it in the editor, which is what the slot is for whether
 * it is full or still empty. `label` puts the slot's name on the button, for
 * a grid where the row has no other place for it.
 */
function SlotActions(props: {
  slot: string
  blob: string
  label?: string
  onEdit: (slot: string) => void
}) {
  const empty = () => props.blob === ''

  async function copy() {
    try {
      await navigator.clipboard.writeText(`!bSet ${props.slot} ${props.blob}`)
      pushNotice('info', `!bSet ${props.slot} copied`)
    } catch (error) {
      pushNotice('warning', `copy: ${describeError(error)}`)
    }
  }

  return (
    <span class='slot-cell' classList={{ filled: !empty() }}>
      <button
        class='slot-copy'
        title={`Copy the !bSet command for ${props.slot}`}
        disabled={empty()}
        onClick={() => void copy()}
      >
        <Glyph id='act-copy' />
      </button>
      <button
        class='slot-open'
        title={
          empty()
            ? `Write a tweak into ${props.slot}`
            : `Open ${props.slot} in the editor`
        }
        onClick={() => props.onEdit(props.slot)}
      >
        <Glyph id='act-pen' />
        <Show when={props.label}>
          {(label) => <span class='kk'>{label()}</span>}
        </Show>
        <span class='vv'>{empty() ? '—' : `${props.blob.length} B`}</span>
      </button>
    </span>
  )
}

/**
 * One editable setting. The value is sent when it is committed, never on
 * every keystroke: each send is a chat command the whole room sees.
 */
function Control(props: { row: Row }) {
  const value = () => props.row.current ?? defaultText(props.row.option)

  async function set(next: string) {
    if (next === value()) return
    try {
      await api.setOption(props.row.option.key, next)
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
        <select
          class='v-edit'
          value={value()}
          onChange={(event) => void set(event.currentTarget.value)}
        >
          <For each={props.row.option.items ?? []}>
            {(item) => <option value={item.key}>{item.name}</option>}
          </For>
        </select>
      </Match>
    </Switch>
  )
}

/** The twenty slots as a grid, every one of them, filled or not. */
function TweakSlots(props: {
  values: Record<string, string>
  onEdit: (slot?: string) => void
}) {
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

      <div class='setup-note'>
        Spectators cannot set a modoption or call a vote on one. The editor
        formats, diffs and copies the command for someone who can.
      </div>
      <button onClick={() => props.onEdit()}>Open editor</button>
    </>
  )
}
