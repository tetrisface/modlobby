import { For, Show, createEffect, createSignal } from 'solid-js'
import type { ServerEntry } from '../ipc/bindings/ServerEntry'
import { api, describeError, errorCode } from '../ipc/client'
import { serverId, serverName, sessionStatus } from '../lib/servers'
import { pushNotice } from '../store/chat'
import { lobby } from '../store/lobby'
import { settings } from '../store/settings'
import { dismiss } from './dismiss'
import { LoginSheet } from './LoginForm'

const disconnected = (entry: ServerEntry) =>
	(lobby.servers[serverId(entry.host)]?.phase ?? null) === null

/**
 * Who you are, in the corner, and behind it every server: how each one
 * stands, and a way back into one that dropped without waiting out its timer.
 */
export function AccountMenu(props: { name: string }) {
	const [open, setOpen] = createSignal(false)
	/** The server the login form is open on, for one with nothing to reconnect with. */
	const [asking, setAsking] = createSignal<string | null>(null)
	let root: HTMLDivElement | undefined
	createEffect(() => {
		if (open())
			dismiss(
				() => root,
				() => setOpen(false),
			)
	})

	const listed = () => settings()?.servers ?? []
	/**
	 * A server that dropped and is being tried again: the one kind of "not
	 * connected" worth a mark on the name. One logged out of on purpose, or
	 * never logged in to this run, is where it was asked to be.
	 */
	const dropped = () =>
		Object.values(lobby.servers).some(
			(session) => session.phase === null && session.retryAt !== null,
		)

	function logIn(server: string) {
		setOpen(false)
		setAsking(server)
	}

	async function logOut(server: string) {
		try {
			await api.logout(server)
		} catch (error) {
			pushNotice('warning', `log out: ${describeError(error)}`)
		}
	}

	async function reconnect(server: string) {
		try {
			await api.reconnect(server)
		} catch (error) {
			if (errorCode(error) === 'noCredentials') logIn(server)
			else pushNotice('warning', `reconnect: ${describeError(error)}`)
		}
	}

	return (
		<div class='account' ref={root}>
			<button
				type='button'
				class='account-name'
				classList={{ partial: dropped() }}
				aria-expanded={open()}
				title={dropped() ? 'A server dropped; trying it again' : 'Your servers'}
				onClick={() => setOpen(!open())}
			>
				{props.name}
			</button>
			<Show when={open()}>
				<div class='popover account-menu'>
					<For each={listed()}>
						{(entry) => {
							const id = () => serverId(entry.host)
							const status = () => sessionStatus(lobby.servers[id()])
							const way = () => lobby.ways[id()]
							return (
								<div class='account-server'>
									<span class='account-server-name'>{serverName(entry)}</span>
									<span
										class={`chip ${status().tone}`}
										title={way() ? `Last way in: ${way()}` : ''}
									>
										{status().text}
									</span>
									<Show
										when={disconnected(entry)}
										fallback={
											<button type='button' onClick={() => void logOut(id())}>
												Log out
											</button>
										}
									>
										<button
											type='button'
											onClick={() =>
												entry.username ? void reconnect(id()) : logIn(id())
											}
										>
											{entry.username ? 'Reconnect' : 'Log in'}
										</button>
									</Show>
								</div>
							)
						}}
					</For>
				</div>
			</Show>
			<Show when={asking()} keyed>
				{(server) => (
					<LoginSheet server={server} close={() => setAsking(null)} />
				)}
			</Show>
		</div>
	)
}
