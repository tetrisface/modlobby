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
import type { Kind as TweakKind } from '../ipc/bindings/Kind'
import { api, describeError } from '../ipc/client'
import {
  MODDING_TAB,
  TWEAK_SLOTS,
  changedCount,
  defaultText,
  displayText,
  isOn,
  readModOptions,
  rowsOf,
  tabs,
  type Row,
  type Tab,
} from '../lib/setup'
import { pushNotice } from '../store/chat'
import { lobby } from '../store/lobby'
import { Tweaks } from './Tweaks'

const TABS = tabs()
const TWEAK_GROUP = 'Tweak slots'

/**
 * The room's settings, in BAR's own tabs and groups.
 *
 * A tab opens on what differs from BAR's default and keeps the rest one click
 * away: 221 options ship across these tabs, and the four somebody changed are
 * the four that decide how the game plays. Everything is read-only until we
 * hold a seat — SPADS grants `bSet` as `battle,pv:player:stopped`, so a
 * spectator can neither set a value nor call a vote on one.
 */
export function Setup(props: {
  wide: boolean
  onWide: (wide: boolean) => void
}) {
  const [tab, setTab] = createSignal<Tab>(TABS[0]!)
  const [group, setGroup] = createSignal<string | null>(null)

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

  const shown = createMemo(() => {
    const name = group()
    const found = tab().groups.find((entry) => entry.name === name)
    return found ? rowsOf(found, values()) : []
  })

  /** The slots group, expanded: the pane becomes the whole tweak workspace. */
  const editing = () =>
    props.wide && tab().key === MODDING_TAB && group() === TWEAK_GROUP

  function open(next: Tab) {
    setTab(next)
    setGroup(null)
    if (next.key !== MODDING_TAB) props.onWide(false)
  }

  function editTweaks() {
    setTab(TABS.find((entry) => entry.key === MODDING_TAB)!)
    setGroup(TWEAK_GROUP)
    props.onWide(true)
  }

  return (
    <aside class='setup'>
      <div class='setup-head'>
        <span class='t'>Setup</span>
        <span class='note'>
          {editable()
            ? 'a change is proposed to the host'
            : 'read-only · spectator'}
        </span>
        <Show when={tab().key === MODDING_TAB}>
          <button
            class='setup-wide'
            onClick={() => props.onWide(!props.wide)}
            title={
              props.wide ? 'Show the rosters again' : 'Give Modding the room'
            }
          >
            {props.wide ? 'Collapse' : 'Expand'}
          </button>
        </Show>
      </div>

      <div class='setup-tabs'>
        <For each={TABS}>
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

      <Show when={!editing()} fallback={<Tweaks />}>
        <div class='setup-body'>
          <nav class='groups'>
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
          </nav>

          <div class='setup-detail'>
            <Show
              when={group() !== null}
              fallback={
                <Changed
                  rows={changed}
                  editable={editable()}
                  onShowAll={() => setGroup(firstGroup())}
                />
              }
            >
              <Show
                when={tab().key === MODDING_TAB && group() === TWEAK_GROUP}
                fallback={<Rows rows={shown()} editable={editable()} />}
              >
                <TweakSlots values={values()} onEdit={editTweaks} />
              </Show>
            </Show>
          </div>
        </div>
      </Show>
    </aside>
  )

  function firstGroup() {
    return tab().groups[0]?.name ?? null
  }
}

function Changed(props: {
  rows: Accessor<Row[]>
  editable: boolean
  onShowAll: () => void
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
        <Rows rows={props.rows()} editable={props.editable} />
      </Show>
      <button class='setup-reveal' onClick={props.onShowAll}>
        Show every setting
      </button>
    </>
  )
}

function Rows(props: { rows: Row[]; editable: boolean }) {
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
            <Show
              when={props.editable}
              fallback={<span class='v'>{displayText(row)}</span>}
            >
              <Control row={row} />
            </Show>
          </div>
        )}
      </For>
    </div>
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

/**
 * The twenty slots, and the Lua in whichever one is open. Editing happens in
 * the full editor; this is the view a spectator can act on — read it, format
 * it, copy the command someone with a seat can run.
 */
function TweakSlots(props: {
  values: Record<string, string>
  onEdit: () => void
}) {
  const [open, setOpen] = createSignal<string | null>(null)

  const filled = createMemo(() =>
    TWEAK_SLOTS.filter((key) => (props.values[key] ?? '') !== ''),
  )

  let luaHost: HTMLPreElement | undefined

  const [view] = createResource(
    () => {
      const key = open()
      const blob = key === null ? '' : (props.values[key] ?? '')
      if (blob === '') return null
      const kind: TweakKind = key!.startsWith('tweakdefs') ? 'defs' : 'units'
      return { blob, kind }
    },
    (request) => api.tweakDecode(request.blob, request.kind),
  )

  // `Show` keeps the same <pre> across slots, so a new tweak inherits the last
  // one's scroll and opens somewhere in its middle.
  createEffect(() => {
    view()
    if (luaHost) luaHost.scrollTop = 0
  })

  return (
    <>
      <div class='setup-section'>
        <span>Slots</span>
        <span class='count'>{filled().length} filled</span>
      </div>

      <div class='slot-grid'>
        <For each={TWEAK_SLOTS}>
          {(key) => {
            const blob = () => props.values[key] ?? ''
            return (
              <button
                class='slot-cell'
                classList={{ filled: blob() !== '', on: open() === key }}
                onClick={() => setOpen(open() === key ? null : key)}
              >
                <span class='kk'>{key}</span>
                <span class='vv'>
                  {blob() === '' ? '—' : `${blob().length} B`}
                </span>
              </button>
            )
          }}
        </For>
      </div>

      <Show when={view()}>
        {(tweak) => (
          <div class='slot-view'>
            <div class='slot-view-head'>
              <span class='nm'>{tweak().name ?? open()}</span>
              <span class='hash'>{tweak().summary}</span>
            </div>
            <pre class='slot-lua' ref={luaHost}>
              {tweak().formatted}
            </pre>
            <Show when={tweak().diagnostics.length > 0}>
              <div class='slot-warn'>
                <For each={tweak().diagnostics}>
                  {(note) => <div>{String(note)}</div>}
                </For>
              </div>
            </Show>
          </div>
        )}
      </Show>

      <div class='setup-note'>
        Spectators cannot set a modoption or call a vote on one. Open the editor
        to format, diff and copy the command.
      </div>
      <button onClick={props.onEdit}>Open editor</button>
    </>
  )
}
