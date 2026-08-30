import {
  For,
  Show,
  createMemo,
  createResource,
  createSignal,
  onCleanup,
} from 'solid-js'
import { Ask } from '../components/Ask'
import type { Preset } from '../ipc/bindings/Preset'
import type { Sections } from '../ipc/bindings/Sections'
import { api, describeError } from '../ipc/client'
import {
  DEFAULT_SORT,
  type Column,
  naturalDescending,
  search,
  sort,
  tweakCount,
  when,
} from '../lib/presets'
import { pushNotice } from '../store/chat'
import { lobby } from '../store/lobby'

const COLUMNS: Array<[Column, string, string]> = [
  ['name', 'Name', 'What you called it'],
  ['map', 'Map', 'The map it was saved on'],
  ['options', 'Settings', 'How many settings it carries'],
  ['used', 'Last used', 'When it was last applied to a room'],
  ['updated', 'Updated', 'When it last changed'],
  ['created', 'Created', 'When it was first saved'],
]

const EVERYTHING: Sections = {
  map: true,
  modoptions: true,
  battle: true,
  startBoxes: true,
  bots: false,
  reset: true,
}

/**
 * Saved room setups.
 *
 * A table rather than a dropdown because these accumulate — sixty-five of them
 * in the file this was built against — and the questions people ask of them
 * are "which did I use last", "which was for this map" and "which is the big
 * one", none of which a list of names answers.
 */
