import { fireEvent, render } from '@solidjs/testing-library'
import { beforeEach, describe, expect, test, vi } from 'vitest'
import type { Settings } from '../ipc/bindings/Settings'
import { applySettings } from '../store/settings'
import { SettingsView } from './Settings'

// The view talks to Rust when it saves. Nothing here waits for that, but the
// call must not be a missing import.
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => {
    throw new Error('not in this test')
  }),
}))

function loaded(): Settings {
  return {
    $schema: null,
    server: { host: 'server4', port: 8201, tls: true },
    account: { username: 'me', rememberPassword: false, autoLogin: false },
    paths: { dataDir: null },
    play: {
      inPublicRooms: true,
      joinAs: 'remember',
      lastWasPlayer: true,
      autoLaunch: true,
      pveStats: true,
    },
    notifications: {
      privateMessage: 'desktop',
      mention: 'desktop',
      ring: 'desktop',
      friendOnline: 'lobby',
      vote: 'lobby',
      gameStarting: 'desktop',
      gameEnded: 'lobby',
    },
    battleList: {
      showPassworded: true,
      showLocked: true,
      showRunning: true,
      showEmpty: false,
      friendsOnly: false,
      mode: 'all',
      sort: 'relevance',
      sortDescending: false,
    },
    chat: { filterHostChatter: true, maxLines: 3000, channels: ['main'] },
    overlay: {
      enabled: true,
      hotkey: 'Alt+Shift+L',
      returnFocusToGame: true,
      inGameEscape: true,
    },
    tweaks: { styluaConfig: null, defaultSlot: 'tweakdefs1' },
    logging: { filter: 'info' },
    updates: { automatic: true },
  }
}

/** The three buttons of one notification row, by the row's label. */
function choiceFor(container: HTMLElement, label: string) {
  const row = [...container.querySelectorAll('.choice-row')].find((element) =>
    element.textContent?.startsWith(label),
  )
  if (!row) throw new Error(`no row for ${label}`)
  const buttons = [...row.querySelectorAll('button')]
  return {
    click: (name: string) => {
      const button = buttons.find((b) => b.textContent?.trim() === name)
      if (!button) throw new Error(`no ${name} button`)
      fireEvent.click(button)
    },
    lit: () =>
      buttons
        .filter((button) => button.classList.contains('on'))
        .map((button) => button.textContent?.trim()),
  }
}

describe('choosing where a notification goes', () => {
  beforeEach(() => applySettings(loaded()))

  test('exactly one of the three is ever chosen', () => {
    const { container } = render(() => <SettingsView />)
    const mention = choiceFor(container, 'Someone says my name')

    expect(mention.lit()).toEqual(['Desktop'])

    mention.click('In lobby')
    expect(mention.lit()).toEqual(['In lobby'])

    mention.click('Desktop')
    expect(mention.lit()).toEqual(['Desktop'])

    mention.click('Off')
    expect(mention.lit()).toEqual(['Off'])
  })

  test('a row is chosen on its own, leaving its neighbours alone', () => {
    const { container } = render(() => <SettingsView />)
    const mention = choiceFor(container, 'Someone says my name')
    const ring = choiceFor(container, 'Someone rings me')

    mention.click('In lobby')
    expect(ring.lit()).toEqual(['Desktop'])
  })
})
