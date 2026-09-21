import { createSignal } from 'solid-js'
import type { Settings } from '../ipc/bindings/Settings'
import { api, describeError } from '../ipc/client'
import { logsInAtStart, serverId } from '../lib/servers'
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
	const due = settings.servers.filter(
		(entry) => entry.username.trim() && logsInAtStart(entry, settings.account),
	)
	if (due.length === 0) return
	attempted = true

	// Every server due, with a remembered password, side by side: each waits
	// out its own login limit, and one being slow holds up none.
	await Promise.all(
		due.map(async (entry) => {
			const username = entry.username.trim()
			const server = serverId(entry.host)
			const stored = await api.hasPassword(server, username).catch(() => false)
			if (stored) await loginTo(server, username, settings.account.autoLogin)
		}),
	)
}

async function loginTo(
	server: string,
	username: string,
	autoLogin: boolean,
): Promise<void> {
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
		// No password given: Rust falls back to the one in the keyring. The
		// account's own answer goes back as it was, since a login writes it: a
		// server that logs in at startup on its own say turns it on for no other.
		applySettings(await api.login(server, username, null, true, autoLogin))
	} catch (error) {
		pushNotice('warning', `could not log in: ${describeError(error)}`)
	}
}

function pause(ms: number): Promise<void> {
	return new Promise((wake) => setTimeout(wake, ms))
}
