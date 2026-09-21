import {
	For,
	Match,
	Show,
	Switch,
	createEffect,
	createResource,
	createSignal,
	onCleanup,
} from 'solid-js'
import type { SetStoreFunction } from 'solid-js/store'
import { Portal } from 'solid-js/web'
import { ActionCell, CellButton } from '../components/ActionCell'
import { Ask } from '../components/Ask'
import { LoginSheet } from '../components/LoginForm'
import { openExternal } from '../components/Linkify'
import { Row } from '../components/SettingRow'
import type { Account } from '../ipc/bindings/Account'
import type { BarConfig } from '../ipc/bindings/BarConfig'
import type { ServerEntry } from '../ipc/bindings/ServerEntry'
import type { Settings } from '../ipc/bindings/Settings'
import { api, describeError } from '../ipc/client'
import { localStore } from '../lib/resize'
import { headCount, readSeen, seenText } from '../lib/seen'
import {
	forgotPasswordUrl,
	guessedRapid,
	hostProblem,
	KNOWN,
	newServer,
	parsePorts,
	serverId,
	serverName,
	sessionStatus,
	splitHost,
} from '../lib/servers'
import { LanServerCard } from '../lan/LanServerCard'
import { isLan } from '../lan/lan'
import { pushNotice } from '../store/chat'
import { lobby } from '../store/lobby'

/** What the login sheet is open on, if anything. */
type Asking = { server: string; mode: 'login' | 'register' }

/**
 * The servers this app talks to, a card each, and a way to add another.
 *
 * Edits go into the page's draft like every other setting, and are saved the
 * same way; what needs the server — logging in, registering — opens the login
 * form over the page.
 */
export function ServerRows(props: {
	draft: Settings
	setDraft: SetStoreFunction<Settings>
	/** Saves the draft now; the login form acts on what is saved. */
	settle: () => Promise<void>
}) {
	const [asking, setAsking] = createSignal<Asking | null>(null)
	const [bar] = createResource(() => api.barConfig().catch(() => undefined))

	/**
	 * A server just added may publish its own games. Where most keep their
	 * rapid index is looked at once, and written in only if it is one with
	 * something of the server's own in it: a guess that is wrong leaves the
	 * field empty, and the server on BAR's games.
	 */
	async function add(typed: string) {
		const added = newServer(typed)
		props.setDraft('servers', (servers) => [...servers, added])
		if (added.rapid) return
		const guess = guessedRapid(added.host)
		const found = await api.checkRapid(guess).catch(() => null)
		if (!found || found.own === 0) return
		props.setDraft(
			'servers',
			(entry) => serverId(entry.host) === serverId(added.host) && !entry.rapid,
			'rapid',
			guess,
		)
	}

	async function ask(entry: ServerEntry, mode: Asking['mode']) {
		await props.settle()
		setAsking({ server: serverId(entry.host), mode })
	}

	return (
		<>
			<For each={props.draft.servers}>
				{(entry, index) => (
					<Show when={!isLan(entry)}>
						<Row>
							<ServerCard
								entry={entry}
								account={props.draft.account}
								bar={bar()}
								others={props.draft.servers
									.filter((_, at) => at !== index())
									.map((other) => other.host)}
								change={(field, value) =>
									props.setDraft('servers', index(), field, value)
								}
								remove={() =>
									props.setDraft('servers', (servers) =>
										servers.filter((_, at) => at !== index()),
									)
								}
								ask={(mode) => void ask(entry, mode)}
							/>
						</Row>
					</Show>
				)}
			</For>
			{/* Its own row, not the list's: that one is there only while it is on. */}
			<Row>
				<LanServerCard
					draft={props.draft}
					setDraft={props.setDraft}
					settle={props.settle}
				/>
			</Row>
			<Row>
				<AddServer
					listed={props.draft.servers.map((entry) => entry.host)}
					add={(typed) => void add(typed)}
				/>
			</Row>
			<Show when={asking()} keyed>
				{(open) => (
					<LoginSheet
						server={open.server}
						mode={open.mode}
						close={() => setAsking(null)}
					/>
				)}
			</Show>
		</>
	)
}

