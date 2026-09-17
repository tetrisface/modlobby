import {
  For,
  Show,
  createMemo,
  createResource,
  createSignal,
  onCleanup,
  onMount,
} from 'solid-js'
import { api } from '../ipc/client'
import type { WidgetUsage } from '../ipc/bindings/WidgetUsage'
import type { WindowStats } from '../ipc/bindings/WindowStats'
import { devicePixels, thumbSrc } from '../lib/thumb'
import {
  type Action,
  type Audience,
  AUDIENCE_ORDER,
  DEFAULT_AUDIENCE,
  DEFAULT_WINDOW,
  SORT_STARTS_DESCENDING,
  type SortKey,
  WINDOW_ORDER,
  actionsFor,
  audiences,
  configuredState,
  disabledOnly,
  isEnabled,
  isInstalled,
  isRepresentative,
  loadWidgetUsage,
  localFor,
  locationOf,
  matches,
  ranked,
  refreshInstalled,
  sortWidgets,
  statsFor,
  status,
  unavailableBecause,
  usage,
  windows,
} from '../store/widgets'

/**
 * What BAR players actually run, and what is on this machine — in one list.
 *
 * BAR's own Widget Hub says what is offered; this says what is used. The
 * numbers are pve.bar's weekly projection over public replays, counted in
 * distinct players rather than sightings, so one enthusiast playing all
 * evening does not read as a crowd. Nobody is named, here or upstream.
 *
 * **A row, not a card.** Every field the card carried is still here; a table
 * fits four times as many widgets on a screen, and every column sorts, because
 * "which of these do people keep switched on" is a question about a column.
 *
 * **Your files, not just BAR's config.** BAR knows a widget by its name alone,
 * so a homebrewed copy and the published one look identical to it. Each row
 * says which file on disk answers to its name and whether that file *is* the
 * published revision, byte for byte, or only shares the name.
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

/** The tile in a row, in CSS pixels. 16:10, the hub's own cover shape. */
const TILE = { width: 64, height: 40 }
/**
 * The enlarged picture: about four rows tall. Cut from the same download as
 * the tile, so opening it costs a local resize and no request.
 */
const PREVIEW = { width: 480, height: 300 }

