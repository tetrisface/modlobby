import { MemoryRouter, Route } from '@solidjs/router'
import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { reconcile } from 'solid-js/store'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { ModOption } from '../../ipc/bindings/ModOption'
import type { SkirmishView } from '../../ipc/bindings/SkirmishView'
import type { VersionView } from '../../ipc/bindings/VersionView'
import { setBuild } from '../../store/build'
import { forgetAskedEngines } from '../../components/GetEngine'
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
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
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

/** What the shell says about this machine. */
const BUILD = (why: string | null = null): VersionView => ({
  version: '0.0.0',
  commit: 'abc1234',
  playsOnline: true,
  noPublishedEngine: why,
})

beforeEach(() => {
  setLobby(reconcile(emptyLobby()))
  // The engine download waits for the shell to say whether this machine has
  // one to fetch. Saying so here is what makes these tests about Windows and
  // Linux rather than about a question nobody answered.
  setBuild(BUILD())
  // A fresh copy each time: the store writes through to whatever object it
  // is given, so a test that empties a field would empty it for the rest.
  setLobby('skirmish', structuredClone(room))
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
  // Module state shared across the file, so the reset is not optional.
  setBuild(null)
  forgetAskedEngines()
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
    const pen = container.querySelector('.team .bot-edit') as HTMLElement
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

/**
 * What the room offers a machine that has nothing on it.
 *
 * The fixture above has all three installed, which is why none of this was
 * covered: every state below is one a first run actually passes through.
 */
describe('a room whose content is not all here', () => {
  test('offers the download once the engine is in place', async () => {
    setLobby('skirmish', 'content', { engine: true, game: false, map: false })
    const { getByText } = await open()
    // Auto-download on, nothing running, engine present: every arm of the
    // switch missed, and the row used to offer nothing at all.
    expect(getByText('Download')).toBeTruthy()
  })

  test('says so rather than asking for the engine called nothing', async () => {
    setLobby('skirmish', 'content', { engine: false, game: false, map: false })
    setLobby('skirmish', 'battle', 'engineVersion', '')
    const { getByText } = await open()
    expect(getByText('This room names no engine to fetch')).toBeTruthy()
    expect(sent('download_engine')).toEqual([])
  })

  test('fetches the engine the room does name', async () => {
    setLobby('skirmish', 'content', { engine: false, game: false, map: false })
    const { container } = await open()
    expect(sent('download_engine')).toEqual([{ version: '2026.07.04' }])
    expect(container.textContent).toContain(
      'Engine 2026.07.04 is not installed.',
    )
  })

  test('Start is refused while anything is missing', async () => {
    setLobby('skirmish', 'content', { engine: true, game: false, map: false })
    const { container } = await open()
    const start = container.querySelector(
      '.card-actions button',
    ) as HTMLButtonElement
    expect(start.textContent).toBe('Start')
    expect(start.disabled).toBe(true)
  })
})

/**
 * What the room offers a machine Beyond All Reason publishes no engine for.
 *
 * The only one that will ever be here is a bundle somebody dropped into the
 * data directory, so every offer to fetch one, choose between them or try
 * again is an offer that ends in a 404. What is left is saying where an engine
 * comes from, and the three steps of doing it.
 */
describe('a room on a machine no engine is published for', () => {
  const WHY =
    'Beyond All Reason publishes no engine for this machine, so modlobby cannot fetch one. An Apple Silicon build goes into the engine folder by hand.'

  beforeEach(() => setBuild(BUILD(WHY)))

  test('says where an engine comes from rather than asking for one', async () => {
    setLobby('skirmish', 'content', { engine: false, game: false, map: false })
    // The real first-run shape: `newest()` ends in `unwrap_or_default()`, so a
    // machine with no engine opens its room with no version either.
    setLobby('skirmish', 'battle', 'engineVersion', '')
    const { container, getByText, queryByText } = await open()

    expect(sent('download_engine')).toEqual([])
    // The older sentence is the wrong answer here: it blames the room for a
    // fact about the machine.
    expect(queryByText('This room names no engine to fetch')).toBeNull()
    expect(getByText('Get one')).toBeTruthy()
    expect(getByText('Engine folder')).toBeTruthy()
    expect(container.textContent).toContain(
      'publishes no engine for this machine',
    )
  })

  test('and no Download either, since pr-downloader is inside the engine', async () => {
    setLobby('skirmish', 'content', { engine: false, game: false, map: false })
    setLobby('skirmish', 'battle', 'engineVersion', '')
    const { queryByText } = await open()
    expect(queryByText('Download')).toBeNull()
  })

  test('names the engine rather than offering a choice of one', async () => {
    const { container } = await open()
    expect(
      container.querySelector('b.chat-link[title="Play a different engine"]'),
    ).toBeNull()
    // The pair is the assertion: the game is still pickable here, because
    // pr-downloader lives inside the hand-installed bundle and fetches games
    // and maps normally. Anything that reached for `picksContent` would take
    // both.
    expect(
      container.querySelector('b.chat-link[title="Play a different game"]'),
    ).toBeTruthy()
  })

  test('a room with nothing installed says none is installed', async () => {
    setLobby('skirmish', 'battle', 'engineVersion', '')
    const { getByText } = await open()
    // Rather than the word "Engine" with a gap after it.
    expect(getByText('none installed')).toBeTruthy()
  })

  test('asks the runtime to look again once something is in the folder', async () => {
    setLobby('skirmish', 'content', { engine: false, game: false, map: false })
    setLobby('skirmish', 'battle', 'engineVersion', '')
    const { getByText } = await open()
    fireEvent.click(getByText('Look again'))
    await settle()
    // Nothing about the room changes when a bundle is dropped in by hand, so
    // being asked is the only way it is found.
    expect(sent('recheck_content').length).toBe(1)
  })
})

describe('what the room says about itself', () => {
  test('counts agree with their words', async () => {
    const { container } = await open()
    // One player, no spectators — not "1 players · 0 spectators".
    expect(container.textContent).toContain('1 player · 0 spectators')
  })

  test('a team nobody has rated carries no sum', async () => {
    const { container } = await open()
    // A skirmish has no skills at all, and Σ 0.0 is not a fact about the team.
    expect(container.querySelector('.team-head .os')).toBeNull()
  })

  test('a room that has not been told what to play still offers the choice', async () => {
    setLobby('skirmish', 'battle', 'mapName', '')
    setLobby('skirmish', 'battle', 'gameName', '')
    const { getAllByText } = await open()
    // An empty link is a click target with no width, which reads as dead text.
    // Map and Game: the engine has a link too, but this room was given a
    // version for it. Suppressing the engine by way of `picksContent` would
    // take these two with it, which is what this count is here to catch.
    expect(getAllByText('choose one').length).toBe(2)
  })

  test('an engine it was never given is an invitation where one can be fetched', async () => {
    setLobby('skirmish', 'battle', 'mapName', '')
    setLobby('skirmish', 'battle', 'gameName', '')
    setLobby('skirmish', 'battle', 'engineVersion', '')
    const { getAllByText } = await open()
    expect(getAllByText('choose one').length).toBe(3)
  })
})