/** How long an address goes unedited before it is asked about. */
const CHECK_AFTER = 800

/**
 * What an address typed into a field turned out to be, asked once typing has
 * stopped — it is a request to somebody's server, not something to send per
 * keystroke. A refusal is an answer here, not a failure: a resource that
 * rejects with anything but an `Error` reports "Unknown error", and what Rust
 * sends is a plain object with the reason in it.
 */
function checked<T>(
	typed: () => string | null,
	ask: (url: string) => Promise<T>,
) {
	const [settled, setSettled] = createSignal(typed())
	createEffect(() => {
		const now = typed()
		const timer = setTimeout(() => setSettled(now), CHECK_AFTER)
		onCleanup(() => clearTimeout(timer))
	})
	const [answer] = createResource(
		() => settled() || null,
		(url) =>
			ask(url).then(
				(found) => ({ found, refused: null }),
				(error: unknown) => ({ found: null, refused: describeError(error) }),
			),
	)
	return answer
}

/** How long a Remove waits for its second click before it stands down. */
const CONFIRM_FOR = 4000

/** What Auto login on a card does, and whose answer that is. */
function autoTitle(entry: ServerEntry, account: Account): string {
	if (!account.rememberPassword)
		return 'Log in at startup: needs Remember passwords, under Account'
	const on = entry.autoLogin ?? account.autoLogin
	const whose = entry.autoLogin === null ? 'as Account says' : 'for this server'
	return `Log in at startup: ${on ? 'on' : 'off'}, ${whose}`
}

const PORTS_TIP =
	'Every port is tried encrypted both ways at once, and the first to answer is remembered.'
const UNENCRYPTED_TIP =
	'Only for a server with nothing else: your password and every message cross the network readable. Never raced against encryption.'

