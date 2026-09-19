import {
  For,
  Show,
  createMemo,
  createResource,
  createSignal,
  onCleanup,
  onMount,
} from 'solid-js'
import { Glyph } from '../components/icons'
import { openExternal } from '../components/Linkify'
import { api } from '../ipc/client'
import type { Fork } from '../ipc/bindings/Fork'
import type { LocalWidget } from '../ipc/bindings/LocalWidget'
import type { WidgetUsage } from '../ipc/bindings/WidgetUsage'
import type { WindowStats } from '../ipc/bindings/WindowStats'
import { age, exactly } from '../lib/age'
import { sticky } from '../lib/sticky'
import { devicePixels, thumbSrc } from '../lib/thumb'
import {
  type Action,
  type Audience,
  AUDIENCE_ORDER,
  DEFAULT_AUDIENCE,
  DEFAULT_USING_MODE,
  DEFAULT_WINDOW,
  SORT_STARTS_DESCENDING,
  type SortKey,
  USING_LABEL,
  type UsingMode,
  WINDOW_ORDER,
  actionsFor,
  audiences,
  configuredState,
  forkActions,
  forkStatsFor,
  forksOf,
  installedEntry,
  isEnabled,
  isInstalled,
  isRepresentative,
  loadWidgetUsage,
  localFor,
  localForFork,
  locationOf,
  mainFork,
  matches,
  notUsing,
  picturesOf,
  ranked,
  refreshInstalled,
  sortWidgets,
  statsFor,
  status,
  unavailableBecause,
  usage,
  usingShare,
  windows,
  yourVersions,
} from '../store/widgets'

/**
 * What BAR players actually run, and what is on this machine — in one list.
 *
 * BAR's own Widget Hub says what is offered; this says what is used. The
 * numbers are pve.bar's weekly projection over public replays, counted in
 * distinct players rather than sightings, so one enthusiast playing all
 * evening does not read as a crowd. Nobody is named, here or upstream.
 *
 * **A row is a widget name; a fork is a version of it.** BAR knows a widget by
 * its name alone — one config entry, one switch — so that is what a row is,
 * with the name's combined numbers. Opening it lists each publisher's version,
 * the players nobody could trace, and any file on this machine that matches
 * none of them. One level deep, never a tree.
 *
 * **Still using, unless asked otherwise.** A player counts as still using a
 * widget when their latest replay had it on; "Include used once" counts anyone
 * who had it on at all.
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

/** A source in words, for the button that opens it. */
const SOURCE_LABEL: Record<string, string> = {
  hub: 'Widget Hub',
  github: 'GitHub',
  gist: 'Gist',
  discord: 'Discord',
}

const label = (window: string) => WINDOW_LABEL[window] ?? window

const ACTION_LABEL: Record<Action, string> = {
  install: 'Install',
  update: 'Update',
  disable: 'Disable',
  enable: 'Enable',
  delete: 'Delete',
}

/**
 * The tile in a row, in CSS pixels.
 *
 * The width is fixed for every row -- a column of pictures that each started
 * at a different place would be noise -- and the height is the row's, so a
 * tile is as tall as what the row says. The size asked for here is what the
 * picture is cut to: a shape near the middle of the heights rows actually
 * take, so that neither a short row nor a tall one is showing an upscale.
 */
const TILE = { width: 96, height: 72 }
/** A fork's tile: narrower, so the version lines under a row read as lesser. */
const FORK_TILE = { width: 64, height: 48 }
/**
 * The enlarged picture: about four rows tall. Cut from the same download as
 * the tile, so opening it costs a local resize and no request.
 */
const PREVIEW = { width: 480, height: 300 }
/** A picture in the gallery's strip. */
const FILM = { width: 64, height: 40 }
/** Pictures shown beside a hovered tile; the rest are counted on the last. */
const COMPANIONS = 4
/** Which picture of which widget is open, and how many there are to step through. */
type Enlarged = { key: string; index: number; count: number }
const STEP: Record<string, number> = { ArrowRight: 1, ArrowLeft: -1 }

