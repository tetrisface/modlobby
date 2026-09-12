import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { reconcile } from 'solid-js/store'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { LanGameView } from '../ipc/bindings/LanGameView'
import type { SkirmishView } from '../ipc/bindings/SkirmishView'
import { battle, myBattle, user } from '../views/room/fixture'
import { emptyLobby, setLobby } from '../store/lobby'
import { LanGames } from './LanGames'

/**
 * Playing with the people in the building, over the real mirrored state.
 *
 * `lobby.lan` is written the way a delta writes it and the strip reads it,
 * so what is asserted is what a machine on a real network would draw — and
 * what it asks Rust for is asserted as the command names and payloads.
 */
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

async function settle() {
  for (let turn = 0; turn < 8; turn++) await Promise.resolve()
}

function sent(command: string) {
  return vi
    .mocked(invoke)
    .mock.calls.filter(([name]) => name === command)
    .map(([, args]) => args)
}

/** A room of our own, closed to the network, as `SkirmishRoom` opens one. */
function room(port = 0, members: string[] = ['me']): SkirmishView {
  return {
    battle: {
      ...battle({ title: 'Skirmish', founder: 'me' }),
      id: 0,
      port,
      members,
    },
    my: myBattle({ boss: 'me', id: 0 }),
    users: members.map((name) => user(name)),
    me: 'me',
    content: { engine: true, game: true, map: true },
  }
}

/** Somebody else's game, as it arrives off the network. */
function announced(over: Partial<LanGameView> = {}): LanGameView {
  return {
    id: 'abc123',
    title: 'Friday',
    host: 'ann',
    address: '192.168.1.20',
    engine: '2026.07.04',
    game: 'Beyond All Reason test-31134',
    map: 'Supreme Isthmus v2.1',
    seats: ['me'],
    running: false,
    content: { engine: true, game: true, map: true },
    ...over,
  }
}

beforeEach(() => {
  vi.mocked(invoke).mockResolvedValue(null)
  setLobby(reconcile(emptyLobby()))
  setLobby('skirmish', room())
  setLobby('lan', 'listening', true)
})

afterEach(() => {
  cleanup()
  setLobby(reconcile(emptyLobby()))
  vi.clearAllMocks()
})

describe('playing over the network', () => {
  test('a machine with nothing near it draws one line', async () => {
    const { container, getByText } = render(() => <LanGames />)
    await settle()

    expect(getByText('Play over the network')).toBeTruthy()
    // Nothing else: no empty list, no section header, no explanation of a
    // feature nobody on this network is using.
    expect(container.querySelector('.lan-games')).toBeNull()
    expect(container.textContent).not.toContain('Expecting')
    expect(container.textContent).not.toContain('Also here')
  })

  test('opening the room asks for the engine port and says so', async () => {
    const { getByText } = render(() => <LanGames />)
    await settle()
    fireEvent.click(getByText('Play over the network'))
    await settle()
    expect(sent('skirmish_act')).toEqual([
      { act: { type: 'setLan', port: 8452 } },
    ])

    // The room is what says whether it is open, through the field that means
    // that online too -- so the strip cannot disagree with the start script.
    setLobby('skirmish', room(8452))
    await settle()
    expect(getByText('Close to the network')).toBeTruthy()
  })

  test('closing it again sends the other half', async () => {
    setLobby('skirmish', room(8452))
    const { getByText } = render(() => <LanGames />)
    await settle()
    fireEvent.click(getByText('Close to the network'))
    await settle()
    expect(sent('skirmish_act')).toEqual([
      { act: { type: 'setLan', port: null } },
    ])
  })

  /**
   * The reason the people on the network are listed at all: the engine's
   * server admits a joining client only under a name the start script lists,
   * so a name spelled wrong is a guest who cannot get in — and the mistake
   * shows up on their machine, as a refused connection, with nothing on
   * screen to explain it.
   */
  test('a guest is picked off the network rather than spelled', async () => {
    setLobby('skirmish', room(8452))
    setLobby('lan', 'people', ['ann', 'bo'])
    const { getByText, queryByText } = render(() => <LanGames />)
    await settle()

    fireEvent.click(getByText('+ ann'))
    await settle()
    // Only the name: where they sit is the room's to decide.
    expect(sent('skirmish_act')).toEqual([
      { act: { type: 'addGuest', name: 'ann' } },
    ])

    // Once expected, they move from the offer to the list, and the button
    // there takes them back out.
    setLobby('skirmish', room(8452, ['me', 'ann']))
    await settle()
    expect(queryByText('+ ann')).toBeNull()
    fireEvent.click(getByText('ann ×'))
    await settle()
    expect(sent('skirmish_act')).toHaveLength(2)
    expect(sent('skirmish_act')[1]).toEqual({
      act: { type: 'removeGuest', name: 'ann' },
    })
  })

  test('nobody is offered while the room is closed', async () => {
    setLobby('lan', 'people', ['ann'])
    const { queryByText } = render(() => <LanGames />)
    await settle()
    expect(queryByText('+ ann')).toBeNull()
  })

  test('a game on the network is joined by its announcement, not its address', async () => {
    setLobby('lan', 'games', [announced()])
    const { getByText, container } = render(() => <LanGames />)
    await settle()

    expect(getByText('Friday')).toBeTruthy()
    expect(container.textContent).toContain('192.168.1.20')
    fireEvent.click(getByText('Join'))
    await settle()
    // The id, so a game that stopped being announced between the row being
    // drawn and the click is refused rather than joined at an address that
    // may since be somebody else's machine.
    expect(sent('join_lan_game')).toEqual([{ id: 'abc123', asName: null }])
  })

  /**
   * Every reason a row cannot be joined is said rather than drawn as a dead
   * button. The last one is the engine's own rule, and saying it is the
   * difference between "ask them to add you" and a connection refused with
   * nothing on screen to explain it.
   */
  test.each([
    [{ running: true }, 'already started'],
    [
      { content: { engine: true, game: true, map: false } },
      'you are missing the map',
    ],
    [{ seats: ['ann', 'bo'] }, 'ann has not added you'],
  ])('a game that cannot be joined says why (%o)', async (over, why) => {
    setLobby('lan', 'games', [announced(over as Partial<LanGameView>)])
    const { getByText, queryByText } = render(() => <LanGames />)
    await settle()

    expect(getByText(why)).toBeTruthy()
    expect(queryByText('Join')).toBeNull()
  })

  /**
   * An empty list means two different things and only one of them is "nobody
   * is playing". It is only worth saying to somebody who is trying to host,
   * which is the only time it costs them anything.
   */
  test('a machine that could not listen says so, and only when it matters', async () => {
    setLobby('lan', 'listening', false)
    const closed = render(() => <LanGames />)
    await settle()
    expect(closed.queryByText('Not announcing')).toBeNull()
    closed.unmount()

    setLobby('skirmish', room(8452))
    const open = render(() => <LanGames />)
    await settle()
    expect(open.getByText('Not announcing')).toBeTruthy()
  })
})
