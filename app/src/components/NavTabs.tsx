import { A } from '@solidjs/router'
import {
  For,
  Show,
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  onMount,
} from 'solid-js'
import type { BattleView } from '../ipc/bindings/BattleView'
import { fitCount } from '../lib/fold'
import { Glyph } from './icons'
import { NavRoom } from './NavRoom'

/**
 * The pages, in the nav's row, folding into a menu as the window narrows.
 *
 * Everything that does not fit goes into the menu at the row's end, one page
 * at a time from the right; the room you are in and Battles give way last.
 * The row measures itself, so a badge appearing or the room card arriving
 * is taken into account the same way a narrower window is. Nothing to the
 * right of the row -- the account, the build, the window's controls -- is
 * touched.
 */

export type NavKey =
  | 'battles'
  | 'skirmish'
  | 'chat'
  | 'news'
  | 'replays'
  | 'settings'
  | 'login'
  | 'room'

type Link = { key: NavKey; href: string; label: string }

/** The pages, in the order the row draws them. */
const LINKS: readonly Link[] = [
  { key: 'battles', href: '/battles', label: 'Battles' },
  { key: 'skirmish', href: '/skirmish', label: 'Skirmish' },
  { key: 'chat', href: '/chat', label: 'Chat' },
  { key: 'news', href: '/news', label: 'News' },
  { key: 'replays', href: '/replays', label: 'Replays' },
  { key: 'settings', href: '/settings', label: 'Settings' },
  { key: 'login', href: '/login', label: 'Log in' },
]

/**
 * Who stays longest as the row narrows: the room you are in, then Battles,
 * then the row's own order from the left.
 */
const KEPT: readonly NavKey[] = [
  'room',
  'battles',
  'skirmish',
  'chat',
  'news',
  'replays',
  'settings',
  'login',
]

/** The fold button's width in rem, for when it is not there to be measured. */
const MENU_REM = 1.875

/**
 * How wide things are. The row's own answer reads the DOM; a test, whose
 * DOM has no widths, hands in its own.
 */
export type Measure = {
  /** The row the pages share. */
  room: (row: HTMLElement) => number
  gap: (row: HTMLElement) => number
  /**
   * What an item needs: its floor where it has one -- the room card shrinks
   * its title down to that -- and otherwise its width as drawn.
   */
  need: (el: HTMLElement, key: NavKey) => number
  /** The button that stands in for the folded pages, when it is drawn. */
  menu: (button: HTMLElement | undefined) => number
}

const DOM: Measure = {
  room: (row) => row.clientWidth,
  gap: (row) => parseFloat(getComputedStyle(row).columnGap) || 0,
  need: (el) => {
    const floor = parseFloat(getComputedStyle(el).minWidth) || 0
    return floor > 0 ? floor : el.offsetWidth
  },
  menu: (button) =>
    button?.offsetWidth ||
    MENU_REM *
      (parseFloat(getComputedStyle(document.documentElement).fontSize) || 16),
}

type Watch = (key: NavKey, el: HTMLElement) => void

