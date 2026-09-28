import { useNavigate, useSearchParams } from '@solidjs/router'
import { For, Show, createSignal } from 'solid-js'
import { LoginForm } from '../components/LoginForm'
import { Select } from '../components/Select'
import type { ServerEntry } from '../ipc/bindings/ServerEntry'
import { isLan } from '../lan/lan'
import { serverId, serverName } from '../lib/servers'
import { mainServer } from '../store/lobby'
import { unattended } from '../store/session'
import { settings } from '../store/settings'

/** How many servers get a chip of their own; the rest fold into a dropdown. */
const CHIPS = 10

/** What a chip for a server that goes in beside the picked one says. */
const BESIDE = 'Logs in too, with the password this machine remembers'

/**
 * The way in while there is no session: the login form, for whichever server
 * is picked: the one a link asked for, else the one the lobby already means,
 * else the first listed. Every other server set to log in at startup goes
 * in beside it, and its chip says so.
 */
export function Login() {
	const navigate = useNavigate()
	// The LAN has no accounts; it is joined from the battle list.
	const servers = () => (settings()?.servers ?? []).filter((e) => !isLan(e))
	const [picked, setPicked] = createSignal<string | null>(null)
	/** The server a link came for: `/login?server=…`. */
	const [params] = useSearchParams()
	const asked = () =>
		(Array.isArray(params.server) ? params.server[0] : params.server) ?? null
	const server = () =>
		picked() ??
		asked() ??
		mainServer() ??
		(servers()[0] ? serverId(servers()[0]!.host) : null)
	/** The servers that go in beside the picked one when Log in is pressed. */
	const beside = (): ServerEntry[] => {
		const saved = settings()
		if (!saved) return []
		return unattended(saved).filter(
			(entry) => serverId(entry.host) !== server(),
		)
	}
	const goesIn = (entry: ServerEntry) => beside().includes(entry)
	const chipped = () => servers().slice(0, CHIPS)
	const folded = () => servers().slice(CHIPS)
	/** The dropdown's value: the picked server when it is among the folded. */
	const foldedPick = () =>
		folded().some((entry) => serverId(entry.host) === server()) ? server()! : ''

	return (
		<div class='login-page'>
			<Show when={servers().length > 1}>
				<div class='chips login-servers' role='group' aria-label='Server'>
					<For each={chipped()}>
						{(entry) => (
							<button
								type='button'
								class='chip-choice'
								classList={{
									on: server() === serverId(entry.host),
									also: goesIn(entry),
								}}
								aria-pressed={server() === serverId(entry.host)}
								title={goesIn(entry) ? BESIDE : undefined}
								onClick={() => setPicked(serverId(entry.host))}
							>
								{serverName(entry)}
							</button>
						)}
					</For>
					<Show when={folded().length > 0}>
						<Select
							value={foldedPick()}
							onChange={(event) => setPicked(event.currentTarget.value)}
						>
							<option value='' disabled>
								more…
							</option>
							<For each={folded()}>
								{(entry) => (
									<option
										value={serverId(entry.host)}
										title={goesIn(entry) ? BESIDE : undefined}
									>
										{goesIn(entry) ? '✓ ' : ''}
										{serverName(entry)}
									</option>
								)}
							</For>
						</Select>
					</Show>
				</div>
			</Show>
			<Show
				when={server()}
				keyed
				fallback={
					<p class='muted'>
						No server is set up. Add one in Settings, under Servers.
					</p>
				}
			>
				{(id) => (
					<LoginForm
						server={id}
						asksFlags
						beside={beside()}
						onDone={() => navigate('/battles', { replace: true })}
					/>
				)}
			</Show>
		</div>
	)
}
