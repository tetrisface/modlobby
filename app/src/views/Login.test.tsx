import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { Settings } from '../ipc/bindings/Settings'
import { newServer } from '../lib/servers'
import { setSettingsSignal } from '../store/settings'
import { Login } from './Login'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@solidjs/router', () => ({ useNavigate: () => vi.fn() }))
const asked = vi.mocked(invoke)

function withServers(...hosts: string[]): Settings {
  return {
    servers: hosts.map((host) => newServer(host)),
    account: { rememberPassword: false, autoLogin: false },
  } as unknown as Settings
}

const serverLine = (root: HTMLElement) =>
  [...root.querySelectorAll('p.muted')].find((p) =>
    p.textContent?.startsWith('Server:'),
  )?.textContent

beforeEach(() => {
  asked.mockReset()
  asked.mockImplementation(async (command: string) =>
    command === 'login_wait' ? 0 : null,
  )
})

afterEach(() => {
  cleanup()
  setSettingsSignal(null)
})

describe('the login page', () => {
  test('with one server there is nothing to choose', () => {
    setSettingsSignal(withServers('server4.beyondallreason.info'))
    const { container } = render(() => <Login />)
    expect(container.querySelector('select')).toBeNull()
    expect(serverLine(container)).toContain('server4.beyondallreason.info')
  })

  test('with more, the form is for the one picked', () => {
    setSettingsSignal(
      withServers('server4.beyondallreason.info', 'randomguyrapid.duckdns.org'),
    )
    const { container } = render(() => <Login />)
    const choice = container.querySelector('select')!
    expect(choice.options).toHaveLength(2)
    expect(serverLine(container)).toContain('server4.beyondallreason.info')

    fireEvent.change(choice, {
      target: { value: 'randomguyrapid.duckdns.org' },
    })
    expect(serverLine(container)).toContain('randomguyrapid.duckdns.org')
  })

  test('with none, says where one is added', () => {
    setSettingsSignal(withServers())
    const { container } = render(() => <Login />)
    expect(container.textContent).toContain('Add one in Settings')
  })
})