function ServerCard(props: {
	entry: ServerEntry
	account: Account
	/** What an empty address means: BAR's, as its launcher config says. */
	bar: BarConfig | undefined
	/** Every other server's host, which this one may not be changed to. */
	others: string[]
	change: <K extends keyof ServerEntry>(field: K, value: ServerEntry[K]) => void
	remove: () => void
	ask: (mode: 'login' | 'register') => void
}) {
	const id = () => serverId(props.entry.host)
	const session = () => lobby.servers[id()]
	const way = () => lobby.ways[id()]
	const [portsText, setPortsText] = createSignal(props.entry.ports.join(', '))
	const [removing, setRemoving] = createSignal(false)
	const [rehosting, setRehosting] = createSignal(false)
	// What each address turned out to be, asked once typing has stopped.
	const rapid = checked(
		() => props.entry.rapid,
		(url) => api.checkRapid(url),
	)
	const maps = checked(
		() => props.entry.maps,
		(url) => api.checkMapSearch(url).then(() => true),
	)
	/**
	 * Read once: open for a server that has needed it, and after that the
	 * reader's to open and close — not shut under the hand that unticks a box.
	 */
	const startsOpen = props.entry.allowUnencrypted

	const status = () => sessionStatus(session())
	/** Live while logged in; else the last count, since no server says before. */
	const headline = () => {
		const live = session()
		return seenText(
			live?.phase === 'ready' ? headCount(live) : null,
			readSeen(localStore(), id()),
			Date.now(),
		)
	}
	const connected = () => (session()?.phase ?? null) !== null
	const logsIn = () => props.entry.autoLogin ?? props.account.autoLogin

	async function act(what: string, run: () => Promise<unknown>) {
		try {
			await run()
		} catch (error) {
			pushNotice('warning', `${what}: ${describeError(error)}`)
		}
	}

	function remove() {
		if (!removing()) {
			setRemoving(true)
			setTimeout(() => setRemoving(false), CONFIRM_FOR)
			return
		}
		void act('remove', forget).then(props.remove)
	}

	/**
	 * A server that is gone from the list is not one to stay logged in to,
	 * nor one to go on keeping a password and a way in for -- and a changed
	 * host is a server gone, since the host is what it is known by.
	 */
	async function forget() {
		if (connected()) await api.logout(id())
		if (props.entry.username)
			await api.clearPassword(id(), props.entry.username)
		await api.forgetWay(props.entry.host)
	}

	/** A port typed after the host is its one port, as when it was added. */
	function rehost(typed: string) {
		setRehosting(false)
		const { host, port } = splitHost(typed)
		if (port !== null) {
			props.change('ports', [port])
			setPortsText(String(port))
		}
		if (serverId(host) === id()) return props.change('host', host)
		void act('change the host', forget).then(() => props.change('host', host))
	}

	return (
		<div class='server-card'>
			<div class='server-head'>
				<input
					class='server-name'
					aria-label='Name'
					value={props.entry.name}
					placeholder={props.entry.host}
					onInput={(event) => props.change('name', event.currentTarget.value)}
				/>
				<button
					type='button'
					class='chip-choice'
					classList={{ on: logsIn() }}
					aria-pressed={logsIn()}
					disabled={!props.account.rememberPassword}
					title={autoTitle(props.entry, props.account)}
					onClick={() => props.change('autoLogin', !logsIn())}
				>
					Auto login
				</button>
				<span class={`chip ${status().tone}`}>{status().text}</span>
			</div>
			<p class='muted'>
				{props.entry.host}{' '}
				<ActionCell>
					<CellButton
						icon='act-pen'
						title='Change the host'
						onClick={() => setRehosting(true)}
					/>
				</ActionCell>
				<Show when={props.entry.username}>
					{(name) => <> · account {name()}</>}
				</Show>
				<Show when={headline()}>{(text) => <> · {text()}</>}</Show>
				<Show when={way()}>
					{(found) => (
						<>
							{' · '}last way in: <b>{found()}</b>{' '}
							<button
								type='button'
								class='link'
								onClick={() =>
									void act('re-probe', () => api.forgetWay(props.entry.host))
								}
							>
								try every way again
							</button>
						</>
					)}
				</Show>
			</p>
			<Show when={rehosting()}>
				<Portal>
					<Ask
						title='Change the host'
						hint='Another host is another server: the password kept for this one is forgotten. A port may follow a colon.'
						initial={props.entry.host}
						confirm='Change'
						problem={(typed) => hostProblem(typed, props.others)}
						onCancel={() => setRehosting(false)}
						onAnswer={rehost}
					/>
				</Portal>
			</Show>
			{/* Set once, if ever: a server that works is never opened here. */}
			<details class='server-connection' open={startsOpen}>
				<summary>Connection</summary>
				<label title={PORTS_TIP}>
					Ports
					<input
						value={portsText()}
						aria-invalid={parsePorts(portsText()) === null}
						onInput={(event) => {
							setPortsText(event.currentTarget.value)
							const ports = parsePorts(event.currentTarget.value)
							if (ports) props.change('ports', ports)
						}}
					/>
				</label>
				<Show when={parsePorts(portsText()) === null}>
					<p class='error'>
						Port numbers, separated by commas — not saved until they are.
					</p>
				</Show>
				<label class='row server-plain' title={UNENCRYPTED_TIP}>
					<input
						type='checkbox'
						checked={props.entry.allowUnencrypted}
						onChange={(event) =>
							props.change('allowUnencrypted', event.currentTarget.checked)
						}
					/>
					Allow unencrypted, once every encrypted way has failed
				</label>
				<label class='split'>
					Rapid server, where this server's own games are published
					<input
						value={props.entry.rapid ?? ''}
						placeholder={props.bar?.rapidMaster}
						onInput={(event) =>
							props.change('rapid', event.currentTarget.value.trim() || null)
						}
					/>
				</label>
				<Switch>
					<Match when={!props.entry.rapid}>
						<p class='muted'>
							Without one, this server's games are looked for in BAR's. A game
							is only ever looked for in its own server's: a mod's name goes
							nowhere else.
						</p>
					</Match>
					<Match when={rapid.loading}>
						<p class='muted'>Reading it…</p>
					</Match>
					<Match when={rapid()?.refused}>
						{(why) => <p class='error'>{why()}. Nothing is fetched from it.</p>}
					</Match>
					<Match when={rapid()?.found}>
						{(found) => (
							<p class='muted'>
								Lists {found().own} of its own and {found().bars} of BAR's.
							</p>
						)}
					</Match>
				</Switch>
				<label class='split'>
					Map search, where this server's own maps are found
					<input
						value={props.entry.maps ?? ''}
						placeholder={props.bar?.search}
						onInput={(event) =>
							props.change('maps', event.currentTarget.value.trim() || null)
						}
					/>
				</label>
				<Switch
					fallback={
						<p class='muted'>
							Asked only for a map that is not one of BAR's; BAR is asked only
							for those that are.
						</p>
					}
				>
					<Match when={props.entry.maps && maps.loading}>
						<p class='muted'>Asking it…</p>
					</Match>
					<Match when={props.entry.maps && maps()?.refused}>
						{(why) => <p class='error'>{why()}. No map is fetched from it.</p>}
					</Match>
					<Match when={props.entry.maps && maps()?.found}>
						<p class='muted'>
							Answers like a map search. Asked only for a map that is not one of
							BAR's.
						</p>
					</Match>
				</Switch>
			</details>
			<div class='server-actions'>
				<Show
					when={connected()}
					fallback={
						<>
							<button
								type='button'
								class='primary'
								onClick={() => props.ask('login')}
							>
								Log in
							</button>
							<button type='button' onClick={() => props.ask('register')}>
								Register
							</button>
						</>
					}
				>
					<button
						type='button'
						onClick={() => void act('log out', () => api.logout(id()))}
					>
						Log out
					</button>
				</Show>
				<button
					type='button'
					onClick={() => void openExternal(forgotPasswordUrl(props.entry))}
				>
					Reset password
				</button>
				<Show when={props.entry.username}>
					{(name) => (
						<button
							type='button'
							onClick={() =>
								void act('forget the password', async () => {
									await api.clearPassword(id(), name())
									pushNotice('info', `forgot the password for ${name()}`)
								})
							}
						>
							Forget password
						</button>
					)}
				</Show>
				{/* The servers every install has stay on the list. */}
				<Show when={!props.entry.builtin}>
					<button
						type='button'
						class='server-remove'
						classList={{ danger: removing() }}
						onClick={remove}
					>
						{removing() ? 'Really remove?' : 'Remove'}
					</button>
				</Show>
			</div>
		</div>
	)
}

