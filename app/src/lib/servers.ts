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
 * Why a host cannot be added, or `null` when it can. Just the name: ports
 * have their own field, and a scheme or a path is not somewhere a lobby
 * connects to.
 */
export function hostProblem(
  host: string,
  listed: readonly string[],
): string | null {
  const id = serverId(host)
  if (!id) return 'a host is needed'
  if (/[\s/:@]/.test(id))
    return 'just the host name — ports have their own field'
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

/** A server just added by its host: teiserver's ports, encrypted only, no account yet. */
export function newServer(host: string): ServerEntry {
  return {
    host: host.trim(),
    name: '',
    ports: [8200, 8201],
    allowUnencrypted: false,
    website: null,
    username: '',
    channels: ['main'],
  }
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
