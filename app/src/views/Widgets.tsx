import { For, Show, createMemo, createResource, createSignal } from 'solid-js'
import { api } from '../ipc/client'
import type { WidgetUsage } from '../ipc/bindings/WidgetUsage'
import type { WindowStats } from '../ipc/bindings/WindowStats'
import {
  type Action,
  type Audience,
  AUDIENCE_ORDER,
  DEFAULT_AUDIENCE,
  DEFAULT_WINDOW,
  WINDOW_ORDER,
  actionsFor,
  audiences,
  configuredState,
  disabledOnly,
  isEnabled,
  isRepresentative,
  loadWidgetUsage,
  matches,
  ranked,
  refreshInstalled,
  statsFor,
  status,
  unavailableBecause,
  usage,
  windows,
} from '../store/widgets'

/**
 * What BAR players actually run, ranked — and manageable.
 *
 * BAR's own Widget Hub says what is offered; this says what is used. The
 * numbers are pve.bar's weekly projection over public replays, counted in
 * distinct players rather than sightings, so one enthusiast playing all
 * evening does not read as a crowd. Nobody is named, here or upstream.
 *
 * **A row, not a card.** Every field the card carried is still here; a table
 * simply fits four times as many widgets on a screen, and comparing two
 * widgets' retention is what the list is for.
 *
 * **Half of these cannot be installed, and the row says so.** Roughly half the
 * published widgets have no traceable source, and of the rest not all may be
 * redistributed. Those keep their numbers and, where there is one, a link. A
 * button that does nothing would be worse than no button.
 */

/** What each window is called, rather than what it is keyed by. */
const WINDOW_LABEL: Record<string, string> = {
  '7d': '7 days',
  '30d': '30 days',
  '90d': '90 days',
  '365d': '1 year',
  all: 'All time',
}

/** Mirrors the battles list, which readers already know. */
const AUDIENCE_LABEL: Record<string, string> = {
  all: 'All',
  pve: 'PvE',
  pvp: 'PvP',
}

const label = (window: string) => WINDOW_LABEL[window] ?? window

const ACTION_LABEL: Record<Action, string> = {
  install: 'Install',
  update: 'Update',
  disable: 'Disable',
  enable: 'Enable',
  delete: 'Delete',
}

