import { MemoryRouter, Route } from '@solidjs/router'
import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { ModOption } from '../../ipc/bindings/ModOption'
import { Room } from '../Room'
import {
  ALONE,
  SERVED,
  battle,
  bot,
  fakeRoom,
  myBattle,
  recordingIo,
  user,
  type Calls,
} from './fixture'
import { status } from './fixture'
import { RoomProvider, type RoomModel } from './model'

/**
 * The room, drawn with nothing behind it.
 *
 * This is what the seam is for: the whole battle room -- teams, seats, the
 * minimap, BAR's settings table -- rendered from plain values and a recording
 * `io`, with no session, no `lobby` store and no Tauri answering anything the
 * room actually asks of its room. What is mocked below is only the calls that
 * take what they need as arguments and would give the same answer either way.
 */
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
  convertFileSrc: (path: string, scheme: string) => `${scheme}://${path}`,
}))

const OPTIONS: ModOption[] = [
  { key: 'options', name: 'Options', desc: '', type: 'section', weight: 1 },
  {
    key: 'ranked_game',
    name: 'Ranked',
    desc: 'Counts towards rating',
    type: 'bool',
    section: 'options',
    def: true,
  },
]

/** The names on the player rows inside one part of the page. */
function named(container: HTMLElement, within: string): string[] {
  return [...container.querySelectorAll(`${within} .pname`)].map(
    (cell) => cell.textContent ?? '',
  )
}

/** The labels of the buttons inside one part of the page. */
function buttons(container: HTMLElement, within: string): string[] {
  const host = container.querySelector(within)
  if (host === null) return []
  return [...host.querySelectorAll('button')].map((b) => b.textContent ?? '')
}

/** Lets awaited answers reach the component and the DOM. */
async function settle() {
  for (let turn = 0; turn < 8; turn++) await Promise.resolve()
}

beforeEach(() => {
  vi.mocked(invoke).mockImplementation(async (command: string) => {
    switch (command) {
      case 'game_modoptions':
        return OPTIONS
      case 'game_ais':
        return [{ name: 'BARb', desc: '' }]
      case 'game_unit_names':
        return []
      case 'engine_def_tags':
        return { weapon: [] }
      default:
        return null
    }
  })
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

async function open(model: RoomModel) {
  const result = render(() => (
    <MemoryRouter
      root={(props) => (
        <RoomProvider value={model}>{props.children}</RoomProvider>
      )}
    >
      <Route path='/' component={Room} />
    </MemoryRouter>
  ))
  await settle()
  return result
}

/** A room of one's own: one human, one AI on the other side, nobody in charge. */
function alone(calls: Calls): RoomModel {
  return fakeRoom({
    caps: ALONE,
    log: '#skirmish',
    battle: () =>
      battle({ title: 'Skirmish', founder: 'me', bots: [bot('BARb')] }),
    my: () => myBattle({ scriptTags: { 'game/modoptions/ranked_game': '0' } }),
    users: () => ({ me: user('me') }),
    io: recordingIo(calls),
  })
}

/** Me on the first team, an AI on the third: the second is an empty gap. */
function gapped(): RoomModel {
  return fakeRoom({
    caps: ALONE,
    battle: () =>
      battle({
        bots: [
          bot('BARb', {
            status: status({ allyTeam: 2, team: 1, sync: 'bot' }),
          }),
        ],
      }),
    users: () => ({
      me: user('me', { battleStatus: status({ allyTeam: 0 }) }),
    }),
  })
}

describe('choosing a team', () => {
  test('an empty team between two full ones can still be taken', async () => {
    const { container } = await open(gapped())

    const picker = container.querySelector<HTMLSelectElement>('.seat select')
    const offered = [...(picker?.options ?? [])].map((o) => o.textContent)
    // Team 2 is empty and sits below an occupied team 3. It used to be
    // missing entirely: the list was the teams in use plus one past the top.
    expect(offered).toContain('New team 2')
    expect(offered).toContain('Join team 3')
  })

  test('the team you are on reads as yours, not as one to join', async () => {
    const { container } = await open(gapped())

    const picker = container.querySelector<HTMLSelectElement>('.seat select')
    const offered = [...(picker?.options ?? [])].map((o) => o.textContent)
    expect(offered[0]).toBe('Team 1')
  })
})

describe('a room behind the seam', () => {
  test('draws teams, the map and the settings with no server at all', async () => {
    const { container, getByText } = await open(alone([]))

    expect(getByText('Skirmish')).toBeTruthy()
    expect(getByText('Comet Catcher Remake 1.8')).toBeTruthy()
    // The human on team 1 and the AI on team 2, from `lib/roster`'s arrange().
    expect(container.querySelectorAll('.team').length).toBe(2)
    expect(named(container, '.team')).toEqual(['me', 'BARb'])
  })

  test('a change goes to the room it was given, not to a server', async () => {
    const calls: Calls = []
    const { container } = await open(alone(calls))

    const box = container.querySelector(
      '.opt input[type=checkbox]',
    ) as HTMLInputElement
    expect(box).toBeTruthy()
    fireEvent.change(box, { target: { checked: true } })
    await settle()

    expect(calls).toContainEqual(['setOption', ['ranked_game', '1']])
    // Nothing reached Tauri: `set_option` is the room's, not a bare command.
    expect(vi.mocked(invoke).mock.calls.map(([name]) => name)).not.toContain(
      'set_option',
    )
  })

  test('what only a host can offer is not offered where there is no host', async () => {
    const { container, queryByText } = await open(alone([]))

    // The host bar's commands, the vote buttons and the hosting buttons.
    expect(queryByText('Balance')).toBeNull()
    expect(queryByText('Force start')).toBeNull()
    expect(queryByText('Host a public room')).toBeNull()
    expect(queryByText('Private room')).toBeNull()
    expect(queryByText('Leave')).toBeNull()
    // Nobody to be ready for. Asked of the seat bar rather than of the page,
    // because a player row's sync icon is titled "Not ready" as well.
    expect(buttons(container, '.seat')).not.toContain('Not ready')
  })

  test('the game is started from here when nobody else will start it', async () => {
    const calls: Calls = []
    const { container } = await open(alone(calls))

    expect(buttons(container, '.card-actions')).toEqual(['Start'])
    fireEvent.click(container.querySelector('.card-actions button')!)
    await settle()
    expect(calls).toContainEqual(['launch', []])
  })

  test('a served room keeps its host bar and starts no game of its own', async () => {
    const { container } = await open(
      fakeRoom({
        caps: SERVED,
        my: () => myBattle({ boss: 'me' }),
        io: recordingIo([]),
      }),
    )

    // Asked of the card, because the host bar has a Start of its own -- and
    // that one, `!start`, is exactly what a served room should still offer.
    expect(buttons(container, '.card-actions')).toEqual(['Leave'])
    expect(buttons(container, '.host-bar')).toContain('Start')
    expect(buttons(container, '.host-bar')).toContain('Force start')
    // A listed room is named for other people and run by somebody, so it says
    // whose it is and offers the pen. The skirmish test asserts neither.
    expect(container.textContent).toContain('Host')
    expect(container.querySelector('.room-title button')).toBeTruthy()
  })
})
