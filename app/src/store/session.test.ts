import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { Account } from '../ipc/bindings/Account'

const { login, loginWait } = vi.hoisted(() => ({
  login: vi.fn(),
  loginWait: vi.fn(),
}))
vi.mock('../ipc/client', () => ({
  api: {
    login: (...args: unknown[]) => login(...args),
    loginWait: () => loginWait(),
  },
  describeError: (error: unknown) => String(error),
}))

/** The once-per-run flag is module state; each test wants a run of its own. */
async function fresh() {
  vi.resetModules()
  return import('./session')
}

/** Lets an awaited answer reach the code under test. */
async function settle() {
  for (let turn = 0; turn < 8; turn++) await Promise.resolve()
}

const remembered: Account = {
  username: 'me',
  rememberPassword: true,
  autoLogin: true,
}

beforeEach(() => {
  login.mockReset()
  login.mockResolvedValue(undefined)
  loginWait.mockReset()
  loginWait.mockResolvedValue(0)
})

afterEach(() => vi.useRealTimers())

describe('logging back in on startup', () => {
  test('logs in once per run, not once per mount', async () => {
    const session = await fresh()

    await session.autoLogin(remembered)
    await session.autoLogin(remembered)

    expect(login).toHaveBeenCalledTimes(1)
    // No password given: the keyring holds it and Rust is what reads it.
    expect(login).toHaveBeenCalledWith('me', null, true, true)
  })

  test('does nothing without a remembered password', async () => {
    const session = await fresh()

    await session.autoLogin({ ...remembered, rememberPassword: false })
    await session.autoLogin({ ...remembered, autoLogin: false })
    await session.autoLogin({ ...remembered, username: '   ' })

    expect(login).not.toHaveBeenCalled()
  })

  test('waits out the throttle instead of failing the login', async () => {
    vi.useFakeTimers()
    loginWait.mockResolvedValue(20)
    const session = await fresh()

    const done = session.autoLogin(remembered)
    await settle()
    await vi.advanceTimersByTimeAsync(20_000)
    expect(login).not.toHaveBeenCalled()

    // A second past the server's own count, so the allowance has lapsed.
    await vi.advanceTimersByTimeAsync(1_000)
    await done

    expect(login).toHaveBeenCalledTimes(1)
  })

  test('a refused login is said once and not tried again', async () => {
    login.mockRejectedValue({ code: 'refused', message: 'no' })
    const session = await fresh()

    await session.autoLogin(remembered)
    await session.autoLogin(remembered)

    expect(login).toHaveBeenCalledTimes(1)
  })
})