export function Widgets() {
  const [fetched] = createResource(async () => {
    await loadWidgetUsage()
    await refreshInstalled()
    return usage()
  })

  /**
   * The windows this document carries, narrowest first.
   *
   * Taken from the declared order rather than the document's own, so the row
   * of buttons reads left to right however the pipeline happened to serialise.
   */
  const offered = createMemo<string[]>(() =>
    WINDOW_ORDER.filter((name) => windows().includes(name)),
  )
  const [picked, setPicked] = createSignal<string | null>(null)
  /**
   * The window being shown. A pick that the document does not carry is not
   * honoured — the default first, and the widest published as a last resort,
   * so there is always something to draw.
   */
  const shown = createMemo(() => {
    const there = offered()
    const chosen = picked()
    if (chosen && there.includes(chosen)) return chosen
    if (there.includes(DEFAULT_WINDOW)) return DEFAULT_WINDOW
    return there[there.length - 1] ?? DEFAULT_WINDOW
  })

  /**
   * The audiences this document carries.
   *
   * Only shown when there is a choice: while the pipeline is re-reading history
   * under new decoding rules it publishes the combined view alone, and a lone
   * "All" button is a control that does nothing.
   */
  const splits = createMemo<Audience[]>(() =>
    AUDIENCE_ORDER.filter((name) => audiences().includes(name)),
  )
  const [audience, setAudience] = createSignal<Audience>(DEFAULT_AUDIENCE)
  const shownAudience = createMemo<Audience>(() => {
    const there = splits()
    const chosen = audience()
    return there.includes(chosen) ? chosen : DEFAULT_AUDIENCE
  })

  const [query, setQuery] = createSignal('')
  const listed = createMemo(() =>
    ranked(shownAudience(), shown()).filter((widget) =>
      matches(widget, query()),
    ),
  )

  /** The day the pipeline built this, in the reader's own format. */
  const built = () => {
    const at = usage()?.generated_at
    if (!at) return ''
    const when = new Date(at)
    return Number.isNaN(when.getTime()) ? '' : when.toLocaleDateString()
  }

  const [busy, setBusy] = createSignal<string | null>(null)
  const [note, setNote] = createSignal<string | null>(null)

  /**
   * Run one action, then ask the disk what actually happened.
   *
   * The state is refetched rather than patched: `BYAR.lua` is a file the game
   * also writes, so what this thinks it did and what is on disk can disagree.
   */
  const act = async (widget: WidgetUsage, action: Action) => {
    setBusy(`${widget.key}:${action}`)
    setNote(null)
    try {
      if (action === 'install' || action === 'update') {
        await api.widgetInstall(widget.key, widget.name, widget.install)
        setNote(`${widget.name} installed to modlobby's own widget folder.`)
      } else if (action === 'disable') {
        await api.widgetDisable(widget.name)
      } else if (action === 'enable') {
        await api.widgetEnable(widget.name)
      } else {
        const deleted = await api.widgetDelete(widget.key)
        setNote(
          deleted.residue.length > 0
            ? `${widget.name} removed. Left alone, because nothing records which widget wrote them: ${deleted.residue.join(', ')}.`
            : `${widget.name} removed, settings included.`,
        )
      }
      await refreshInstalled()
    } catch (err) {
      setNote(message(err))
    } finally {
      setBusy(null)
    }
  }

  return (
    <section class='widgets'>
      <h1>Widgets</h1>

      <div class='widget-controls'>
        <input
          type='search'
          class='widget-search'
          placeholder='Search name, author or description'
          value={query()}
          onInput={(event) => setQuery(event.currentTarget.value)}
        />
        <Show when={splits().length > 1}>
          <div class='tabs'>
            <For each={splits()}>
              {(name) => (
                <button
                  type='button'
                  class='tab'
                  classList={{ on: shownAudience() === name }}
                  onClick={() => setAudience(name)}
                >
                  {AUDIENCE_LABEL[name] ?? name}
                </button>
              )}
            </For>
          </div>
        </Show>
        <Show when={offered().length > 0}>
          <div class='tabs'>
            <For each={offered()}>
              {(name) => (
                <button
                  type='button'
                  class='tab'
                  classList={{ on: shown() === name }}
                  onClick={() => setPicked(name)}
                >
                  {label(name)}
                </button>
              )}
            </For>
          </div>
        </Show>
      </div>

      <Show when={note()}>{(said) => <p class='widget-note'>{said()}</p>}</Show>

      <Show
        when={listed().length > 0}
        fallback={<Empty loading={fetched.loading} searching={!!query()} />}
      >
        <p class='widgets-about'>
          What players actually run, from pve.bar's weekly read of public
          replays. Counted in distinct players
          <Show when={built()}>{(day) => <>, built {day()}</>}</Show>. A widget
          too few people run to be counted anonymously is left out.
          <Show when={status()?.writeDir}>
            {' '}
            Installs go to modlobby's own widget folder, so they apply to games
            launched from here and not to Chobby's.
          </Show>
        </p>
        <table class='widget-table'>
          <thead>
            <tr>
              <th class='num'>#</th>
              <th>Widget</th>
              <th class='num'>Players</th>
              <th class='num'>Kept on</th>
              <th class='num'>Off</th>
              <th class='num'>Replays</th>
              <th class='num'>Sightings</th>
              <th>Window</th>
              <th>Source</th>
              <th class='widget-actions-head'>Actions</th>
            </tr>
          </thead>
          <tbody>
            <For each={listed()}>
              {(widget, at) => (
                <Row
                  widget={widget}
                  audience={shownAudience()}
                  window={shown()}
                  place={at() + 1}
                  busy={busy()}
                  onAct={act}
                />
              )}
            </For>
          </tbody>
        </table>
      </Show>
    </section>
  )
}

function Empty(props: { loading: boolean; searching: boolean }) {
  return (
    <p class='muted'>
      <Show when={!props.loading} fallback='Fetching what people run…'>
        <Show
          when={!props.searching}
          fallback='No widget matches that search in this window.'
        >
          Widget usage could not be fetched. It comes from pve.bar, so this is
          what an offline launch looks like too.
        </Show>
      </Show>
    </p>
  )
}

/**
 * One widget: who made it, what it is, how it did, and what can be done to it.
 *
 * The stats are read through the store's accessor rather than off the widget,
 * but its fallback never fires here: a widget is on this list because the
 * window carries a row for it. One that the window withheld is simply not
 * listed — the anonymity floor is applied inside each window, and a widget
 * with six players this year and two this week is absent from the week rather
 * than shown with a two.
 */
