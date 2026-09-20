import { useNavigate } from '@solidjs/router'
import { For, Show, createResource, createSignal } from 'solid-js'
import { MapPicker } from '../components/MapPicker'
import { Select } from '../components/Select'
import { api, describeError } from '../ipc/client'
import { mapNames } from '../lib/maps'
import { pushNotice } from '../store/chat'
import { lobby } from '../store/lobby'
import { lanApi } from './api'

type Policy = 'open' | 'password' | 'approve'

/**
 * Opening a room on the network: what it is called, who may come in, and
 * what it is played on — chosen here, because the engine that runs the game
 * is this machine's and cannot change once people have joined.
 */
export function LanHostForm() {
	const navigate = useNavigate()
	const [options] = createResource(() => api.skirmishOptions())
	const [names] = createResource(mapNames)
	// The room's own title, which a skirmish calls itself "Skirmish"; a room
	// on the network wants your name on it, so an untouched one is left blank
	// for the placeholder to speak.
	const [title, setTitle] = createSignal('')
	const [policy, setPolicy] = createSignal<Policy>('open')
	const [password, setPassword] = createSignal('')
	const [maxPlayers, setMaxPlayers] = createSignal(8)
	const [engine, setEngine] = createSignal<string | null>(null)
	const [game, setGame] = createSignal<string | null>(null)
	const [map, setMap] = createSignal<string | null>(null)
	const [pickingMap, setPickingMap] = createSignal(false)
	const [busy, setBusy] = createSignal(false)

	/**
	 * The skirmish room, which is where Host on LAN is pressed: the game you
	 * just set up is the one you mean to open, so it is what this opens on.
	 * It outlives the room being closed, so coming here from the Servers card
	 * lands on the last thing you played rather than on nothing.
	 */
	const from = () => lobby.skirmish?.battle

	// Then the newest of each, the way a skirmish guesses it from cold.
	const engineChosen = () =>
		engine() ?? from()?.engineVersion ?? options()?.engines[0] ?? ''
	const gameChosen = () =>
		game() ?? from()?.gameName ?? options()?.games[0] ?? ''
	const mapChosen = () => {
		const chosen = map() ?? from()?.mapName
		if (chosen) return chosen
		const first = options()?.maps[0]
		return first === undefined ? '' : (names()?.[first] ?? first)
	}
	const ready = () =>
		engineChosen() !== '' && gameChosen() !== '' && mapChosen() !== ''

	async function host(event: SubmitEvent) {
		event.preventDefault()
		setBusy(true)
		try {
			await lanApi.host({
				title: title(),
				password: policy() === 'password' ? password() : null,
				approve: policy() === 'approve',
				maxPlayers: maxPlayers(),
				engineVersion: engineChosen(),
				game: gameChosen(),
				map: mapChosen(),
			})
			navigate('/room')
		} catch (error) {
			pushNotice('warning', `host a room: ${describeError(error)}`)
		} finally {
			setBusy(false)
		}
	}

	return (
		<div class='login-page'>
			<form class='login' onSubmit={(event) => void host(event)}>
				<h1>Host a room on the LAN</h1>
				<label>
					Title
					<input
						value={title()}
						placeholder='Your name, unless you say'
						onInput={(event) => setTitle(event.currentTarget.value)}
					/>
				</label>
				<label>
					Who may join
					<Select
						value={policy()}
						onChange={(event) => setPolicy(event.currentTarget.value as Policy)}
					>
						<option value='open'>Anyone on the network</option>
						<option value='password'>Anyone with the password</option>
						<option value='approve'>Whoever I let in, one by one</option>
					</Select>
				</label>
				<Show when={policy() === 'password'}>
					<label>
						Password
						<input
							value={password()}
							onInput={(event) => setPassword(event.currentTarget.value)}
						/>
					</label>
				</Show>
				<Show when={policy() === 'approve'}>
					<p class='muted'>
						Each arrival is announced in the room's chat; answer with{' '}
						<code>!accept name</code> or <code>!deny name</code>.
					</p>
				</Show>
				<label>
					Players at most
					<input
						type='number'
						min={1}
						max={32}
						value={maxPlayers()}
						onChange={(event) =>
							setMaxPlayers(
								Math.max(
									1,
									Math.min(32, Number(event.currentTarget.value) || 8),
								),
							)
						}
					/>
				</label>
				<label>
					Engine
					<Select
						value={engineChosen()}
						onChange={(event) => setEngine(event.currentTarget.value)}
					>
						<For each={options()?.engines ?? []}>
							{(version) => <option value={version}>{version}</option>}
						</For>
					</Select>
				</label>
				<label>
					Game
					<Select
						value={gameChosen()}
						onChange={(event) => setGame(event.currentTarget.value)}
					>
						<For each={options()?.games ?? []}>
							{(name) => <option value={name}>{name}</option>}
						</For>
					</Select>
				</label>
				<label>
					Map
					<button type='button' onClick={() => setPickingMap(true)}>
						{mapChosen() || 'choose one'}
					</button>
				</label>
				<Show when={pickingMap()}>
					<MapPicker
						current={mapChosen()}
						onPick={(name) => {
							setMap(name)
							setPickingMap(false)
						}}
						onClose={() => setPickingMap(false)}
					/>
				</Show>
				<p class='muted'>
					Your engine runs the game and listens on UDP 8452; the first time, the
					system may ask whether to allow that.
				</p>
				<Show when={(options()?.engines.length ?? 1) === 0}>
					<p class='error'>
						No engine is installed. Open the skirmish room first; it fetches
						one.
					</p>
				</Show>
				<button class='primary' type='submit' disabled={busy() || !ready()}>
					Open the room
				</button>
			</form>
		</div>
	)
}
