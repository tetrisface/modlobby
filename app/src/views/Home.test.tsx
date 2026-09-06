import { cleanup, render, waitFor } from '@solidjs/testing-library'
import { reconcile } from 'solid-js/store'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { Settings } from '../ipc/bindings/Settings'
import { emptyLobby, setLobby } from '../store/lobby'
import { setSettingsSignal } from '../store/settings'
import { Home } from './Home'

// Only where it sends you matters, so the redirect is rendered rather than
// performed; a real router here would be scaffolding around one string.
vi.mock('@solidjs/router', () => ({
  Navigate: (props: { href: string }) => <i data-to={props.href} />,
}))

function account(over: Partial<Settings['account']> = {}) {
  setSettingsSignal({
    account: {
      username: 'me',
      rememberPassword: false,
      autoLogin: false,
      ...over,
    },
  } as unknown as Settings)
}

function landsOn(container: HTMLElement): string | null {
  return container.querySelector('i')?.getAttribute('data-to') ?? null
}

beforeEach(() => account())

afterEach(() => {
  cleanup()
  setLobby(reconcile(emptyLobby()))
  setSettingsSignal(null)
})

describe('where a launch lands', () => {
  test('nothing is drawn until the settings say which it is', async () => {
    setSettingsSignal(null)
    const { container } = render(() => <Home />)
    expect(landsOn(container)).toBeNull()

    account({ autoLogin: true, rememberPassword: true })
    await waitFor(() => expect(landsOn(container)).toBe('/battles'))
  })

  test('with no account to log in with, a skirmish', () => {
    const { container } = render(() => <Home />)
    expect(landsOn(container)).toBe('/skirmish')
  })

  test('a launch that will log itself in goes to the lobby', () => {
    account({ autoLogin: true, rememberPassword: true })
    const { container } = render(() => <Home />)
    // Where the offer to rejoin the last room shows up, and it is easy to miss
    // anywhere else.
    expect(landsOn(container)).toBe('/battles')
  })

  test('auto-login with no remembered password cannot log in, so a skirmish', () => {
    account({ autoLogin: true, rememberPassword: false })
    const { container } = render(() => <Home />)
    expect(landsOn(container)).toBe('/skirmish')
  })

  test('already logged in goes to the lobby whatever the settings say', () => {
    setLobby('phase', 'ready')
    const { container } = render(() => <Home />)
    expect(landsOn(container)).toBe('/battles')
  })
})
