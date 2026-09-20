import { useNavigate } from '@solidjs/router'
import { Show, createSignal } from 'solid-js'
import type { ServerEntry } from '../ipc/bindings/ServerEntry'
import { api, describeError } from '../ipc/client'
import { sessionStatus } from '../lib/servers'
import { pushNotice } from '../store/chat'
import { lobby } from '../store/lobby'
import { lanApi } from './api'
import { LAN } from './lan'

/**
 * The LAN's card among the servers: a name to appear as, where things stand,
 * and the two ways in — host a room, or join one by its address when the
 * network did not carry the announcement.
 */
export function LanServerCard(props: {
	entry: ServerEntry
	change: <K extends keyof ServerEntry>(field: K, value: ServerEntry[K]) => void
}) {
	const navigate = useNavigate()
	const session = () => lobby.servers[LAN]
	const status = () => sessionStatus(session())
	const connected = () => (session()?.phase ?? null) !== null
	const [address, setAddress] = createSignal('')
	const [password, setPassword] = createSignal('')
	const [busy, setBusy] = createSignal(false)

	async function join() {
		setBusy(true)
		try {
			await lanApi.joinAddress(address(), password().trim() || null)
			navigate('/room')
		} catch (error) {
			pushNotice('warning', `join: ${describeError(error)}`)
		} finally {
			setBusy(false)
		}
	}

	async function leave() {
		try {
			await lanApi.stop()
			if (connected()) await api.logout(LAN)
		} catch (error) {
			pushNotice('warning', `leave: ${describeError(error)}`)
		}
	}

	return (
		<div class='server-card'>
			<div class='server-head'>
				<input
					class='server-name'
					aria-label='Your name on the LAN'
					value={props.entry.username}
					placeholder='Your name on the LAN'
					onInput={(event) =>
						props.change('username', event.currentTarget.value)
					}
				/>
				<span class={`chip ${status().tone}`}>{status().text}</span>
			</div>
			<p class='muted'>
				Games with people on your network, with no server in between. Rooms
				announced here appear in the battle list by themselves.
			</p>
			<div class='server-actions'>
				<button
					type='button'
					class='primary'
					onClick={() => navigate('/lan/host')}
				>
					Host a room
				</button>
				<input
					aria-label='Address'
					placeholder='Join by address, e.g. 192.168.1.5'
					value={address()}
					onInput={(event) => setAddress(event.currentTarget.value)}
				/>
				<input
					aria-label='Room password'
					placeholder='Password, if any'
					value={password()}
					onInput={(event) => setPassword(event.currentTarget.value)}
				/>
				<button
					type='button'
					disabled={busy() || address().trim() === ''}
					onClick={() => void join()}
				>
					Join
				</button>
				<Show when={connected()}>
					<button type='button' onClick={() => void leave()}>
						Leave the LAN
					</button>
				</Show>
			</div>
		</div>
	)
}
