import { MemoryRouter, Route } from '@solidjs/router'
import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { createSignal } from 'solid-js'
import { afterEach, describe, expect, test, vi } from 'vitest'
import type { BattleView } from '../ipc/bindings/BattleView'
import { NavTabs, type Measure, type NavKey } from './NavTabs'

vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: (path: string, scheme: string) => `${scheme}://${path}`,
}))

/** Widths the test's DOM would not have. The room card's is its floor. */
const NEEDS: Record<NavKey, number> = {
  battles: 60,
  skirmish: 70,
  chat: 40,
  news: 45,
  replays: 65,
  settings: 70,
  login: 50,
  room: 230,
}
const GAP = 18
const MENU = 30
/** The six links and the gaps between them. */
const ALL_SIX = 350 + 5 * GAP

type Props = Parameters<typeof NavTabs>[0]

const battle = (): BattleView => ({
  id: 7,
  founder: '[teh]host',
  ip: '',
  port: 0,
  maxPlayers: 16,
  passworded: false,
  locked: false,
  mapHash: '',
  mapName: 'Supreme Isthmus v2.1',
  engineName: '',
  engineVersion: '',
  title: 'EU 8v8',
  gameName: '',
  members: [],
  spectatorCount: 3,
  playerCount: 12,
  layout: null,
  bots: [],
  startRects: [],
  queue: [],
})

/** Lets the second reckoning, after the first draw, reach the DOM. */
async function settle() {
  for (let turn = 0; turn < 8; turn++) await Promise.resolve()
}

function mount(width: number, over: Partial<Props> = {}) {
  const [room, setRoom] = createSignal(width)
  const measure: Measure = {
    room: () => room(),
    gap: () => GAP,
    need: (_, key) => NEEDS[key],
    menu: () => MENU,
  }
  const rendered = render(() => (
    <MemoryRouter>
      <Route
        path='*'
        component={() => (
          <NavTabs
            room={undefined}
            loggedOut={false}
            unread={0}
            named={false}
            news={0}
            drag={false}
            measure={measure}
            {...over}
          />
        )}
      />
    </MemoryRouter>
  ))
  return { ...rendered, setRoom }
}

const inRow = (container: HTMLElement) =>
  [...container.querySelectorAll('.nav-tabs > a')].map(
    (a) => a.textContent ?? '',
  )
const more = (container: HTMLElement) =>
  container.querySelector<HTMLButtonElement>('.nav-more')
const inMenu = (container: HTMLElement) =>
  [...container.querySelectorAll('.nav-menu a')].map(
    (a) => a.querySelector('.nav-room-title')?.textContent ?? a.textContent,
  )

afterEach(cleanup)

describe('NavTabs', () => {
  test('a row with room for every page folds none of them', async () => {
    const { container } = mount(1000)
    await settle()
    expect(inRow(container)).toEqual([
      'Battles',
      'Skirmish',
      'Chat',
      'News',
      'Replays',
      'Settings',
    ])
    expect(more(container)).toBeNull()
  })

  test('folds from the right, one page at a time, into the menu', async () => {
    // Five pages, the button and their gaps come to exactly this.
    const { container } = mount(ALL_SIX - 70 + MENU)
    await settle()
    expect(inRow(container)).toEqual([
      'Battles',
      'Skirmish',
      'Chat',
      'News',
      'Replays',
    ])
    fireEvent.click(more(container)!)
    expect(inMenu(container)).toEqual(['Settings'])
  })

  test('the room you are in stays when only Battles does', async () => {
    // The card's floor, Battles, the button and two gaps.
    const { container } = mount(230 + 60 + MENU + 2 * GAP, { room: battle() })
    await settle()
    expect(inRow(container)).toEqual(['Battles', 'EU 8v812/16 +3'])
    fireEvent.click(more(container)!)
    expect(inMenu(container)).toEqual([
      'Skirmish',
      'Chat',
      'News',
      'Replays',
      'Settings',
    ])
  })

  test('a row too narrow even for the card puts the room first in the menu', async () => {
    const { container } = mount(60 + MENU + GAP, { room: battle() })
    await settle()
    expect(inRow(container)).toEqual([])
    fireEvent.click(more(container)!)
    expect(inMenu(container)).toEqual([
      'EU 8v8',
      'Battles',
      'Skirmish',
      'Chat',
      'News',
      'Replays',
      'Settings',
    ])
  })

  test('unread in a folded page shows on the button that hides it', async () => {
    // Battles and Skirmish, the button and two gaps: Chat is folded.
    const { container } = mount(130 + MENU + 2 * GAP, {
      unread: 3,
      named: true,
      news: 2,
    })
    await settle()
    expect(inRow(container)).toEqual(['Battles', 'Skirmish'])
    const badge = more(container)?.querySelector('.badge')
    expect(badge?.textContent).toBe('5')
    expect(badge?.className).toContain('named')
  })

  test('a row that widens takes its pages back', async () => {
    const { container, setRoom } = mount(ALL_SIX - 70 + MENU)
    await settle()
    expect(more(container)).not.toBeNull()
    setRoom(1000)
    await settle()
    expect(inRow(container)).toHaveLength(6)
    expect(more(container)).toBeNull()
  })

  test('the menu closes on Escape and after picking a page', async () => {
    const { container } = mount(ALL_SIX - 70 + MENU)
    await settle()
    fireEvent.click(more(container)!)
    expect(container.querySelector('.nav-menu')).not.toBeNull()
    fireEvent.keyDown(window, { key: 'Escape' })
    expect(container.querySelector('.nav-menu')).toBeNull()

    fireEvent.click(more(container)!)
    fireEvent.click(container.querySelector('.nav-menu a')!)
    expect(container.querySelector('.nav-menu')).toBeNull()
  })

  test('the way in is offered while logged out, and folds first', async () => {
    const { container } = mount(1000, { loggedOut: true })
    await settle()
    expect(inRow(container)).toContain('Log in')
    const { container: tight } = mount(ALL_SIX, { loggedOut: true })
    await settle()
    expect(inRow(tight)).not.toContain('Log in')
  })
})
