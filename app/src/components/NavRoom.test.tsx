import { MemoryRouter, Route, createMemoryHistory } from '@solidjs/router'
import { cleanup, render } from '@solidjs/testing-library'
import { createStore } from 'solid-js/store'
import { afterEach, describe, expect, test, vi } from 'vitest'
import type { BattleView } from '../ipc/bindings/BattleView'
import { NavRoom, glance } from './NavRoom'

vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: (path: string, scheme: string) =>
    `${scheme}://${encodeURIComponent(path)}`,
}))

const battle = (over: Partial<BattleView> = {}): BattleView => ({
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
  title: 'EU 8v8 Rookies Only Please',
  gameName: '',
  members: [],
  spectatorCount: 3,
  playerCount: 12,
  layout: null,
  bots: [],
  startRects: [],
  ...over,
})

/** Lets a router transition reach the DOM. */
async function settle() {
  for (let turn = 0; turn < 8; turn++) await Promise.resolve()
}

/**
 * Mounted at `at`, fed a store node the way Layout feeds it a row of
 * `lobby.battles`. `A` refuses to render outside a router, so the card sits
 * under a memory router with its history set before the first render.
 */
function mount(at: string, b: BattleView) {
  const history = createMemoryHistory()
  history.set({ value: at })
  const [room, setRoom] = createStore(b)
  const rendered = render(() => (
    <MemoryRouter history={history}>
      <Route path='*' component={() => <NavRoom battle={room} />} />
    </MemoryRouter>
  ))
  return { ...rendered, history, setRoom }
}

afterEach(cleanup)

describe('NavRoom', () => {
  test('shows the map, the title and the count in the list form', () => {
    const { container } = mount('/battles', battle())
    expect(
      container.querySelector('.nav-room-pic img')?.getAttribute('src'),
    ).toBe('thumb://40x28%2FSupreme%20Isthmus%20v2.1')
    expect(container.querySelector('.nav-room-title')?.textContent).toBe(
      'EU 8v8 Rookies Only Please',
    )
    expect(container.querySelector('.nav-room-count')?.textContent).toBe(
      '12/16 +3',
    )
  })

  test('the DOM never cuts; the tooltip names the map and the whole title', () => {
    const long = battle({ title: 'x'.repeat(93), spectatorCount: 0 })
    const { container } = mount('/battles', long)
    const a = container.querySelector('a.nav-room') as HTMLAnchorElement
    expect(container.querySelector('.nav-room-title')?.textContent).toBe(
      long.title,
    )
    expect(container.querySelector('.nav-room-count')?.textContent).toBe(
      '12/16 +0',
    )
    expect(a.title).toBe(glance(long))
    expect(a.title).toContain('Supreme Isthmus v2.1')
    expect(a.title).toContain('12 players · 0 spectators')
  })

  test('is the active link on the room page and its tweak page only', async () => {
    const { container, history } = mount('/room/tweaks', battle())
    expect(container.querySelector('a.nav-room.active')).not.toBeNull()
    history.set({ value: '/chat' })
    await settle()
    expect(container.querySelector('a.nav-room.active')).toBeNull()
  })

  test('a count ticking over changes the digits and nothing else', () => {
    const { container, setRoom } = mount('/room', battle())
    const before = container.querySelector('a.nav-room')
    setRoom('playerCount', 13)
    expect(container.querySelector('a.nav-room')).toBe(before)
    expect(container.querySelector('.nav-room-count')?.textContent).toBe(
      '13/16 +3',
    )
  })
})