export function Presets() {
  const [column, setColumn] = createSignal<Column>(DEFAULT_SORT)
  const [descending, setDescending] = createSignal(true)
  const [needle, setNeedle] = createSignal('')
  const [chosen, setChosen] = createSignal<string | null>(null)
  const [sections, setSections] = createSignal<Sections>(EVERYTHING)
  const [busy, setBusy] = createSignal(false)

  const [book, { mutate, refetch }] = createResource(async () => {
    try {
      return await api.listPresets()
    } catch (error) {
      pushNotice('warning', describeError(error))
      return { version: 1, presets: [] }
    }
  })
  const [chobbyPath] = createResource(() => api.chobbyPresetsPath())

  // A clock, so "3m ago" does not sit there being wrong.
  const [now, setNow] = createSignal(Date.now())
  const tick = setInterval(() => setNow(Date.now()), 30_000)
  onCleanup(() => clearInterval(tick))

  const rows = createMemo(() =>
    sort(search(book()?.presets ?? [], needle()), column(), descending()),
  )
  const selected = () => rows().find((preset) => preset.name === chosen())
  const inRoom = () => lobby.myBattle !== null

  function head(next: Column) {
    if (next === column()) return setDescending(!descending())
    setColumn(next)
    setDescending(naturalDescending(next))
  }

  async function act(what: string, run: () => Promise<unknown>) {
    setBusy(true)
    try {
      await run()
    } catch (error) {
      pushNotice('warning', `${what}: ${describeError(error)}`)
    } finally {
      setBusy(false)
    }
  }

  /** Which text the sheet is asking for, when it is open. */
  const [asking, setAsking] = createSignal<
    { kind: 'save' } | { kind: 'rename'; from: string } | null
  >(null)

  const save = (name: string) =>
    act('save', async () => {
      mutate(await api.savePreset(name))
      setChosen(name)
    })

  const apply = (preset: Preset) =>
    act('apply', async () => {
      const plan = await api.applyPreset(preset.name, sections())
      void refetch()
      const already = plan.alreadySet ? `, ${plan.alreadySet} already set` : ''
      pushNotice(
        'info',
        plan.lines.length === 0
          ? `${preset.name}: the room already matches`
          : `${preset.name}: sending ${plan.lines.length} commands${already}. ` +
              `About ${Math.ceil(plan.lines.length * 1.1)}s — the host only accepts so many at once.`,
      )
      if (plan.startBoxesUnsent)
        pushNotice(
          'warning',
          'the room already has start boxes; left as they are',
        )
    })

  return (
    <section class='presets'>
      <header class='toolbar'>
        <input
          class='search'
          placeholder='Search presets'
          value={needle()}
          onInput={(event) => setNeedle(event.currentTarget.value)}
        />
        <button
          disabled={!inRoom() || busy()}
          onClick={() => setAsking({ kind: 'save' })}
        >
          Save this room
        </button>
        <button
          disabled={busy()}
          title={chobbyPath() ?? 'no BAR data directory found'}
          onClick={() =>
            void act('import', async () => {
              const { book: next, skipped } = await api.importPresets(null)
              mutate(next)
              pushNotice(
                'info',
                skipped
                  ? `imported; ${skipped} left alone because you already had those names`
                  : 'imported from Chobby',
              )
            })
          }
        >
          Import from Chobby
        </button>
        <button
          disabled={busy() || !selected()}
          title={chobbyPath() ?? ''}
          onClick={() => {
            const preset = selected()
            if (!preset) return
            void act('export', async () => {
              await api.exportPresets(null, [preset.name])
              pushNotice('info', `${preset.name} written to Chobby's presets`)
            })
          }}
        >
          Export selected
        </button>
        <span class='spacer' />
        <span class='muted'>{rows().length} presets</span>
      </header>

      <div class='preset-table'>
        <div class='preset-row head'>
          <For each={COLUMNS}>
            {([key, label, hint]) => (
              <button
                class='preset-head'
                classList={{ on: column() === key }}
                title={hint}
                onClick={() => head(key)}
              >
                {label}
                <Show when={column() === key}>
                  <span class='arrow'>{descending() ? ' ↓' : ' ↑'}</span>
                </Show>
              </button>
            )}
          </For>
        </div>

        <div class='preset-rows'>
          <For
            each={rows()}
            fallback={
              <p class='muted setup-empty'>
                Nothing saved yet. Join a room and press <b>Save this room</b>,
                or import what Chobby has.
              </p>
            }
          >
            {(preset) => (
              <div
                class='preset-row'
                classList={{ on: chosen() === preset.name }}
                onClick={() => setChosen(preset.name)}
                onDblClick={() => inRoom() && void apply(preset)}
              >
                <span class='preset-name' title={preset.name}>
                  {preset.name}
                  <Show when={tweakCount(preset) > 0}>
                    <span class='chip'>{tweakCount(preset)} tweaks</span>
                  </Show>
                </span>
                <span class='muted' title={preset.map ?? ''}>
                  {preset.map ?? '—'}
                </span>
                <span class='tabular'>
                  {Object.keys(preset.modoptions).length}
                </span>
                <span class='tabular muted'>
                  {when(preset.lastUsed, now())}
                </span>
                <span class='tabular muted'>{when(preset.updated, now())}</span>
                <span class='tabular muted'>{when(preset.created, now())}</span>
              </div>
            )}
          </For>
        </div>
      </div>

      <Show when={selected()}>
        {(preset) => (
          <footer class='preset-bar'>
            <span class='preset-name'>{preset().name}</span>
            <For
              each={
                [
                  ['map', 'Map'],
                  ['modoptions', 'Settings'],
                  ['battle', 'Room'],
                  ['startBoxes', 'Start boxes'],
                ] as const
              }
            >
              {([key, label]) => (
                <button
                  class='chip-choice'
                  classList={{ on: sections()[key] }}
                  onClick={() =>
                    setSections({ ...sections(), [key]: !sections()[key] })
                  }
                >
                  {label}
                </button>
              )}
            </For>

            {/* The one that decides whether two presets stack. */}
            <button
              class='chip-choice'
              classList={{ on: !sections().reset }}
              title={
                sections().reset
                  ? 'Resets the room first, so this preset is all that is left'
                  : 'Leaves what is already set, so presets combine'
              }
              onClick={() =>
                setSections({ ...sections(), reset: !sections().reset })
              }
            >
              Combine
            </button>

            <span class='spacer' />
            <button
              disabled={busy()}
              onClick={() => setAsking({ kind: 'rename', from: preset().name })}
            >
              Rename
            </button>
            <button
              disabled={busy()}
              onClick={() =>
                void act('delete', async () => {
                  mutate(await api.deletePreset(preset().name))
                  setChosen(null)
                })
              }
            >
              Delete
            </button>
            <button
              class='primary'
              disabled={!inRoom() || busy()}
              title={inRoom() ? '' : 'join a room to apply a preset'}
              onClick={() => void apply(preset())}
            >
              Apply
            </button>
          </footer>
        )}
      </Show>

      <Show when={asking()}>
        {(what) => (
          <Ask
            title={
              what().kind === 'save' ? 'Save this room as' : 'Rename preset'
            }
            hint={
              what().kind === 'save'
                ? 'Saving over a name keeps the day it was first made.'
                : undefined
            }
            initial={what().kind === 'rename' ? (chosen() ?? '') : ''}
            confirm={what().kind === 'save' ? 'Save' : 'Rename'}
            onCancel={() => setAsking(null)}
            onAnswer={(answer) => {
              const asked = what()
              setAsking(null)
              if (asked.kind === 'save') return void save(answer)
              if (answer === asked.from) return
              void act('rename', async () => {
                mutate(await api.renamePreset(asked.from, answer))
                setChosen(answer)
              })
            }}
          />
        )}
      </Show>
    </section>
  )
}