export function NavTabs(props: {
  /** The room you are in, when you are in one. */
  room: BattleView | undefined
  /** Whether the way in is offered: there is no session to be in. */
  loggedOut: boolean
  /** Chat's unread count, and whether any of it names you. */
  unread: number
  named: boolean
  /** Unread news. */
  news: number
  /** Whether the row's empty space drags the window. */
  drag: boolean
  measure?: Measure
}) {
  const measure = () => props.measure ?? DOM
  /** The pages there are, most important first. */
  const present = createMemo(() =>
    KEPT.filter((key) => {
      if (key === 'room') return props.room !== undefined
      if (key === 'login') return props.loggedOut
      return true
    }),
  )
  const [folded, setFolded] = createSignal<readonly NavKey[]>([], {
    equals: (a, b) => a.join() === b.join(),
  })
  const isFolded = (key: NavKey) => folded().includes(key)
  const [open, setOpen] = createSignal(false)

  let row!: HTMLDivElement
  let fold: HTMLSpanElement | undefined
  let more: HTMLButtonElement | undefined
  let live = true
  onCleanup(() => (live = false))

  /** Each page's width as last drawn; a folded page keeps the width it had. */
  const widths = new Map<NavKey, number>()
  const drawn = new Map<NavKey, HTMLElement>()

  /**
   * Which pages fold. A page not in the row has no fresh width to reckon
   * with -- none at all if it was never drawn, an old one if the interface
   * has been scaled since -- so one that is kept is drawn, measured, and the
   * reckoning done once more. Once: a row nobody can see, where nothing has
   * a width, must not spin.
   */
  function layout(again: boolean) {
    if (!live) return
    const m = measure()
    for (const [key, el] of drawn) {
      const need = m.need(el, key)
      if (need > 0) widths.set(key, need)
    }
    const keys = present()
    const kept = fitCount(
      keys.map((key) => widths.get(key) ?? 0),
      m.room(row),
      m.gap(row),
      m.menu(more),
    )
    setFolded(keys.slice(kept))
    const unfolded = keys.slice(0, kept).some((key) => !drawn.has(key))
    if (again && unfolded) queueMicrotask(() => layout(false))
  }
  // Whatever changes a page's width or the pages there are: the badges, the
  // room coming and going (read inside), the display font arriving, and the
  // row itself -- only the row is observed, since folding never changes its
  // width, and watching the pages too would answer their own coming and
  // going with another round of notifications.
  createEffect(() => {
    props.unread
    props.news
    layout(true)
  })
  onMount(() => {
    void document.fonts?.ready.then(() => layout(true))
    if (typeof ResizeObserver === 'undefined') return
    const watching = new ResizeObserver(() => layout(true))
    watching.observe(row)
    onCleanup(() => watching.disconnect())
  })

  const watch: Watch = (key, el) => {
    drawn.set(key, el)
    onCleanup(() => drawn.delete(key))
  }

  const badgeOf = (key: NavKey) => {
    if (key === 'chat') return props.unread
    if (key === 'news') return props.news
    return 0
  }
  /** Unread inside the folded pages, shown on the button that hides them. */
  const foldedBadge = () => folded().reduce((sum, key) => sum + badgeOf(key), 0)

  const inRow = (link: Link) =>
    present().includes(link.key) && !isFolded(link.key)

  // The menu closes on a click anywhere else, on Escape, and when there is
  // nothing left in it.
  createEffect(() => {
    if (!open()) return
    const away = (event: MouseEvent) => {
      if (!fold?.contains(event.target as Node)) setOpen(false)
    }
    const escape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setOpen(false)
    }
    window.addEventListener('mousedown', away)
    window.addEventListener('keydown', escape)
    onCleanup(() => {
      window.removeEventListener('mousedown', away)
      window.removeEventListener('keydown', escape)
    })
  })
  createEffect(() => {
    if (folded().length === 0) setOpen(false)
  })

  return (
    <div
      class='nav-tabs'
      ref={row}
      data-tauri-drag-region={props.drag ? true : undefined}
    >
      {/* Battles and chat are the two things that need a session, but the
          links stay: a row that loses half its tabs when the server drops
          reads as an app that has broken, and each view says for itself
          what it is waiting for. Replays and presets are files on this
          machine, so they are here whether or not anyone is logged in. */}
      <For each={LINKS}>
        {(link) => (
          <Show when={inRow(link)}>
            <Tab
              link={link}
              badge={badgeOf(link.key)}
              named={link.key === 'chat' && props.named}
              watch={watch}
            />
          </Show>
        )}
      </For>
      {/* The room you are in, at the end of the tabs. */}
      <Show when={isFolded('room') ? undefined : props.room}>
        {(b) => <RoomTab battle={b()} watch={watch} />}
      </Show>
      <Show when={folded().length > 0}>
        <span class='nav-fold' ref={fold}>
          <button
            ref={more}
            type='button'
            class='nav-more'
            title='More pages'
            aria-label='More pages'
            aria-haspopup='menu'
            aria-expanded={open()}
            onClick={() => setOpen(!open())}
          >
            <Glyph id='act-menu' />
            <Show when={foldedBadge() > 0}>
              <span
                class='badge'
                classList={{ named: isFolded('chat') && props.named }}
              >
                {foldedBadge()}
              </span>
            </Show>
          </button>
          <Show when={open()}>
            <div class='nav-menu' role='menu'>
              {/* The room first: it folds last, so when it is here so is
                  everything else, and it is the one you most likely want. */}
              <Show when={isFolded('room') ? props.room : undefined}>
                {(b) => (
                  <span onClick={() => setOpen(false)}>
                    <NavRoom battle={b()} />
                  </span>
                )}
              </Show>
              <For each={LINKS.filter((link) => isFolded(link.key))}>
                {(link) => (
                  <Tab
                    link={link}
                    badge={badgeOf(link.key)}
                    named={link.key === 'chat' && props.named}
                    onPick={() => setOpen(false)}
                  />
                )}
              </For>
            </div>
          </Show>
        </span>
      </Show>
    </div>
  )
}

/** A page, as a link with its unread count. Measured while it is in the row. */
function Tab(props: {
  link: Link
  badge: number
  named: boolean
  watch?: Watch
  onPick?: () => void
}) {
  let el!: HTMLAnchorElement
  onMount(() => props.watch?.(props.link.key, el))
  return (
    <A href={props.link.href} ref={el} onClick={() => props.onPick?.()}>
      {props.link.label}
      <Show when={props.badge > 0}>
        <span class='badge' classList={{ named: props.named }}>
          {props.badge}
        </span>
      </Show>
    </A>
  )
}

function RoomTab(props: { battle: BattleView; watch: Watch }) {
  let el!: HTMLAnchorElement
  onMount(() => props.watch('room', el))
  return <NavRoom battle={props.battle} ref={el} />
}