/** Headers in column order. Every one sorts. */
const COLUMNS: ReadonlyArray<{
  key: SortKey
  label: string
  numeric?: boolean
}> = [
  { key: 'rank', label: '#', numeric: true },
  { key: 'name', label: 'Widget' },
  { key: 'players', label: 'Players', numeric: true },
  { key: 'retention', label: 'Kept on', numeric: true },
  { key: 'off', label: 'Off', numeric: true },
  { key: 'replays', label: 'Replays', numeric: true },
  { key: 'sightings', label: 'Sightings', numeric: true },
  { key: 'window', label: 'Window' },
  { key: 'source', label: 'Source' },
  { key: 'status', label: 'Actions' },
]

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
   * PvE and PvP are offered only when the document carries the split. The
   * pipeline withholds it while it is still re-reading history under new
   * rules, and a button that changes nothing is worse than no button.
   */
  const splits = createMemo<Audience[]>(() =>
    AUDIENCE_ORDER.filter((name) => audiences().includes(name)),
  )
  const [audience, setAudience] = createSignal<Audience>(DEFAULT_AUDIENCE)
  const shownAudience = createMemo<Audience>(() =>
    splits().includes(audience()) ? audience() : DEFAULT_AUDIENCE,
  )

  const [query, setQuery] = createSignal('')
  const [installedOnly, setInstalledOnly] = createSignal(false)
  const [sort, setSort] = createSignal<SortKey>('rank')
  const [descending, setDescending] = createSignal(false)

  /** Clicking the header already sorted by flips it, as any table does. */
  const sortBy = (key: SortKey) => {
    if (sort() === key) {
      setDescending(!descending())
      return
    }
    setSort(key)
    setDescending(SORT_STARTS_DESCENDING[key])
  }

  const inWindow = createMemo(() => ranked(shownAudience(), shown()))
  const installedCount = createMemo(
    () => inWindow().filter((widget) => isInstalled(widget)).length,
  )
  const listed = createMemo(() =>
    sortWidgets(
      inWindow().filter(
        (widget) =>
          matches(widget, query()) && (!installedOnly() || isInstalled(widget)),
      ),
      sort(),
      descending(),
      shownAudience(),
      shown(),
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
  const [enlarged, setEnlarged] = createSignal<string | null>(null)

  onMount(() => {
    const close = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setEnlarged(null)
    }
    window.addEventListener('keydown', close)
    onCleanup(() => window.removeEventListener('keydown', close))
  })

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
        setNote(
          `${widget.name} installed to modlobby's widget folder. BAR starts a new widget switched off — use Enable to turn it on.`,
        )
      } else if (action === 'disable') {
        await api.widgetDisable(widget.name)
      } else if (action === 'enable') {
        await api.widgetEnable(widget.name)
      } else {
        const deleted = await api.widgetDelete(widget.key)
        setNote(
          deleted.residue.length > 0
            ? `${widget.name} removed. Left alone, because nothing records which widget wrote them: ${deleted.residue.join('; ')}.`
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

      <header class='toolbar widget-toolbar'>
        <input
          type='search'
          class='search'
          placeholder='Search name, author or description'
          value={query()}
          onInput={(event) => setQuery(event.currentTarget.value)}
        />

        <div class='filter-group' role='group' aria-label='Show'>
          <Choice
            label='All'
            on={!installedOnly()}
            onClick={() => setInstalledOnly(false)}
          />
          <Choice
            label='Installed'
            on={installedOnly()}
            onClick={() => setInstalledOnly(true)}
          />
        </div>

        <Show when={splits().length > 1}>
          <div class='filter-group' role='group' aria-label='Mode'>
            <For each={splits()}>
              {(name) => (
                <Choice
                  label={AUDIENCE_LABEL[name] ?? name}
                  on={shownAudience() === name}
                  onClick={() => setAudience(name)}
                />
              )}
            </For>
          </div>
        </Show>

        <Show when={offered().length > 0}>
          <div class='filter-group' role='group' aria-label='Window'>
            <For each={offered()}>
              {(name) => (
                <Choice
                  label={label(name)}
                  on={shown() === name}
                  onClick={() => setPicked(name)}
                />
              )}
            </For>
          </div>
        </Show>

        <span class='spacer' />
        <Show when={usage()}>
          <span class='muted count'>
            {listed().length} widgets · {installedCount()} installed
          </span>
        </Show>
      </header>

      <Show when={note()}>{(said) => <p class='widget-note'>{said()}</p>}</Show>

      <Show
        when={listed().length > 0}
        fallback={
          <Empty
            loading={fetched.loading && !usage()}
            searching={!!query()}
            installedOnly={installedOnly()}
          />
        }
      >
        <p class='widgets-about'>
          What players actually run, from pve.bar's weekly read of public
          replays. Counted in distinct players
          <Show when={built()}>{(day) => <>, built {day()}</>}</Show>. A widget
          too few people run to be counted anonymously is left out. Installs go
          to modlobby's own widget folder, so they apply to games launched from
          here and not to Chobby's.
        </p>
        <table class='widget-table'>
          <thead>
            <tr>
              <For each={COLUMNS}>
                {(column) => (
                  <th
                    classList={{ num: !!column.numeric }}
                    aria-sort={
                      sort() === column.key
                        ? descending()
                          ? 'descending'
                          : 'ascending'
                        : 'none'
                    }
                  >
                    <button
                      type='button'
                      class='sort-head'
                      onClick={() => sortBy(column.key)}
                    >
                      {column.label}
                      <span class='sort-mark' aria-hidden='true'>
                        {sort() === column.key
                          ? descending()
                            ? '↓'
                            : '↑'
                          : ''}
                      </span>
                    </button>
                  </th>
                )}
              </For>
            </tr>
          </thead>
          <tbody>
            <For each={listed()}>
              {(widget) => (
                <Row
                  widget={widget}
                  audience={shownAudience()}
                  window={shown()}
                  busy={busy()}
                  enlarged={enlarged() === widget.key}
                  onEnlarge={() =>
                    setEnlarged(enlarged() === widget.key ? null : widget.key)
                  }
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

function Choice(props: { label: string; on: boolean; onClick: () => void }) {
  return (
    <button
      type='button'
      class='chip-choice'
      classList={{ on: props.on }}
      aria-pressed={props.on}
      onClick={props.onClick}
    >
      {props.label}
    </button>
  )
}

function Empty(props: {
  loading: boolean
  searching: boolean
  installedOnly: boolean
}) {
  return (
    <p class='muted'>
      <Show when={!props.loading} fallback='Fetching what people run…'>
        <Show
          when={props.searching || props.installedOnly}
          fallback={
            <>
              Widget usage could not be fetched. It comes from pve.bar, so this
              is what an offline launch looks like too.
            </>
          }
        >
          <Show
            when={props.installedOnly}
            fallback='No widget matches that search in this window.'
          >
            None of the widgets in this window are installed here.
          </Show>
        </Show>
      </Show>
    </p>
  )
}

/**
 * One widget: its picture, who made it, how it did, what is on this machine,
 * and what can be done about it.
 */
function Row(props: {
  widget: WidgetUsage
  audience: string
  window: string
  busy: string | null
  enlarged: boolean
  onEnlarge: () => void
  onAct: (widget: WidgetUsage, action: Action) => void
}) {
  const found = () => statsFor(props.widget, props.audience, props.window)
  const stats = () => found()?.stats
  const shownWindow = () => found()?.window ?? props.window
  const configured = () => configuredState(props.widget)

  return (
    <tr classList={{ off: !!configured() && !isEnabled(configured()!) }}>
      <td class='num'>{stats()?.rank ?? '—'}</td>
      <td class='widget-name'>
        <div class='widget-identity'>
          <Thumb
            widget={props.widget}
            enlarged={props.enlarged}
            onEnlarge={props.onEnlarge}
          />
          <div class='widget-text'>
            <strong>{props.widget.name}</strong>
            <Show when={props.widget.author}>
              <span class='widget-by'> by {props.widget.author}</span>
            </Show>
            <Show when={props.widget.description}>
              <p class='widget-about'>{props.widget.description}</p>
            </Show>
            <Marks widget={props.widget} stats={stats()} />
            <OnThisMachine widget={props.widget} />
          </div>
        </div>
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
 * A widget's picture, which opens to about four rows tall.
 *
 * Both sizes are cut from one download — the picture is fetched once and kept
 * — so opening it is a local resize. A widget with no picture, or one whose
 * picture will not load, gets its initials instead of a broken image.
 */
function Thumb(props: {
  widget: WidgetUsage
  enlarged: boolean
  onEnlarge: () => void
}) {
  const [failed, setFailed] = createSignal(false)
  const src = (box: { width: number; height: number }) => {
    const tile = devicePixels(box)
    return thumbSrc(`widget/${tile.width}x${tile.height}/${props.widget.key}`)
  }
  const pictured = () => !!props.widget.image && !failed()

  return (
    <Show
      when={pictured()}
      fallback={
        <span
          class='widget-thumb placeholder'
          style={{ '--hue': String(hue(props.widget.name)) }}
          aria-hidden='true'
        >
          {initials(props.widget.name)}
        </span>
      }
    >
      <span class='widget-thumb-anchor'>
        <button
          type='button'
          class='widget-thumb'
          aria-label={`Show a larger picture of ${props.widget.name}`}
          aria-expanded={props.enlarged}
          onClick={props.onEnlarge}
        >
          <img
            src={src(TILE)}
            width={TILE.width}
            height={TILE.height}
            loading='lazy'
            alt=''
            onError={() => setFailed(true)}
          />
        </button>
        <Show when={props.enlarged}>
          <button
            type='button'
            class='widget-preview'
            aria-label='Close the larger picture'
            onClick={props.onEnlarge}
          >
            <img
              src={src(PREVIEW)}
              width={PREVIEW.width}
              height={PREVIEW.height}
              alt={props.widget.name}
            />
          </button>
        </Show>
      </span>
    </Show>
  )
}

/**
 * Which file on disk answers to this widget's name, and whether it *is* it.
 *
 * The distinction BAR's config cannot make: a name there covers every file
 * that declares it. Two such files means BAR loads one and rejects the other
 * as a duplicate, which is worth saying before somebody wonders which one is
 * running.
 */
function OnThisMachine(props: { widget: WidgetUsage }) {
  const found = createMemo(() => localFor(props.widget))
  return (
    <Show when={found().length > 0}>
      <ul class='widget-local'>
        <For each={found()}>
          {(match) => (
            <li
              classList={{ exact: match.exact }}
              title={`${match.file.dir}/${match.file.file}`}
            >
              <span class='widget-local-kind'>
                {match.exact ? 'This version' : 'Your own version'}
              </span>{' '}
              <code>{match.file.file.replace(/^LuaUI\/Widgets\//, '')}</code>{' '}
              <span class='muted'>in {locationOf(match.file)}</span>
            </li>
          )}
        </For>
        <Show when={found().length > 1}>
          <li class='muted'>
            {found().length} files declare this name. BAR loads the first and
            skips the rest as duplicates.
          </li>
        </Show>
      </ul>
    </Show>
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

/** Up to two letters, from the words of a name. */
export function initials(name: string): string {
  const words = name.match(/[A-Za-z0-9]+/g) ?? []
  const [first = '?', second] = words
  const letters = second
    ? `${first.charAt(0)}${second.charAt(0)}`
    : first.slice(0, 2)
  return letters.toUpperCase()
}

/** A stable hue per name, so a placeholder tile is recognisable next time. */
export function hue(name: string): number {
  let hash = 0
  for (const char of name) hash = (hash * 31 + char.charCodeAt(0)) % 360
  return hash
}

function message(err: unknown): string {
  if (err && typeof err === 'object' && 'message' in err) {
    return String((err as { message: unknown }).message)
  }
  return String(err)
}