function AddServer(props: { listed: string[]; add: (host: string) => void }) {
	const [host, setHost] = createSignal('')
	const problem = () => hostProblem(host(), props.listed)
	const unlisted = () =>
		KNOWN.filter(
			(known) =>
				!props.listed.some((held) => serverId(held) === serverId(known.host)),
		)

	function add() {
		if (problem() !== null) return
		props.add(host())
		setHost('')
	}

	return (
		<>
			<div class='server-add'>
				<label>
					Add a server
					<input
						value={host()}
						list='known-servers'
						placeholder='its host, e.g. server.example.com'
						onInput={(event) => setHost(event.currentTarget.value)}
						onKeyDown={(event) => {
							if (event.key !== 'Enter') return
							event.preventDefault()
							add()
						}}
					/>
					<datalist id='known-servers'>
						<For each={unlisted()}>
							{(known) => <option value={known.host}>{known.name}</option>}
						</For>
					</datalist>
				</label>
				<button type='button' disabled={problem() !== null} onClick={add}>
					Add
				</button>
			</div>
			<Show when={host().trim() && problem()}>
				{(why) => <p class='error'>{why()}</p>}
			</Show>
			<p class='muted'>
				Each server has its own accounts: an account on one is nothing to
				another. Add a server once — the same one under two names would be two
				logins that keep throwing each other out.
			</p>
		</>
	)
}
