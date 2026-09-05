import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { reconcile } from 'solid-js/store'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { Book } from '../ipc/bindings/Book'
import type { Preset } from '../ipc/bindings/Preset'
import { emptyLobby, setLobby } from '../store/lobby'
import { Presets } from './Presets'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

/** Lets an awaited answer reach the component and the DOM. */
async function settle() {
  for (let turn = 0; turn < 8; turn++) await Promise.resolve()
}

function preset(name: string, over: Partial<Preset> = {}): Preset {
  return {
    name,
    map: 'Comet Catcher Remake 1.8',
    modoptions: { tweakdefs1: 'abc', ranked_game: '0' },
    battle: {},
    startBoxes: {},
    bots: {},
    created: 1_700_000_000,
    updated: 1_700_000_000,
    lastUsed: null,
    ...over,
  }
}

const BOOK: Book = { version: 1, presets: [preset('raptors'), preset('ffa')] }

const NO_PLAN = {
  lines: [],
  alreadySet: 0,
  startBoxes: [],
  startBoxesUnsent: false,
}

/** Answers each command the pane asks for, from a book that can be swapped. */
function serve(book: Book) {
  vi.mocked(invoke).mockImplementation(async (command: string) => {
    switch (command) {
      case 'list_presets':
        return book
      case 'chobby_presets_path':
        return null
      case 'apply_preset':
        return NO_PLAN
      case 'delete_preset':
      case 'rename_preset':
        return book
      default:
        throw new Error(`unexpected ${command}`)
    }
  })
}

function calls(command: string) {
  return vi
    .mocked(invoke)
    .mock.calls.filter(([name]) => name === command)
    .map(([, args]) => args)
}

/** Puts us in room 7, which is what Save and Load need. */
function enterRoom() {
  const next = emptyLobby()
  next.me = 'me'
  next.battles[7] = {
    id: 7,
    founder: 'host',
    ip: '',
    port: 0,
    maxPlayers: 16,
    passworded: false,
    locked: false,
    mapHash: '',
    mapName: 'Comet Catcher Remake 1.8',
    engineName: 'spring',
    engineVersion: '',
    title: 'raptors',
    gameName: 'BAR',
    members: ['host', 'me'],
    spectatorCount: 0,
    playerCount: 1,
    layout: null,
    bots: [],
    startRects: [],
  }
  next.myBattle = {
    boss: null,
    id: 7,
    gameHash: '',
    scriptTags: {},
    vote: null,
    history: [],
  }
  setLobby(reconcile(next))
}

beforeEach(() => {
  vi.mocked(invoke).mockReset()
  serve(BOOK)
  enterRoom()
})
afterEach(() => {
  cleanup()
  setLobby(reconcile(emptyLobby()))
})

describe('Presets', () => {
  test('without a room, Save and Load wait and the file actions do not', async () => {
    setLobby(reconcile(emptyLobby()))
    const { getByText } = render(() => <Presets />)
    await settle()
    const save = getByText('Save') as HTMLButtonElement
    const load = getByText('Load') as HTMLButtonElement
    expect(save.disabled).toBe(true)
    expect(save.title).toBe('Join a room first')
    fireEvent.click(getByText('raptors'))
    expect(load.disabled).toBe(true)
    expect((getByText('Export to Chobby') as HTMLButtonElement).disabled).toBe(
      false,
    )
    expect(
      (getByText('Import from Chobby') as HTMLButtonElement).disabled,
    ).toBe(false)
  })

  test('the toolbar reads Save, Load, Import from Chobby, Export to Chobby', async () => {
    const { container } = render(() => <Presets />)
    await settle()
    const labels = [...container.querySelectorAll('.toolbar button')].map(
      (button) => button.textContent,
    )
    expect(labels).toEqual([
      'Save',
      'Load',
      'Import from Chobby',
      'Export to Chobby',
    ])
  })

  test('Load and Export wait for a row to be chosen', async () => {
    const { getByText, container } = render(() => <Presets />)
    await settle()
    const load = getByText('Load') as HTMLButtonElement
    const exportButton = getByText('Export to Chobby') as HTMLButtonElement
    expect(load.disabled).toBe(true)
    expect(exportButton.disabled).toBe(true)

    fireEvent.click(getByText('raptors'))
    expect(load.disabled).toBe(false)
    expect(exportButton.disabled).toBe(false)
    expect(container.querySelector('.preset-row.on')?.textContent).toContain(
      'raptors',
    )
  })

  test('clicking the name only selects; the pen is what renames', async () => {
    const { getByText, getByLabelText, container } = render(() => <Presets />)
    await settle()

    fireEvent.click(getByText('raptors'))
    expect(container.querySelector('.sheet')).toBeNull()

    fireEvent.click(getByLabelText('Rename raptors'))
    expect(container.querySelector('.sheet')).not.toBeNull()
    expect(getByText('Rename preset')).toBeTruthy()
    // The pen did not toggle the selection under it.
    expect(container.querySelector('.preset-row.on')?.textContent).toContain(
      'raptors',
    )
  })

  test('the name column carries no tweak count', async () => {
    const { container } = render(() => <Presets />)
    await settle()
    expect(container.querySelector('.preset-name .chip')).toBeNull()
  })

  test('Reset lobby is off by default, and Load says so', async () => {
    const { getByText } = render(() => <Presets />)
    await settle()
    const reset = getByText('Reset lobby')
    expect(reset.classList.contains('on')).toBe(false)

    fireEvent.click(getByText('raptors'))
    fireEvent.click(getByText('Load'))
    await settle()
    expect(calls('apply_preset')).toEqual([
      {
        name: 'raptors',
        sections: {
          map: true,
          modoptions: true,
          battle: true,
          startBoxes: true,
          bots: false,
          reset: false,
        },
      },
    ])

    fireEvent.click(reset)
    expect(reset.classList.contains('on')).toBe(true)
  })

  test('the bin on a row deletes that preset', async () => {
    const { getByLabelText } = render(() => <Presets />)
    await settle()
    fireEvent.click(getByLabelText('Delete ffa'))
    await settle()
    expect(calls('delete_preset')).toEqual([{ name: 'ffa' }])
  })
})