function Row(props: {
  widget: WidgetUsage
  audience: string
  window: string
  place: number
  busy: string | null
  onAct: (widget: WidgetUsage, action: Action) => void
}) {
  const found = () => statsFor(props.widget, props.audience, props.window)
  const stats = () => found()?.stats
  const shownWindow = () => found()?.window ?? props.window
  const configured = () => configuredState(props.widget)

  return (
    <tr classList={{ off: !!configured() && !isEnabled(configured()!) }}>
      <td class='num'>{props.place}</td>
      <td class='widget-name'>
        <strong>{props.widget.name}</strong>
        <Show when={props.widget.author}>
          <span class='widget-by'> by {props.widget.author}</span>
        </Show>
        <Show when={props.widget.description}>
          <p class='widget-about'>{props.widget.description}</p>
        </Show>
        <Marks widget={props.widget} stats={stats()} />
      </td>
      <td class='num'>{stats()?.players.toLocaleString() ?? '—'}</td>
      {/* Install-and-keep. A widget people install and then switch off scores
          low here and nowhere else, which is the one thing a download count
          cannot tell you. */}
      <td class='num'>
        <Show when={stats()} fallback='—'>
          {(found) => <>{Math.round(found().retention * 100)}%</>}
        </Show>
      </td>
      <td class='num'>
        <Show when={stats()} fallback='—'>
          {(found) => <>{disabledOnly(found()).toLocaleString()}</>}
        </Show>
      </td>
      <td class='num'>{stats()?.replays.toLocaleString() ?? '—'}</td>
      <td class='num'>{stats()?.sightings.toLocaleString() ?? '—'}</td>
      <td>
        {label(shownWindow())}
        <Show when={stats() && !isRepresentative(stats()!)}>
          <span class='muted'> ({stats()!.days_covered}d)</span>
        </Show>
      </td>
      <td class='widget-source'>
        <Source widget={props.widget} />
      </td>
      <td class='widget-actions'>
        <Actions widget={props.widget} busy={props.busy} onAct={props.onAct} />
      </td>
    </tr>
  )
}

/**
 * Where the widget comes from — a link whenever there is one to give.
 *
 * "A link for all of them" is the rule: a widget we may not redistribute, and
 * even one we cannot install at all, still points at wherever it lives.
 */
function Source(props: { widget: WidgetUsage }) {
  const install = () => props.widget.install
  const why = () => unavailableBecause(install())
  return (
    <>
      <Show when={install().page} fallback={<span class='muted'>unknown</span>}>
        {(page) => (
          <a href={page()} target='_blank' rel='noreferrer'>
            {install().kind}
          </a>
        )}
      </Show>
      <Show when={why()}>
        {(reason) => <p class='muted widget-why'>{reason()}</p>}
      </Show>
    </>
  )
}

function Actions(props: {
  widget: WidgetUsage
  busy: string | null
  onAct: (widget: WidgetUsage, action: Action) => void
}) {
  const offered = () => actionsFor(props.widget)
  const locked = () => status()?.locked ?? false
  /** Disable, enable and delete all write the config, which BAR would clobber. */
  const writesConfig = (action: Action) => action !== 'install'

  return (
    <Show when={offered().length > 0} fallback={<span class='muted'>—</span>}>
      <div class='widget-buttons'>
        <For each={offered()}>
          {(action) => (
            <button
              type='button'
              class='small'
              classList={{ danger: action === 'delete' }}
              disabled={
                props.busy === `${props.widget.key}:${action}` ||
                (locked() && writesConfig(action))
              }
              title={
                locked() && writesConfig(action)
                  ? 'A game is running. BAR rewrites its widget config on exit and would discard this.'
                  : undefined
              }
              onClick={() => props.onAct(props.widget, action)}
            >
              {ACTION_LABEL[action]}
            </button>
          )}
        </For>
      </div>
    </Show>
  )
}

/** What is worth saying about a widget besides its numbers. */
function Marks(props: { widget: WidgetUsage; stats: WindowStats | undefined }) {
  const configured = () => configuredState(props.widget)
  return (
    <div class='chips'>
      <Show when={props.widget.install.kind === 'hub'}>
        <span class='chip ok'>On the Widget Hub</span>
      </Show>
      <Show when={configured() && !isEnabled(configured()!)}>
        <span class='chip'>Switched off</span>
      </Show>
      <Show when={props.widget.install.license}>
        {(licence) => <span class='chip'>{licence()}</span>}
      </Show>
      {/* The pipeline backfills history a slice at a time, so a year window
          can hold a fortnight. Saying so beats implying a year of evidence. */}
      <Show when={props.stats && !isRepresentative(props.stats)}>
        <span class='chip warn'>
          {props.stats!.days_covered}{' '}
          {props.stats!.days_covered === 1 ? 'day' : 'days'} harvested
        </span>
      </Show>
    </div>
  )
}

function message(err: unknown): string {
  if (err && typeof err === 'object' && 'message' in err) {
    return String((err as { message: unknown }).message)
  }
  return String(err)
}
