import { MemoryRouter, Route } from '@solidjs/router'
import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { reconcile } from 'solid-js/store'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { ModOption } from '../../ipc/bindings/ModOption'
import type { SkirmishView } from '../../ipc/bindings/SkirmishView'
import { emptyLobby, setLobby } from '../../store/lobby'
import { Room } from '../Room'
import { battle, bot, myBattle, user } from './fixture'
import { localRoom, skirmishIo } from './local'
import { RoomProvider } from './model'

/**
 * The room with no server behind it, over the real mirrored state.
 *
 * Everything below the Tauri boundary is faked and everything above it is the
 * shipping code: `lobby.skirmish` is written the way a delta writes it,
 * `localRoom()` reads it, and the room draws from that. What the room asks for
 * is asserted as the command names and payloads Rust would receive.
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
    desc: '',
    type: 'bool',
    section: 'options',
    def: true,
  },
]

const room: SkirmishView = {
  battle: {
    ...battle({ title: 'Skirmish', founder: 'me', bots: [bot('BARb')] }),
    id: 0,
  },
  my: myBattle({
    boss: 'me',
    id: 0,
    scriptTags: { 'game/modoptions/ranked_game': '0' },
  }),
  users: [user('me')],
  me: 'me',
  content: { engine: true, game: true, map: true },
}

async function settle() {
  for (let turn = 0; turn < 8; turn++) await Promise.resolve()
}

/** The commands that reached Tauri, with what they carried. */
function sent(command: string) {
  return vi
    .mocked(invoke)
    .mock.calls.filter(([name]) => name === command)
    .map(([, args]) => args)
}

beforeEach(() => {
  setLobby(reconcile(emptyLobby()))
  setLobby('skirmish', room)
  vi.mocked(invoke).mockImplementation(async (command: string) => {
    switch (command) {
      case 'game_modoptions':
        return OPTIONS
      case 'game_ais':
        return [{ name: 'BARb', desc: '' }]
      case 'skirmish_options':
        return { games: [], maps: [], engines: [], ais: [] }
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
  setLobby(reconcile(emptyLobby()))
  vi.clearAllMocks()
})

async function open() {
  const result = render(() => (
    <MemoryRouter
      root={(props) => (
        <RoomProvider value={localRoom()}>{props.children}</RoomProvider>
      )}
    >
      <Route path='/' component={Room} />
    </MemoryRouter>
  ))
  await settle()
  return result
}

describe('the room with no server behind it', () => {
  test('is read from its own branch of the mirror', async () => {
    const { container, getByText } = await open()
    expect(getByText('Skirmish')).toBeTruthy()
    expect(container.querySelectorAll('.team').length).toBe(2)
    expect(
      [...container.querySelectorAll('.pname')].map((n) => n.textContent),
    ).toEqual(['me', 'BARb'])
  })

  test('a setting change becomes one act, not a chat command', async () => {
    const { container } = await open()
    const box = container.querySelector(
      '.opt input[type=checkbox]',
    ) as HTMLInputElement
    fireEvent.change(box, { target: { checked: true } })
    await settle()

    expect(sent('skirmish_act')).toEqual([
      { act: { type: 'setOption', key: 'ranked_game', value: '1' } },
    ])
    // Never the online command, whose value goes out as `!bSet` over chat.
    expect(sent('set_option')).toEqual([])
    expect(sent('say_battle')).toEqual([])
  })

  test('Start launches this room rather than joining anyone', async () => {
    const { container } = await open()
    fireEvent.click(container.querySelector('.card-actions button')!)
    await settle()
    expect(sent('skirmish_launch')).toHaveLength(1)
    expect(sent('launch')).toEqual([])
  })

  test('the composer is a console, and its line goes to the room', async () => {
    const { container } = await open()
    const input = container.querySelector(
      '.room-chat textarea, .room-chat input',
    ) as HTMLElement
    expect(input.getAttribute('placeholder')).toContain('!command')
  })

  test('everything the seam offers maps onto a command', async () => {
    // `satisfies RoomIo` in `local.ts` already proves the shapes line up; this
    // proves each one reaches Rust rather than quietly resolving.
    await skirmishIo.takeSeat(1, 1)
    await skirmishIo.releaseSeat()
    await skirmishIo.setSide(3)
    await skirmishIo.removeBot('BARb')
    await skirmishIo.renameRoom('Tuesday')
    await skirmishIo.setMap('Supreme Isthmus v2.1')
    await skirmishIo.sayBattle('!help')

    expect(
      sent('skirmish_act').map(
        (args) => (args as { act: { type: string } }).act,
      ),
    ).toEqual([
      { type: 'takeSeat', team: 1, allyTeam: 1 },
      { type: 'releaseSeat' },
      { type: 'setSide', side: 3 },
      { type: 'removeBot', name: 'BARb' },
      { type: 'setTitle', title: 'Tuesday' },
      { type: 'setMap', map: 'Supreme Isthmus v2.1' },
      { type: 'say', text: '!help' },
    ])
  })

  test("an AI's own options are read and set as the room's are", async () => {
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === 'ai_options')
        return [
          {
            key: 'cheating',
            name: 'LOS vision',
            desc: '',
            type: 'bool',
            def: false,
          },
        ]
      if (command === 'game_modoptions') return OPTIONS
      return null
    })
    const { container } = await open()

    // The pen on the AI's row, which only a room that can tell it anything
    // draws at all.
    const pen = container.querySelector('.team .bot-remove') as HTMLElement
    fireEvent.click(pen)
    await settle()

    expect(sent('ai_options')).toEqual([{ engine: '2026.07.04', ai: 'BARb' }])
    const box = container.querySelector(
      '.bot-options input[type=checkbox]',
    ) as HTMLInputElement
    expect(box).toBeTruthy()
    fireEvent.change(box, { target: { checked: true } })
    await settle()
    expect(
      sent('skirmish_act').map((args) => (args as { act: unknown }).act),
    ).toContainEqual({
      type: 'setBotOption',
      name: 'BARb',
      key: 'cheating',
      value: '1',
    })
  })

  test('what needs somebody else is not drawn', async () => {
    const { container, queryByText } = await open()
    expect(queryByText('Balance')).toBeNull()
    expect(queryByText('Host a public room')).toBeNull()
    expect(queryByText('Leave')).toBeNull()
    // Nobody to host it for, and nobody to read its name.
    expect(container.textContent).not.toContain('Host')
    expect(container.querySelector('.room-title button')).toBeNull()
    // A preset, though, this room can carry.
    expect(localRoom().presets()).not.toBeNull()
  })
})
