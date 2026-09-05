import {
  For,
  Show,
  createMemo,
  createResource,
  createSignal,
  onCleanup,
} from 'solid-js'
import { ActionCell, CellButton } from '../components/ActionCell'
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
  when,
} from '../lib/presets'
import { pushNotice } from '../store/chat'

/**
 * The columns the pane has room for. Settings and Created are still sortable
 * in `lib/presets`, but at pane width they would cost the name its space;
 * the creation date rides in the row's tooltip instead.
 */
const COLUMNS: Array<[Column, string, string]> = [
  ['name', 'Name', 'What you called it'],
  ['map', 'Map', 'The map it was saved on'],
  ['used', 'Used', 'When it was last applied to a room'],
  ['updated', 'Updated', 'When it last changed'],
]

const SECTIONS = [
  ['map', 'Map'],
  ['modoptions', 'Settings'],
  ['battle', 'Room'],
  ['startBoxes', 'Start boxes'],
] as const

/**
 * Everything but the bots, layered over what the room already has. Resetting
 * first is the exception: it wipes the room down to the SPADS preset, which
 * is rarely what somebody stacking a tweak on top of a room wants.
 */
const DEFAULT_SECTIONS: Sections = {
  map: true,
  modoptions: true,
  battle: true,
  startBoxes: true,
  bots: false,
  reset: false,
}

/** What the sheet is for: a fresh name, or a new name for `from`. */
type Asking = { kind: 'save' } | { kind: 'rename'; from: string }

/** A rename starts from the old name; a save starts blank. */
function initialText(asked: Asking): string {
  return asked.kind === 'rename' ? asked.from : ''
}

/**
 * Saved room setups, in the room's pane beside Setup.
 *
 * A table rather than a dropdown because these accumulate — sixty-five of them
 * in the file this was built against — and the questions people ask of them
 * are "which did I use last", "which was for this map" and "which is the big
 * one", none of which a list of names answers.
 *
 * Selecting is a click anywhere on the row; renaming and deleting are the two
 * icons on it. Keeping the name itself inert is what makes a row selectable
 * on a small display without opening something by accident.
 */
export function Presets() {
  const [column, setColumn] = createSignal<Column>(DEFAULT_SORT)
  const [descending, setDescending] = createSignal(true)
  const [needle, setNeedle] = createSignal('')
  const [chosen, setChosen] = createSignal<string | null>(null)
  const [sections, setSections] = createSignal<Sections>(DEFAULT_SECTIONS)
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
  const [asking, setAsking] = createSignal<Asking | null>(null)

  const save = (name: string) =>
    act('save', async () => {
      mutate(await api.savePreset(name))
      setChosen(name)
    })

  const load = (preset: Preset) =>
    act('load', async () => {
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

  const remove = (preset: Preset) =>
    act('delete', async () => {
      mutate(await api.deletePreset(preset.name))
      if (chosen() === preset.name) setChosen(null)
    })

  const importFromChobby = () =>
    act('import', async () => {
      const { book: next, skipped } = await api.importPresets(null)
      mutate(next)
      pushNotice(
        'info',
        skipped
          ? `imported; ${skipped} left alone because you already had those names`
          : 'imported from Chobby',
      )
    })

  const exportToChobby = (preset: Preset) =>
    act('export', async () => {
      await api.exportPresets(null, [preset.name])
      pushNotice('info', `${preset.name} written to Chobby's presets`)
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
          disabled={busy()}
          title='Save this room as a preset'
          onClick={() => setAsking({ kind: 'save' })}
        >
          Save
        </button>
        <button
          class='primary'
          disabled={busy() || !selected()}
          title='Apply the selected preset to this room'
          onClick={() => {
            const preset = selected()
            if (preset) void load(preset)
          }}
        >
          Load
        </button>
        <button
          disabled={busy()}
          title={chobbyPath() ?? 'no BAR data directory found'}
          onClick={() => void importFromChobby()}
        >
          Import from Chobby
        </button>
        <button
          disabled={busy() || !selected()}
          title={chobbyPath() ?? ''}
          onClick={() => {
            const preset = selected()
            if (preset) void exportToChobby(preset)
          }}
        >
          Export to Chobby
        </button>
        <span class='spacer' />
        <span class='muted'>{rows().length} presets</span>
      </header>

      {/* Which parts a Load sends. */}
      <div class='preset-sections'>
        <For each={SECTIONS}>
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
          classList={{ on: sections().reset }}
          title={
            sections().reset
              ? 'Sends !preset first, so the room is reset to this preset alone'
              : 'Leaves what the room already has, so presets stack'
          }
          onClick={() =>
            setSections({ ...sections(), reset: !sections().reset })
          }
        >
          Reset lobby
        </button>
      </div>

      {/* The header scrolls with the rows, stuck to the top: inside the same
          box as the rows, its columns stay over theirs whether or not a
          scrollbar is taking width from that box. */}
      <div class='preset-table'>
        <div class='preset-rows'>
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
            <span />
          </div>
          <For
            each={rows()}
            fallback={
              <p class='muted setup-empty'>
                Nothing saved yet. Press <b>Save</b>, or import what Chobby has.
              </p>
            }
          >
            {(preset) => (
              <div
                class='preset-row'
                classList={{ on: chosen() === preset.name }}
                title={`${preset.name}\ncreated ${when(preset.created, now())}`}
                onClick={() => setChosen(preset.name)}
                onDblClick={() => void load(preset)}
              >
                <span class='preset-name'>{preset.name}</span>
                <span class='preset-map muted' title={preset.map ?? ''}>
                  {preset.map ?? '—'}
                </span>
                <span class='tabular muted'>
                  {when(preset.lastUsed, now())}
                </span>
                <span class='tabular muted'>{when(preset.updated, now())}</span>
                <ActionCell>
                  <CellButton
                    icon='act-pen'
                    title='Rename'
                    label={`Rename ${preset.name}`}
                    disabled={busy()}
                    onClick={(event) => {
                      event.stopPropagation()
                      setAsking({ kind: 'rename', from: preset.name })
                    }}
                  />
                  <CellButton
                    class='danger'
                    icon='act-trash'
                    title='Delete'
                    label={`Delete ${preset.name}`}
                    disabled={busy()}
                    onClick={(event) => {
                      event.stopPropagation()
                      void remove(preset)
                    }}
                  />
                </ActionCell>
              </div>
            )}
          </For>
        </div>
      </div>

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
            initial={initialText(what())}
            confirm={what().kind === 'save' ? 'Save' : 'Rename'}
            onCancel={() => setAsking(null)}
            onAnswer={(answer) => {
              const asked = what()
              setAsking(null)
              if (asked.kind === 'save') return void save(answer)
              if (answer === asked.from) return
              void act('rename', async () => {
                mutate(await api.renamePreset(asked.from, answer))
                if (chosen() === asked.from) setChosen(answer)
              })
            }}
          />
        )}
      </Show>
    </section>
  )
}
