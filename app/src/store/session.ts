import { createSignal } from 'solid-js'
import type { Settings } from '../ipc/bindings/Settings'
import { api, describeError } from '../ipc/client'
import { serverId } from '../lib/servers'
import { pushNotice } from './chat'
import { applySettings } from './settings'

/**
 * When each held auto-login goes out, as a `Date.now()` moment, by server,
 * while it is held. Nothing is logged in yet to say so otherwise.
 */
const [holds, setHolds] = createSignal<Record<string, number>>({})

/** The soonest held auto-login; the corner counts down to it. */
export function loginHold(): number | null {
  const moments = Object.values(holds())
  return moments.length === 0 ? null : Math.min(...moments)
}

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

export async function autoLogin(settings: Settings): Promise<void> {
  if (attempted) return
  // Without a remembered password there is nothing to log in with, and
  // `autoLogin` is only ever saved alongside `rememberPassword`.
  const { account } = settings
  if (!account.autoLogin || !account.rememberPassword) return
  attempted = true

  // Every server with an account and a remembered password, side by side:
  // each waits out its own login limit, and one being slow holds up none.
  await Promise.all(
    settings.servers.map(async (entry) => {
      const username = entry.username.trim()
      const server = serverId(entry.host)
      if (!username) return
      const stored = await api.hasPassword(server, username).catch(() => false)
      if (stored) await loginTo(server, username)
    }),
  )
}

async function loginTo(server: string, username: string): Promise<void> {
  // teiserver refuses a login within twenty seconds of the account's last,
  // and Rust keeps that clock across restarts — so the start after an update
  // or a rebuild arrives here already throttled. Waiting it out beats being
  // refused: a refusal starts the twenty seconds again.
  const held = await api.loginWait(server).catch(() => 0)
  if (held > 0) {
    setHolds((all) => ({ ...all, [server]: Date.now() + (held + 1) * 1000 }))
    await pause((held + 1) * 1000)
    setHolds(({ [server]: _, ...rest }) => rest)
  }

  try {
    // No password given: Rust falls back to the one in the keyring.
    applySettings(await api.login(server, username, null, true, true))
  } catch (error) {
    pushNotice('warning', `could not log in: ${describeError(error)}`)
  }
}

function pause(ms: number): Promise<void> {
  return new Promise((wake) => setTimeout(wake, ms))
}
