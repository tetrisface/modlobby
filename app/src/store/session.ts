import { createSignal } from 'solid-js'
import type { ServerEntry } from '../ipc/bindings/ServerEntry'
import type { Settings } from '../ipc/bindings/Settings'
import { api, describeError } from '../ipc/client'
import { logsInAtStart, serverId } from '../lib/servers'
import { pushNotice } from './chat'
import { lobby } from './lobby'
import { applySettings, serverLabel } from './settings'

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
	if (attempted || unattended(settings).length === 0) return
	attempted = true
	await loginUnattended(settings)
}

/**
 * The servers that log in without being asked: set to log in at startup,
 * with an account to do it as, and without a session yet.
 */
export function unattended(settings: Settings): ServerEntry[] {
	return settings.servers.filter(
		(entry) =>
			entry.username.trim() !== '' &&
			logsInAtStart(entry, settings.account) &&
			!lobby.servers[serverId(entry.host)]?.phase,
	)
}

/**
 * Logs in to every server that logs in without being asked, each with the
 * password the keyring remembers for it: what a startup does, and what the
 * battle list's Log in does. `false` at once when none has a password to go
 * in with, so a press can open the login page instead.
 */
export async function loginUnattended(settings: Settings): Promise<boolean> {
	const kept = await Promise.all(
		unattended(settings).map(async (entry) => {
			const stored = await api
				.hasPassword(serverId(entry.host), entry.username.trim())
				.catch(() => false)
			return stored ? entry : null
		}),
	)
	const going = kept.filter((entry) => entry !== null)
	if (going.length === 0) return false
	// Side by side: each waits out its own login limit, and one being slow
	// holds up none. The account's own answer goes back as it was, since a
	// login writes it: a server that logs in at startup on its own say turns
	// it on for no other.
	await Promise.all(
		going.map((entry) =>
			loginTo(
				serverId(entry.host),
				entry.username.trim(),
				true,
				settings.account.autoLogin,
			),
		),
	)
	return true
}

/**
 * Logs in to `server` as `username` with the password the keyring remembers,
 * waiting out the server's login limit first. What a startup auto-login is,
 * and what the login page does for its other servers beside the one typed
 * into. `remember` and `autoLogin` are written back as the account's
 * answers, so every login of one press should carry the same two.
 */
export async function loginTo(
	server: string,
	username: string,
	remember: boolean,
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
		// No password given: Rust falls back to the one in the keyring.
		applySettings(await api.login(server, username, null, remember, autoLogin))
	} catch (error) {
		pushNotice(
			'warning',
			`could not log in to ${serverLabel(server)}: ${describeError(error)}`,
		)
	}
}

function pause(ms: number): Promise<void> {
	return new Promise((wake) => setTimeout(wake, ms))
}
