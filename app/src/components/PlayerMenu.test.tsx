import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { afterEach, describe, expect, test, vi } from 'vitest'
import type { BotView } from '../ipc/bindings/BotView'
import {
  PlayerMenu,
  showBotMenu,
  showPlayerMenu,
  type Moves,
} from './PlayerMenu'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
  convertFileSrc: (path: string) => path,
}))

afterEach(cleanup)

const BOT: BotView = {
  name: 'BARb(1)',
  owner: 'me',
  status: {
    ready: true,
    team: 1,
    allyTeam: 1,
    player: true,
    handicap: 25,
    sync: 'bot',
    side: 0,
  },
  teamColour: 0,
  ai: 'BARb',
  options: {},
}

/** A pointer event as the rows hand it over. */
function press(): MouseEvent {
  return new MouseEvent('contextmenu', { clientX: 10, clientY: 20 })
}

function moves(given: number[]): Moves {
  return {
    teams: [0, 1],
    on: 1,
    to: () => Promise.resolve(),
    bonus: (percent) => {
      given.push(percent)
      return Promise.resolve()
    },
    bonusNow: 25,
  }
}

/** A real click: the mouse goes down before it comes up. */
function click(element: Element) {
  fireEvent.mouseDown(element)
  fireEvent.click(element)
}

async function settle() {
  for (let turn = 0; turn < 4; turn++) await Promise.resolve()
}

const labels = (container: HTMLElement) =>
  [...container.querySelectorAll('.player-menu button')].map(
    (b) => b.textContent,
  )

describe('the bonus, from the row menu', () => {
  test('an AI of ours offers it, and the panel sets it', async () => {
    const given: number[] = []
    const { container } = render(() => <PlayerMenu />)
    showBotMenu(BOT, () => Promise.resolve(), press(), moves(given))
    await settle()
    expect(labels(container)).toEqual(['Move to team 1', 'Bonus', 'Remove'])

    click(
      [...container.querySelectorAll('button')].find(
        (b) => b.textContent === 'Bonus',
      ) as HTMLElement,
    )
    await settle()
    const panel = container.querySelector('.bonus-pick')
    expect(panel, 'the panel opens in the menu').toBeTruthy()
    const number = panel?.querySelector(
      'input[type=number]',
    ) as HTMLInputElement
    expect(number.value).toBe('25')

    fireEvent.input(number, { target: { value: '40' } })
    fireEvent.submit(panel as HTMLFormElement)
    await settle()
    expect(given).toEqual([40])
    expect(container.querySelector('.player-menu')).toBeNull()
  })

  test('a person offers it too, where the room lets us', async () => {
    const given: number[] = []
    const { container } = render(() => <PlayerMenu />)
    showPlayerMenu('alice', press(), moves(given))
    await settle()
    expect(labels(container)).toContain('Bonus')
  })
})
