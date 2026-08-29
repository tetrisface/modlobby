import { A } from '@solidjs/router'
import {
  For,
  Show,
  createMemo,
  createResource,
  createSignal,
  type Accessor,
} from 'solid-js'
import type { Kind as TweakKind } from '../ipc/bindings/Kind'
import { api } from '../ipc/client'
import {
  MODDING_TAB,
  TWEAK_SLOTS,
  changedCount,
  displayText,
  readModOptions,
  rowsOf,
  tabs,
  type Row,
  type Tab,
} from '../lib/setup'
import { lobby } from '../store/lobby'

const TABS = tabs()

/**
 * The room's settings, in BAR's own tabs and groups.
 *
 * A tab opens on what differs from BAR's default and keeps the rest one click
 * away: 221 options ship across these tabs, and the four somebody changed are
 * the four that decide how the game plays. Everything is read-only until we
 * hold a seat — SPADS grants `bSet` as `battle,pv:player:stopped`, so a
 * spectator can neither set a value nor call a vote on one.
 */
export function Setup() {
  const [tab, setTab] = createSignal<Tab>(TABS[0]!)
  const [group, setGroup] = createSignal<string | null>(null)

  const values = createMemo(() => readModOptions(lobby.myBattle?.scriptTags))

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

  function open(next: Tab) {
    setTab(next)
    setGroup(null)
  }

  return (
    <aside class='setup'>
      <div class='setup-head'>
        <span class='t'>Setup</span>
        <span class='note'>read-only · spectator</span>
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
                onShowAll={() => setGroup(firstGroup())}
              />
            }
          >
            <Show
              when={tab().key === MODDING_TAB && group() === 'Tweak slots'}
              fallback={<Rows rows={shown()} />}
            >
              <TweakSlots values={values()} />
            </Show>
          </Show>
        </div>
      </div>
    </aside>
  )

  function firstGroup() {
    return tab().groups[0]?.name ?? null
  }
}

function Changed(props: { rows: Accessor<Row[]>; onShowAll: () => void }) {
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
        <Rows rows={props.rows()} />
      </Show>
      <button class='setup-reveal' onClick={props.onShowAll}>
        Show every setting
      </button>
    </>
  )
}

function Rows(props: { rows: Row[] }) {
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
            <span class='v'>{displayText(row)}</span>
          </div>
        )}
      </For>
    </div>
  )
}

/**
 * The twenty slots, and the Lua in whichever one is open. Editing happens in
 * the full editor; this is the view a spectator can act on — read it, format
 * it, copy the command someone with a seat can run.
 */
function TweakSlots(props: { values: Record<string, string> }) {
  const [open, setOpen] = createSignal<string | null>(null)

  const filled = createMemo(() =>
    TWEAK_SLOTS.filter((key) => (props.values[key] ?? '') !== ''),
  )

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
            <pre class='slot-lua'>{tweak().formatted}</pre>
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
      <A class='button' href='/room/tweaks'>
        Open editor
      </A>
    </>
  )
}
