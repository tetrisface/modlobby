import { useNavigate } from '@solidjs/router'
import { Show, createSignal } from 'solid-js'
import type { SetStoreFunction } from 'solid-js/store'
import type { Settings } from '../ipc/bindings/Settings'
import { api, describeError } from '../ipc/client'
import { sessionStatus } from '../lib/servers'
import { pushNotice } from '../store/chat'
import { lobby } from '../store/lobby'
import { lanApi } from './api'
import { LAN, isLan } from './lan'

/**
 * The LAN's card among the servers: the switch, a name to appear as, where
 * things stand, and the two ways in — host a room, or join one by its
 * address when the network did not carry the announcement.
 *
 * Drawn whether the LAN is on or off, since the switch is on it. Its row in
 * the server list is there only while it is on, so the name waits for that.
 */
export function LanServerCard(props: {
	draft: Settings
	setDraft: SetStoreFunction<Settings>
	/** Saves the draft now; hosting asks Rust, which acts on what is saved. */
	settle: () => Promise<void>
}) {
	const navigate = useNavigate()
	const at = () => props.draft.servers.findIndex(isLan)
	const entry = () => props.draft.servers[at()]
	const session = () => lobby.servers[LAN]
	const status = () => sessionStatus(session())
	const connected = () => (session()?.phase ?? null) !== null
	const [address, setAddress] = createSignal('')
	const [password, setPassword] = createSignal('')
	const [busy, setBusy] = createSignal(false)

	/**
	 * Pressing it is asking for the LAN, so it is switched on first -- and
	 * saved now, since leaving the page drops a save that is still waiting.
	 */
	async function host() {
		props.setDraft('lan', 'enabled', true)
		await props.settle()
		navigate('/lan/host')
	}

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
					value={entry()?.username ?? ''}
					placeholder='Your name on the LAN'
					disabled={!entry()}
					onInput={(event) =>
						props.setDraft(
							'servers',
							at(),
							'username',
							event.currentTarget.value,
						)
					}
				/>
				<span class={`chip ${status().tone}`}>{status().text}</span>
			</div>
			<label class='row'>
				<input
					type='checkbox'
					checked={props.draft.lan.enabled}
					onChange={(event) =>
						props.setDraft('lan', 'enabled', event.currentTarget.checked)
					}
				/>
				Enable LAN games
			</label>
			<p class='muted'>
				Games with people on your network, with no server in between. Rooms
				announced here appear in the battle list by themselves.
			</p>
			<div class='server-actions'>
				<button type='button' class='primary' onClick={() => void host()}>
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
