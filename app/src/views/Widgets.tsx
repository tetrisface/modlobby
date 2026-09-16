import { For, Show, createMemo, createResource, createSignal } from 'solid-js'
import type { WidgetUsage } from '../ipc/bindings/WidgetUsage'
import type { WindowStats } from '../ipc/bindings/WindowStats'
import {
  DEFAULT_WINDOW,
  WINDOW_ORDER,
  disabledOnly,
  isRepresentative,
  loadWidgetUsage,
  ranked,
  statsFor,
  usage,
  windows,
} from '../store/widgets'

/**
 * What BAR players actually run, ranked.
 *
 * BAR's own Widget Hub says what is offered; this says what is used. The
 * numbers are pve.bar's weekly projection over public replays, counted in
 * distinct players rather than sightings, so one enthusiast playing all
 * evening does not read as a crowd. Nobody is named, here or upstream.
 *
 * A window is a choice rather than a setting: it is the question being asked —
 * what is popular this week, what has lasted a year — and the answer to both
 * is in the one document, so switching costs nothing.
 */

/** What each window is called, rather than what it is keyed by. */
const WINDOW_LABEL: Record<string, string> = {
  '7d': '7 days',
  '30d': '30 days',
  '90d': '90 days',
  '365d': '1 year',
  all: 'All time',
}

const label = (window: string) => WINDOW_LABEL[window] ?? window

export function Widgets() {
  const [fetched] = createResource(async () => {
    await loadWidgetUsage()
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

  const listed = createMemo(() => ranked(shown()))
  /** The day the pipeline built this, in the reader's own format. */
  const built = () => {
    const at = usage()?.generated_at
    if (!at) return ''
    const when = new Date(at)
    return Number.isNaN(when.getTime()) ? '' : when.toLocaleDateString()
  }

  return (
    <section class='widgets'>
      <h1>Widgets</h1>

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

      <Show
        when={listed().length > 0}
        fallback={<Empty loading={fetched.loading} />}
      >
        <p class='widgets-about'>
          What players actually run, from pve.bar's weekly read of public
          replays. Counted in distinct players
          <Show when={built()}>{(day) => <>, built {day()}</>}</Show>. A widget
          too few people run to be counted anonymously is left out.
        </p>
        <div class='widget-list'>
          <For each={listed()}>
            {(widget, at) => (
              <Card widget={widget} window={shown()} place={at() + 1} />
            )}
          </For>
        </div>
      </Show>
    </section>
  )
}

function Empty(props: { loading: boolean }) {
  return (
    <p class='muted'>
      <Show when={!props.loading} fallback='Fetching what people run…'>
        Widget usage could not be fetched. It comes from pve.bar, so this is
        what an offline launch looks like too.
      </Show>
    </p>
  )
}

/**
 * One widget: who made it, what it is, and how it did over the window.
 *
 * The stats are read through the store's accessor rather than off the widget,
 * but its fallback never fires here: a widget is on this list because the
 * window carries a row for it. One that the window withheld is simply not
 * listed — the anonymity floor is applied inside each window, and a widget
 * with six players this year and two this week is absent from the week rather
 * than shown with a two.
 */
function Card(props: { widget: WidgetUsage; window: string; place: number }) {
  const stats = () => statsFor(props.widget, props.window)?.stats

  return (
    <article class='widget-card'>
      <span class='widget-rank'>{props.place}</span>
      <div class='widget-text'>
        <h2>{props.widget.name}</h2>
        <Show when={props.widget.author}>
          <p class='widget-by'>{props.widget.author}</p>
        </Show>
        <Show when={props.widget.description}>
          <p class='widget-about'>{props.widget.description}</p>
        </Show>
        <Show when={stats()}>
          {(found) => <Marks widget={props.widget} stats={found()} />}
        </Show>
      </div>
      <Show when={stats()}>{(found) => <Numbers stats={found()} />}</Show>
    </article>
  )
}

/** What is worth saying about a widget besides its numbers. */
function Marks(props: { widget: WidgetUsage; stats: WindowStats }) {
  const days = () => props.stats.days_covered
  return (
    <div class='chips'>
      <Show when={props.widget.resolved && props.widget.id}>
        <span class='chip ok'>On the Widget Hub</span>
      </Show>
      {/* The pipeline backfills history a slice at a time, so a year window
          can hold a fortnight. Saying so beats implying a year of evidence. */}
      <Show when={!isRepresentative(props.stats)}>
        <span class='chip warn'>
          {days()} {days() === 1 ? 'day' : 'days'} harvested
        </span>
      </Show>
    </div>
  )
}

function Numbers(props: { stats: WindowStats }) {
  const off = () => disabledOnly(props.stats)
  return (
    <dl class='widget-stats'>
      <div>
        <dt>Players</dt>
        <dd>{props.stats.players.toLocaleString()}</dd>
      </div>
      {/* Install-and-keep. A widget people install and then switch off scores
          low here and nowhere else, which is the one thing a download count
          cannot tell you. */}
      <div>
        <dt>Kept on</dt>
        <dd>{Math.round(props.stats.retention * 100)}%</dd>
      </div>
      <Show when={off() > 0}>
        <div>
          <dt>Switched off</dt>
          <dd>{off().toLocaleString()}</dd>
        </div>
      </Show>
    </dl>
  )
}
