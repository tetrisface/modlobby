import { createSignal } from 'solid-js'
import type { Account } from '../ipc/bindings/Account'
import { api, describeError } from '../ipc/client'
import { pushNotice } from './chat'

/**
 * When the held auto-login goes out, as a `Date.now()` moment, while it is
 * held. The corner counts down to it; nothing else is logged in yet to say so.
 */
export const [loginHold, setLoginHold] = createSignal<number | null>(null)

/**
 * Logging back in with the password the keyring remembers.
 *
 * This lives here rather than in the login form because the form is no longer
 * on the way in: the app opens on Skirmish, and an auto-login that only
 * happens when somebody visits the login page is not an auto-login.
 *
 * Attempted once per run rather than once per mount. The flag is module-level
 * so that a reload during development, or a component that comes and goes,
 * never logs someone back in immediately after they logged out.
 */
let attempted = false

export async function autoLogin(account: Account): Promise<void> {
  if (attempted) return
  // Without a remembered password there is nothing to log in with, and
  // `autoLogin` is only ever saved alongside `rememberPassword`.
  if (!account.autoLogin || !account.rememberPassword) return
  const username = account.username.trim()
  if (!username) return
  attempted = true

  // teiserver refuses a login within twenty seconds of the account's last,
  // and Rust keeps that clock across restarts — so the start after an update
  // or a rebuild arrives here already throttled. Waiting it out beats being
  // refused: a refusal starts the twenty seconds again.
  const held = await api.loginWait().catch(() => 0)
  if (held > 0) {
    setLoginHold(Date.now() + (held + 1) * 1000)
    await pause((held + 1) * 1000)
    setLoginHold(null)
  }

  try {
    // No password given: Rust falls back to the one in the keyring.
    await api.login(username, null, true, true)
  } catch (error) {
    pushNotice('warning', `could not log in: ${describeError(error)}`)
  }
}

function pause(ms: number): Promise<void> {
  return new Promise((wake) => setTimeout(wake, ms))
}