/** Headers in column order. Every one sorts. */
const COLUMNS: ReadonlyArray<{
  key: SortKey
  label: (mode: UsingMode) => string
  numeric?: boolean
}> = [
  { key: 'rank', label: () => '#', numeric: true },
  { key: 'name', label: () => 'Widget' },
  { key: 'players', label: () => 'Players', numeric: true },
  { key: 'using', label: (mode) => USING_LABEL[mode], numeric: true },
  { key: 'off', label: () => 'Off', numeric: true },
  { key: 'updated', label: () => 'Updated', numeric: true },
  { key: 'published', label: () => 'Published', numeric: true },
  { key: 'replays', label: () => 'Replays', numeric: true },
  { key: 'sightings', label: () => 'Sightings', numeric: true },
  { key: 'window', label: () => 'Window' },
  { key: 'source', label: () => 'Source' },
  { key: 'status', label: () => 'Actions' },
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
  const [picked, setPicked] = sticky<string | null>('widgets.window', null)
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
  const [audience, setAudience] = sticky<Audience>(
    'widgets.audience',
    DEFAULT_AUDIENCE,
  )
  const shownAudience = createMemo<Audience>(() =>
    splits().includes(audience()) ? audience() : DEFAULT_AUDIENCE,
  )

  // The search box is not remembered: a query kept from last time hides most
  // of the page, and unlike a filter button nothing on screen says why.
  const [query, setQuery] = createSignal('')
  const [installedOnly, setInstalledOnly] = sticky('widgets.installed', false)
  const [includeUsedOnce, setIncludeUsedOnce] = sticky('widgets.once', false)
  const mode = (): UsingMode =>
    includeUsedOnce() ? 'once' : DEFAULT_USING_MODE
  const [chosenSort, setSort] = sticky<SortKey>('widgets.sort', 'rank')
  /** A key a later build renamed is not honoured, the way a window is not. */
  const sort = (): SortKey =>
    chosenSort() in SORT_STARTS_DESCENDING ? chosenSort() : 'rank'
  const [descending, setDescending] = sticky('widgets.descending', false)

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
      mode(),
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
  const [enlarged, setEnlarged] = createSignal<Enlarged | null>(null)
  const [expanded, setExpanded] = createSignal<ReadonlySet<string>>(new Set())

  const toggleExpanded = (key: string) => {
    const next = new Set(expanded())
    if (next.has(key)) next.delete(key)
    else next.add(key)
    setExpanded(next)
  }
  const showPicture = (key: string, index: number | null, count: number) =>
    setEnlarged(index === null ? null : { key, index, count })

  onMount(() => {
    /** Escape closes the picture; the arrows step through its gallery. */
    const close = (event: KeyboardEvent) => {
      const open = enlarged()
      const step = STEP[event.key]
      if (event.key === 'Escape') setEnlarged(null)
      else if (open && open.count > 1 && step) {
        event.preventDefault()
        setEnlarged({
          ...open,
          index: (open.index + step + open.count) % open.count,
        })
      }
    }
    /**
     * An enlarged picture stays open until you look elsewhere, so a press
     * anywhere outside one closes it. On `pointerdown` rather than `click`:
     * the press that opens another row's picture then closes this one on its
     * way down, and the two never disagree about which is open.
     */
    const away = (event: PointerEvent) => {
      const on = event.target as HTMLElement | null
      if (on?.closest('.widget-thumb-anchor')) return
      setEnlarged(null)
    }
    window.addEventListener('keydown', close)
    window.addEventListener('pointerdown', away)
    onCleanup(() => {
      window.removeEventListener('keydown', close)
      window.removeEventListener('pointerdown', away)
    })
  })

  /**
   * Run one action, then ask the disk what actually happened.
   *
   * Install, update and delete act on one version; enable and disable act on
   * the name, because that is all BAR's config knows. The state is refetched
   * rather than patched: `BYAR.lua` is a file the game also writes, so what
   * this thinks it did and what is on disk can disagree.
   */
  const act = async (widget: WidgetUsage, fork: Fork, action: Action) => {
    setBusy(`${fork.key}:${action}`)
    setNote(null)
    try {
      if (action === 'install' || action === 'update') {
        await api.widgetInstall(fork.key, widget.name, fork.install)
        const others = localFor(widget).filter(
          (match) => match.fork?.key !== fork.key,
        )
        setNote(
          others.length > 0
            ? `${widget.name} installed. ${others.length === 1 ? 'Another file' : `${others.length} other files`} on this machine also declare this name — BAR loads the first it finds and skips the rest as duplicates.`
            : `${widget.name} installed to modlobby's widget folder. BAR starts a new widget switched off — use Enable to turn it on.`,
        )
      } else if (action === 'disable') {
        await api.widgetDisable(widget.name)
      } else if (action === 'enable') {
        await api.widgetEnable(widget.name)
      } else {
        const deleted = await api.widgetDelete(fork.key)
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

        {/* Off by default: "still using" is the count that notices a widget
            people tried and dropped. */}
        <div class='filter-group' role='group' aria-label='Counting'>
          <Choice
            label='Include used once'
            on={includeUsedOnce()}
            onClick={() => setIncludeUsedOnce(!includeUsedOnce())}
          />
        </div>

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
                    classList={{
                      num: !!column.numeric,
                      'widget-rank': column.key === 'rank',
                      // A column's width is the whole column's, header
                      // included, so the header carries the same class.
                      'widget-name': column.key === 'name',
                    }}
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
                      {column.label(mode())}
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
                <>
                  <Row
                    widget={widget}
                    audience={shownAudience()}
                    window={shown()}
                    mode={mode()}
                    busy={busy()}
                    expanded={expanded().has(widget.key)}
                    onExpand={() => toggleExpanded(widget.key)}
                    enlarged={enlarged()}
                    onEnlarge={showPicture}
                    onAct={act}
                  />
                  <Show when={expanded().has(widget.key)}>
                    <For each={forksOf(widget)}>
                      {(fork) => (
                        <ForkRow
                          widget={widget}
                          fork={fork}
                          audience={shownAudience()}
                          window={shown()}
                          mode={mode()}
                          busy={busy()}
                          enlarged={enlarged()}
                          onEnlarge={showPicture}
                          onAct={act}
                        />
                      )}
                    </For>
                    <For each={yourVersions(widget)}>
                      {(file) => <YourVersionRow file={file} />}
                    </For>
                  </Show>
                </>
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

/** How many versions a row opens to, counting the player's own files. */
function versionCount(widget: WidgetUsage): number {
  return forksOf(widget).length + yourVersions(widget).length
}

/**
 * One widget name: its picture, who made its main version, how the name did,
 * what is on this machine, and what can be done about it.
 */
function Row(props: {
  widget: WidgetUsage
  audience: string
  window: string
  mode: UsingMode
  busy: string | null
  expanded: boolean
  onExpand: () => void
  enlarged: Enlarged | null
  onEnlarge: (key: string, index: number | null, count: number) => void
  onAct: (widget: WidgetUsage, fork: Fork, action: Action) => void
}) {
  const found = () => statsFor(props.widget, props.audience, props.window)
  const stats = () => found()?.stats
  const shownWindow = () => found()?.window ?? props.window
  const configured = () => configuredState(props.widget)
  const main = () => mainFork(props.widget)
  const versions = () => versionCount(props.widget)

  /**
   * A click anywhere in the row opens it, except on the things that already do
   * something of their own -- the chevron, the picture, the action buttons and
   * the source link -- which is why this asks what was clicked rather than
   * stopping their events: a button that has to remember to stop its own event
   * is a button that will one day forget.
   */
  const openOnClick = (event: MouseEvent) => {
    if (versions() <= 1) return
    const on = event.target as HTMLElement | null
    if (on?.closest('button, a, input, select, label')) return
    props.onExpand()
  }

  return (
    <tr
      class='widget-row'
      classList={{
        off: !!configured() && !isEnabled(configured()!),
        open: props.expanded,
        openable: versions() > 1,
      }}
      onClick={openOnClick}
    >
      <td class='num widget-rank'>
        <span>{stats()?.rank ?? '—'}</span>
        <Show when={versions() > 1}>
          <button
            type='button'
            class='widget-expand'
            aria-expanded={props.expanded}
            aria-label={`${props.expanded ? 'Hide' : 'Show'} the ${versions()} versions of ${props.widget.name}`}
            title={`${versions()} versions`}
            onClick={props.onExpand}
          >
            <Glyph id='act-expand' />
          </button>
        </Show>
      </td>
      <td class='widget-name'>
        <div class='widget-identity'>
          <Thumb
            pictureKey={props.widget.key}
            name={props.widget.name}
            images={picturesOf(props.widget)}
            tile={TILE}
            open={
              props.enlarged?.key === props.widget.key
                ? props.enlarged.index
                : null
            }
            onShow={(index, count) =>
              props.onEnlarge(props.widget.key, index, count)
            }
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
      <Numbers stats={stats()} mode={props.mode} dated={props.widget} />
      <td>
        {label(shownWindow())}
        <Show when={stats() && !isRepresentative(stats()!)}>
          <span class='muted'> ({stats()!.days_covered}d)</span>
        </Show>
      </td>
      <td class='widget-source'>
        <Source fork={main()} />
      </td>
      <td class='widget-actions'>
        <Actions
          actions={actionsFor(props.widget)}
          busyKey={main().key}
          busy={props.busy}
          onAct={(action) => props.onAct(props.widget, main(), action)}
        />
      </td>
    </tr>
  )
}

/**
 * One version under a widget name: a publisher's lineage, or the players no
 * source accounts for.
 *
 * It gets install, update and delete of its own, since each is one
 * publisher's files. It never gets enable or disable: those belong to the
 * name, and live on the row above.
 */
function ForkRow(props: {
  widget: WidgetUsage
  fork: Fork
  audience: string
  window: string
  mode: UsingMode
  busy: string | null
  enlarged: Enlarged | null
  onEnlarge: (key: string, index: number | null, count: number) => void
  onAct: (widget: WidgetUsage, fork: Fork, action: Action) => void
}) {
  const stats = () =>
    forkStatsFor(props.fork, props.audience, props.window) ?? undefined
  const here = () =>
    localForFork(props.widget, props.fork).length > 0 ||
    installedEntry(props.fork.key) !== null
  const other = () => props.fork.kind === 'other'

  return (
    <tr
      class='widget-fork'
      classList={{ main: props.fork.main, other: other() }}
    >
      <td />
      <td class='widget-name'>
        <div class='widget-identity fork'>
          <span class='widget-expand-space' />
          <Show
            when={!other()}
            fallback={<span class='widget-fork-tile' aria-hidden='true' />}
          >
            <Thumb
              pictureKey={props.fork.key}
              name={props.fork.author || props.widget.name}
              images={picturesOf(props.fork)}
              tile={FORK_TILE}
              open={
                props.enlarged?.key === props.fork.key
                  ? props.enlarged.index
                  : null
              }
              onShow={(index, count) =>
                props.onEnlarge(props.fork.key, index, count)
              }
            />
          </Show>
          <div class='widget-text'>
            <span class='widget-fork-kind'>
              {other()
                ? 'Other versions'
                : props.fork.main
                  ? 'Main version'
                  : 'Fork'}
            </span>
            <Show when={!other() && props.fork.author}>
              <span class='widget-by'> by {props.fork.author}</span>
            </Show>
            <Show when={other()}>
              <p class='widget-about'>
                Players whose file matches no version anyone has published.
              </p>
            </Show>
            <Show when={!other() && props.fork.install.license}>
              {(licence) => (
                <div class='chips'>
                  <span class='chip'>{licence()}</span>
                  <Show when={here()}>
                    <span class='chip ok'>On this machine</span>
                  </Show>
                </div>
              )}
            </Show>
            <Show when={!props.fork.install.license && here()}>
              <div class='chips'>
                <span class='chip ok'>On this machine</span>
              </div>
            </Show>
          </div>
        </div>
      </td>
      <Numbers stats={stats()} mode={props.mode} dated={props.fork} />
      <td />
      <td class='widget-source'>
        <Show when={!other()} fallback={<span class='muted'>—</span>}>
          <Source fork={props.fork} />
        </Show>
      </td>
      <td class='widget-actions'>
        <Actions
          actions={forkActions(props.fork)}
          busyKey={props.fork.key}
          busy={props.busy}
          onAct={(action) => props.onAct(props.widget, props.fork, action)}
        />
      </td>
    </tr>
  )
}

/**
 * A file on this machine that declares the name but matches no published
 * version — a private homebrew, or an edited copy.
 *
 * No numbers and no actions: nobody else runs it, and modlobby did not put it
 * there, so it is not modlobby's to remove.
 */
function YourVersionRow(props: { file: LocalWidget }) {
  return (
    <tr class='widget-fork yours'>
      <td />
      <td class='widget-name'>
        <div class='widget-identity fork'>
          <span class='widget-expand-space' />
          <span class='widget-fork-tile' aria-hidden='true' />
          <div class='widget-text'>
            <span class='widget-fork-kind'>Your version</span>
            <p
              class='widget-about'
              title={`${props.file.dir}/${props.file.file}`}
            >
              <code>{props.file.file.replace(/^LuaUI\/Widgets\//, '')}</code> in{' '}
              {locationOf(props.file)}
            </p>
          </div>
        </div>
      </td>
      <td class='num muted'>—</td>
      <td class='num muted'>—</td>
      <td class='num muted'>—</td>
      <td class='num muted'>—</td>
      <td class='num muted'>—</td>
      <td class='num muted'>—</td>
      <td class='num muted'>—</td>
      <td />
      <td class='widget-source'>
        <span class='muted'>local</span>
      </td>
      <td class='widget-actions'>
        <span class='muted'>—</span>
      </td>
    </tr>
  )
}

/**
 * The five number cells, counted the way the page is counting.
 *
 * A fork the anonymity floor withheld has its numbers zeroed in the document;
 * it reads "few" here rather than a zero, which would be a false statement.
 */
/**
 * A published date as "1y 1m ago", with the exact moment on hover. Not
 * withheld like the counts beside it: when a widget was published is its
 * author's public fact, not something about the players who run it.
 */
function When(props: { iso: string }) {
  return (
    <td class='num' title={exactly(props.iso)}>
      {age(props.iso) || '—'}
    </td>
  )
}

function Numbers(props: {
  stats: WindowStats | undefined
  mode: UsingMode
  dated: { first_published: string; last_updated: string }
}) {
  const withheld = () => props.stats?.withheld ?? false
  const cell = (value: () => string) => (
    <td class='num' classList={{ muted: withheld() }}>
      <Show
        when={props.stats && !withheld()}
        fallback={
          <Show when={withheld()} fallback='—'>
            <span title='Too few players to show without identifying them'>
              few
            </span>
          </Show>
        }
      >
        {value()}
      </Show>
    </td>
  )
  return (
    <>
      {cell(() => props.stats!.players.toLocaleString())}
      {cell(() => `${Math.round(usingShare(props.stats!, props.mode) * 100)}%`)}
      {cell(() => notUsing(props.stats!, props.mode).toLocaleString())}
      <When iso={props.dated.last_updated} />
      <When iso={props.dated.first_published} />
      {cell(() => props.stats!.replays.toLocaleString())}
      {cell(() => props.stats!.sightings.toLocaleString())}
    </>
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
  pictureKey: string
  name: string
  images: readonly string[]
  tile: { width: number; height: number }
  /** Which picture the gallery shows, or null when it is closed. */
  open: number | null
  onShow: (index: number | null, count: number) => void
}) {
  const [failed, setFailed] = createSignal(false)
  const [peeking, setPeeking] = createSignal(false)
  const src = (box: { width: number; height: number }, index: number) => {
    const tile = devicePixels(box)
    return thumbSrc(
      `widget/${tile.width}x${tile.height}/${index}/${props.pictureKey}`,
    )
  }
  const count = () => props.images.length
  const show = (index: number | null) => props.onShow(index, count())
  const pictured = () => count() > 0 && !failed()
  // Only the width: the height comes from the row the tile stands in, which
  // the stylesheet stretches it to.
  const size = () => ({ width: `${props.tile.width / 16}rem` })
  // Beside the tile while it is looked at: the next few pictures, the rest
  // counted on the last. Rendered only then, so rows nobody hovers fetch
  // nothing more, and gone as soon as the pointer or focus leaves them.
  const companions = () =>
    props.images.slice(1, 1 + COMPANIONS).map((_, i) => i + 1)
  const more = () => count() - 1 - COMPANIONS
  const leave = (event: FocusEvent) => {
    const anchor = event.currentTarget as HTMLElement
    if (!anchor.contains(event.relatedTarget as Node | null)) setPeeking(false)
  }

  return (
    <Show
      when={pictured()}
      fallback={
        <span
          class='widget-thumb placeholder'
          style={{ '--hue': String(hue(props.name)), ...size() }}
          aria-hidden='true'
        >
          {initials(props.name)}
        </span>
      }
    >
      <span
        class='widget-thumb-anchor'
        onPointerLeave={() => setPeeking(false)}
        onFocusOut={leave}
      >
        <button
          type='button'
          class='widget-thumb'
          style={size()}
          aria-label={`Show a larger picture of ${props.name}`}
          aria-expanded={props.open !== null}
          onPointerEnter={() => setPeeking(true)}
          onFocus={() => setPeeking(true)}
          onClick={() => show(props.open === null ? 0 : null)}
        >
          <img
            src={src(props.tile, 0)}
            width={props.tile.width}
            height={props.tile.height}
            loading='lazy'
            alt=''
            onError={() => setFailed(true)}
          />
        </button>
        <Show when={peeking() && count() > 1 && props.open === null}>
          <span class='widget-companions'>
            <For each={companions()}>
              {(index, position) => (
                <button
                  type='button'
                  class='widget-thumb companion'
                  style={size()}
                  aria-label={`Show picture ${index + 1} of ${count()} of ${props.name}`}
                  onClick={() => show(index)}
                >
                  <img
                    src={src(props.tile, index)}
                    width={props.tile.width}
                    height={props.tile.height}
                    alt=''
                  />
                  <Show
                    when={position() === companions().length - 1 && more() > 0}
                  >
                    <span class='widget-more'>+{more()}</span>
                  </Show>
                </button>
              )}
            </For>
          </span>
        </Show>
        <Show when={props.open !== null}>
          <div class='widget-preview'>
            <button
              type='button'
              class='widget-preview-picture'
              aria-label='Close the larger picture'
              onClick={() => show(null)}
            >
              <img
                src={src(PREVIEW, props.open ?? 0)}
                width={PREVIEW.width}
                height={PREVIEW.height}
                alt={props.name}
              />
            </button>
            <Show when={count() > 1}>
              <div
                class='widget-film'
                role='group'
                aria-label={`Pictures of ${props.name}`}
              >
                <For each={props.images}>
                  {(_, index) => (
                    <button
                      type='button'
                      class='widget-film-tile'
                      aria-current={index() === props.open ? 'true' : undefined}
                      aria-label={`Picture ${index() + 1} of ${count()}`}
                      onClick={() => show(index())}
                    >
                      <img
                        src={src(FILM, index())}
                        width={FILM.width}
                        height={FILM.height}
                        loading='lazy'
                        alt=''
                      />
                    </button>
                  )}
                </For>
              </div>
            </Show>
          </div>
        </Show>
      </span>
    </Show>
  )
}

/**
 * Which file on disk answers to this widget's name, and which version it is.
 *
 * The distinction BAR's config cannot make: a name there covers every file
 * that declares it. Two such files means BAR loads one and rejects the other
 * as a duplicate, which is worth saying before somebody wonders which one is
 * running.
 */
function OnThisMachine(props: { widget: WidgetUsage }) {
  const found = createMemo(() => localFor(props.widget))
  const whose = (fork: Fork) =>
    fork.main
      ? 'Main version'
      : fork.author
        ? `${fork.author}'s version`
        : 'A fork'
  return (
    <Show when={found().length > 0}>
      <ul class='widget-local'>
        <For each={found()}>
          {(match) => (
            <li
              classList={{ exact: match.fork !== null }}
              title={`${match.file.dir}/${match.file.file}`}
            >
              <span class='widget-local-kind'>
                {match.fork ? whose(match.fork) : 'Your own version'}
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
 * Where a version comes from — a button whenever there is somewhere to go.
 *
 * "A link for all of them" is the rule: a widget we may not redistribute, and
 * even one we cannot install at all, still points at wherever it lives. A
 * button rather than a link because it leaves the app: the system browser
 * opens it, and the icon says so before the click.
 */
function Source(props: { fork: Fork }) {
  const install = () => props.fork.install
  const why = () => unavailableBecause(install())
  return (
    <>
      <Show when={install().page} fallback={<span class='muted'>unknown</span>}>
        {(page) => (
          <button
            type='button'
            class='link-button'
            title={page()}
            onClick={() => void openExternal(page())}
          >
            <span>{SOURCE_LABEL[install().kind] ?? install().kind}</span>
            <Glyph id='act-external' />
          </button>
        )}
      </Show>
      <Show when={why()}>
        {(reason) => <p class='muted widget-why'>{reason()}</p>}
      </Show>
    </>
  )
}

function Actions(props: {
  actions: Action[]
  busyKey: string
  busy: string | null
  onAct: (action: Action) => void
}) {
  const locked = () => status()?.locked ?? false
  /** Disable, enable and delete all write the config, which BAR would clobber. */
  const writesConfig = (action: Action) => action !== 'install'

  return (
    <Show
      when={props.actions.length > 0}
      fallback={<span class='muted'>—</span>}
    >
      <div class='widget-buttons'>
        <For each={props.actions}>
          {(action) => (
            <button
              type='button'
              class='small'
              classList={{ danger: action === 'delete' }}
              disabled={
                props.busy === `${props.busyKey}:${action}` ||
                (locked() && writesConfig(action))
              }
              title={
                locked() && writesConfig(action)
                  ? 'A game is running. BAR rewrites its widget config on exit and would discard this.'
                  : undefined
              }
              onClick={() => props.onAct(action)}
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
  const main = () => mainFork(props.widget)
  return (
    <div class='chips'>
      <Show when={main().install.kind === 'hub'}>
        <span class='chip ok'>On the Widget Hub</span>
      </Show>
      <Show when={configured() && !isEnabled(configured()!)}>
        <span class='chip'>Switched off</span>
      </Show>
      <Show when={main().install.license}>
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
