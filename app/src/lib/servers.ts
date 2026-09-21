import type { Account } from '../ipc/bindings/Account'
import type { Phase } from '../ipc/bindings/Phase'
import type { ServerEntry } from '../ipc/bindings/ServerEntry'

/**
 * How a server is known across the app — the store, the remembered ways, a
 * chat key — however its host was typed: the runtime's `server_id`.
 */
export function serverId(host: string): string {
	return host.trim().toLowerCase()
}

/** BAR's own server, as `serverId` names it. */
export const BAR_HOST = 'server4.beyondallreason.info'

/** What a server is called: its name, else its host. */
export function serverName(entry: ServerEntry): string {
	return entry.name.trim() || entry.host.trim()
}

/**
 * Where a server's forgotten passwords are reset: teiserver's own page, on
 * the server's website — `https://<host>` unless the entry says otherwise.
 * Asked of the server itself, teiserver names a page that is gone
 * (`/password_reset`), and a server set up as `localhost` names that.
 */
export function forgotPasswordUrl(entry: ServerEntry): string {
	const site = entry.website?.trim() || `https://${entry.host.trim()}`
	return `${site.replace(/\/+$/, '')}/forgot_password`
}

/**
 * A host as typed, and the port after it if one was: `host:4000` is how a
 * server's address is usually written down.
 */
export function splitHost(typed: string): {
	host: string
	port: number | null
} {
	const found = /^(.+):(\d+)$/.exec(typed.trim())
	if (!found) return { host: typed.trim(), port: null }
	return { host: found[1]!, port: Number(found[2]) }
}

/**
 * Why a host cannot be added, or `null` when it can. The name, and a port
 * after it at most: a scheme or a path is not somewhere a lobby connects to.
 */
export function hostProblem(
	typed: string,
	listed: readonly string[],
): string | null {
	const { host, port } = splitHost(typed)
	const id = serverId(host)
	if (!id) return 'a host is needed'
	if (port !== null && (port < 1 || port > 65535))
		return `${port} is not a port`
	if (/[\s/:@]/.test(id))
		return 'just the host name, and a port after a colon if it needs one'
	if (listed.some((held) => serverId(held) === id))
		return 'that server is already listed'
	return null
}

/** Ports as typed — `8200, 8201` — or `null` while that is not a list of ports. */
export function parsePorts(text: string): number[] | null {
	const words = text.split(/[\s,]+/).filter(Boolean)
	const ports = words.map(Number)
	if (ports.length === 0) return null
	if (
		!ports.every((port) => Number.isInteger(port) && port > 0 && port < 65536)
	)
		return null
	return [...new Set(ports)]
}

/**
 * Where a server's rapid master index is, if it keeps one where most do:
 * beside the lobby, under its own name. A guess to be checked, not a fact.
 */
export function guessedRapid(host: string): string {
	return `https://${host.trim()}/repos.gz`
}

/**
 * Servers worth offering by name when one is added, with what is known of
 * them already.
 *
 * Recoil's lobby is uberserver: STLS on 8200 only (8201 is its UDP port, and
 * a TCP try there just waits out its timeout), behind the self-signed X.509
 * v1 certificate uberserver makes itself, which rustls cannot read — so no
 * encrypted way in until certificates are pinned, and unencrypted is allowed.
 * It keeps no rapid index of its own; its games are on springrts' master.
 */
export const KNOWN: Pick<
	ServerEntry,
	'host' | 'name' | 'ports' | 'allowUnencrypted' | 'rapid'
>[] = [
	{
		host: 'lobby.recoilengine.org',
		name: 'Recoil Official',
		ports: [8200],
		allowUnencrypted: true,
		rapid: 'https://repos.springrts.com/repos.gz',
	},
]

/**
 * A server just added by its host: teiserver's ports unless one was typed
 * after it, encrypted only, no account yet — or what is known of it.
 */
export function newServer(typed: string): ServerEntry {
	const { host, port } = splitHost(typed)
	const known = KNOWN.find((entry) => serverId(entry.host) === serverId(host))
	return {
		host,
		name: known?.name ?? '',
		ports: port === null ? (known?.ports ?? [8200, 8201]) : [port],
		allowUnencrypted: known?.allowUnencrypted ?? false,
		website: null,
		rapid: known?.rapid ?? null,
		maps: null,
		username: '',
		channels: ['main'],
		autoLogin: null,
	}
}

/**
 * Whether `entry` is logged in to at startup: its own answer, else the
 * account's — and never without a remembered password to do it with.
 */
export function logsInAtStart(entry: ServerEntry, account: Account): boolean {
	return account.rememberPassword && (entry.autoLogin ?? account.autoLogin)
}

/** What server `id` is called among `listed`: its entry's name, else the id. */
export function labelOf(listed: readonly ServerEntry[], id: string): string {
	const entry = listed.find((held) => serverId(held.host) === id)
	return entry ? serverName(entry) : id
}

/**
 * The names among `named` that more than one server has: the only ones a
 * server tag is needed to tell apart. With one server, or names all their
 * own, nothing is tagged, and the lobby reads as it did before servers.
 */
export function clashes(
	named: readonly { server: string | null; name: string }[],
): Set<string> {
	const first = new Map<string, string | null>()
	const clashing = new Set<string>()
	for (const { server, name } of named) {
		if (!first.has(name)) first.set(name, server)
		else if (first.get(name) !== server) clashing.add(name)
	}
	return clashing
}

/** Where a server's session stands, in a few words and a chip's colour. */
export function sessionStatus(
	session:
		| { phase: Phase | null; me: string | null; retryAt: number | null }
		| undefined,
): {
	text: string
	tone: '' | 'ok' | 'info' | 'warn'
} {
	if (session?.phase === 'ready')
		return { text: `logged in as ${session.me ?? '?'}`, tone: 'ok' }
	if (session?.phase) return { text: 'connecting…', tone: 'info' }
	if (session?.retryAt) return { text: 'trying again soon', tone: 'warn' }
	return { text: 'not connected', tone: '' }
}
