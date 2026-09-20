import { HashRouter, Route, useLocation, useNavigate } from '@solidjs/router'
import { listen } from '@tauri-apps/api/event'
import {
	For,
	Show,
	createEffect,
	createSignal,
	onCleanup,
	onMount,
	type ParentProps,
} from 'solid-js'
import { AccountMenu } from './components/AccountMenu'
import { GameActions } from './components/GameActions'
import { Glyph, IconSprite } from './components/icons'
import { Thinking } from './components/Thinking'
import { NavTabs } from './components/NavTabs'
import { PlayerMenu } from './components/PlayerMenu'
import { connectChannel } from './ipc/channel'
import { ACTIVITY_EVENTS, activityReporter } from './lib/activity'
import { serverId } from './lib/servers'
import {
	clickLeavesOverlay,
	escapeLeavesOverlay,
	roomOnScreen,
} from './lib/overlay'
import { api, describeError, errorCode } from './ipc/client'
import type { Settings } from './ipc/bindings/Settings'
import { build, setBuild } from './store/build'
import {
	chat,
	holdNotices,
	pushNotice,
	roomKey,
	unreadTotal,
} from './store/chat'
import {
	allAway,
	anyConnected,
	lobby,
	mainSession,
	myRoom,
	roomSession,
	sessions,
	severalServers,
	soonestRetry,
} from './store/lobby'
import { loadNews, unreadNews } from './store/news'
import { over, setOver } from './store/overlay'
import { autoLogin, loginHold } from './store/session'
import {
	applySettings,
	nudgeScale,
	resetScale,
	serverLabel,
	settings,
} from './store/settings'
import {
	available,
	busy as updating,
	checkUpdate,
	checking,
	downloading,
	failure,
	heldBy,
	installUpdate,
	resumeUpdate,
	waiting,
	watchUpdates,
} from './store/update'
import { LanHostForm } from './lan/LanHostForm'
import { BattleList } from './views/BattleList'
import { Chat } from './views/Chat'
import { Home } from './views/Home'
import { Login } from './views/Login'
import { News } from './views/News'
import { PresetsPage } from './views/PresetsPage'
import { Replays } from './views/Replays'
import { OnlineRoom } from './views/room/OnlineRoom'
import { SkirmishRoom } from './views/room/SkirmishRoom'
import { SettingsView } from './views/Settings'
import { Widgets } from './views/Widgets'

type SettingsEvent = { changed: Settings } | { invalid: string }

/**
 * The corner while nobody is logged in. A button, because the one thing to do
 * from here is try again: the runtime retries a dropped connection by itself,
 * but on a timer sized for a server that dropped everyone at once, and a
 * person watching the corner need not wait for it.
 */
function Reconnect() {
	const navigate = useNavigate()
	const connecting = () => anyConnected()
	/** Learned from the runtime: there is nothing to reconnect with. */
	const [needsLogin, setNeedsLogin] = createSignal(false)
	/**
	 * Seconds until the next attempt that is going to happen without a click:
	 * the runtime's own retry after a drop or a refusal, or the auto-login the
	 * throttle is holding back. `null` when nothing is coming.
	 */
	const [left, setLeft] = createSignal<number | null>(null)
	createEffect(() => {
		const at = soonest(soonestRetry(), loginHold())
		if (at === null) {
			setLeft(null)
			return
		}
		const tick = () => setLeft(Math.max(0, Math.ceil((at - Date.now()) / 1000)))
		tick()
		const timer = setInterval(tick, 1000)
		onCleanup(() => clearInterval(timer))
	})

	/** Every server that is not connected, tried again at once. */
	async function reconnect() {
		const idle = (settings()?.servers ?? [])
			.map((entry) => serverId(entry.host))
			.filter((server) => (lobby.servers[server]?.phase ?? null) === null)
		const failed = await Promise.all(
			idle.map((server) =>
				api.reconnect(server).then(
					() => null,
					(error: unknown) => error,
				),
			),
		)
		const errors = failed.filter((error) => error !== null)
		// Nothing to try again with anywhere: this run never logged in, or
		// logged out of everything.
		if (errors.length === idle.length && errors.every(noCredentials)) {
			setNeedsLogin(true)
			navigate('/login')
			return
		}
		for (const error of errors)
			if (!noCredentials(error)) pushNotice('warning', describeError(error))
	}

	const label = () => {
		if (connecting()) return 'connecting'
		if (needsLogin()) return 'log in'
		const seconds = left()
		return seconds ? `retrying in ${seconds}s` : 'not logged in'
	}
	const title = () => {
		if (connecting()) return 'Connecting'
		if (left()) return 'Logging in again when the count ends. Click to try now.'
		return 'Reconnect'
	}

	return (
		<button
			type='button'
			class='reconnect'
			disabled={connecting()}
			title={title()}
			onClick={() => void reconnect()}
		>
			<Glyph id='act-reconnect' />
			{label()}
		</button>
	)
}

const noCredentials = (error: unknown) => errorCode(error) === 'noCredentials'

/** The earlier of two moments, either of which may be missing. */
function soonest(a: number | null, b: number | null): number | null {
	if (a === null) return b
	if (b === null) return a
	return Math.min(a, b)
}

function Layout(props: ParentProps) {
	/**
	 * Everything unread, anywhere. A notification is only raised while the
	 * window is in the background, so without this a message that arrives while
	 * you are reading the battle list leaves no mark at all. Muted rooms stay
	 * out of it until they name you.
	 */
	const unread = () => unreadTotal(settings()?.chat.muted ?? [])
	const named = () => Object.values(chat.named).some(Boolean)

	/**
	 * The window controls in the nav's corner.
	 *
	 * They earn their place because the OS title bar is not a given here — a
	 * transparent window on Windows can lose its frame, and fullscreen has no
	 * chrome at all — and a window that cannot be un-fullscreened or closed from
	 * inside itself is a trap.
	 */
	const [fullscreen, setFullscreen] = createSignal(false)
	onMount(
		() =>
			void api
				.isFullscreen()
				.then(setFullscreen)
				.catch(() => {}),
	)

	/**
	 * Which version this is, in the corner where the brand is. Clicking it looks
	 * for a newer one; found, a button beside it offers the restart into it.
	 */
	onMount(() => {
		void api
			.appVersion()
			.then(setBuild)
			.catch(() => {})
		onCleanup(watchUpdates())
	})
	/** Why the last look failed, said in the tooltip for a while, not forever. */
	const [failedFor, setFailedFor] = createSignal<string | null>(null)
	createEffect(() => {
		const reason = failure()
		setFailedFor(reason)
		if (reason === null) return
		const timer = setTimeout(() => setFailedFor(null), 60_000)
		onCleanup(() => clearTimeout(timer))
	})
	const versionTitle = () => {
		if (checking()) return 'Looking for a newer version…'
		const reason = failedFor()
		if (reason) return `Could not look for an update: ${reason}`
		return 'The version running. Click to look for a newer one.'
	}
	/** Found, fetching or fetched: while any of them, the button is there. */
	const hasUpdate = () =>
		available() !== null || downloading() !== null || waiting() !== null
	const updateTitle = () => {
		const next = waiting()
		if (next !== null) {
			return heldBy()
				? `Version ${next} is downloaded and installs on the next start. Restarting now would lose ${heldBy()}.`
				: `Version ${next} is downloaded. Restart into it.`
		}
		const percent = downloading()
		if (percent !== null) return `Downloading the update — ${percent}%`
		return `Version ${available()} is out. Fetch it and restart into it.`
	}

	/** What the servers say about us, which is what everyone else can see. */
	const away = allAway

	/** Which page is up: a room page carries the game buttons itself. */
	const route = useLocation()

	/**
	 * Over a game the page dresses as a modal: a centred card on a see-through
	 * scrim, and every way out a modal has — Esc, or a click on the scrim —
	 * hands the game back. `over` lives in `store/overlay`; it is seeded here
	 * by asking, because a webview that reloads mid-overlay was not there for
	 * the event.
	 */
	onMount(() => {
		void api.overlayActive().then(setOver)
		const pending = listen<boolean>('overlay', (event) =>
			setOver(event.payload),
		)
		onCleanup(() => void pending.then((unlisten) => unlisten()))
		/**
		 * Raised over a game the window is shown transparent, so that its first
		 * frames -- whatever the webview last had, at the old size -- are never
		 * seen. Two animation frames after being told is when the overlay dress
		 * has actually been painted, and the window is told it can be seen.
		 */
		const veiled = listen('overlay-veiled', () =>
			requestAnimationFrame(() =>
				requestAnimationFrame(() => void api.overlayPainted().catch(() => {})),
			),
		)
		onCleanup(() => void veiled.then((unlisten) => unlisten()))

		const keys = (event: KeyboardEvent) => {
			if (over() && escapeLeavesOverlay(event)) void api.overlayToggle()
		}
		const clicks = (event: MouseEvent) => {
			if (over() && clickLeavesOverlay(event.target)) void api.overlayToggle()
		}
		/**
		 * Ctrl+wheel sizes the interface, Ctrl+0 puts it back.
		 *
		 * Not passive: the whole point is to take the gesture off the browser,
		 * which would otherwise zoom the webview itself and leave the two
		 * fighting over the same wheel.
		 */
		const zoom = (event: WheelEvent) => {
			if (!event.ctrlKey) return
			event.preventDefault()
			if (event.deltaY !== 0) nudgeScale(event.deltaY < 0 ? 1 : -1)
		}
		const reset = (event: KeyboardEvent) => {
			if ((event.ctrlKey || event.metaKey) && event.key === '0') {
				event.preventDefault()
				resetScale()
			}
		}
		window.addEventListener('keydown', keys)
		window.addEventListener('keydown', reset)
		window.addEventListener('mousedown', clicks)
		window.addEventListener('wheel', zoom, { passive: false })
		// The idle disconnect counts from the last of these. Passive: none of
		// them is prevented, and a scroll must not wait on IPC.
		const touch = activityReporter(() => void api.activity().catch(() => {}))
		for (const event of ACTIVITY_EVENTS)
			window.addEventListener(event, touch, { passive: true })
		onCleanup(() => {
			window.removeEventListener('keydown', keys)
			window.removeEventListener('keydown', reset)
			window.removeEventListener('mousedown', clicks)
			window.removeEventListener('wheel', zoom)
			for (const event of ACTIVITY_EVENTS)
				window.removeEventListener(event, touch)
		})
	})
	createEffect(() =>
		document.documentElement.classList.toggle('overlay', over()),
	)

	/**
	 * Channels are rejoined once a session.
	 *
	 * The server forgets your channels the moment you disconnect, so a client
	 * that does not remember them leaves you rejoining `#main` by hand on every
	 * launch. The list is read from the file here rather than from the settings
	 * signal: this runs the instant the lobby is ready, and a signal that has
	 * not arrived yet looks exactly like a user with no channels.
	 *
	 * What gets written back is driven by joining and leaving, never by a diff
	 * of what happens to be open — that would save an empty list in the moment
	 * between asking to join and being let in, which is precisely when this
	 * effect fires.
	 */
	const restored = new Set<string>()

	createEffect(() => {
		for (const [server, session] of sessions()) {
			// A session that ends takes its channel membership with it. Without
			// this reset, logging out and back in within one run leaves you in
			// none of your channels, because the restore had already happened.
			if (session.phase === null) restored.delete(server)
			if (session.phase !== 'ready' || restored.has(server)) continue
			restored.add(server)
			void restoreChannels(server)
		}
	})

	async function restoreChannels(server: string) {
		try {
			const saved = await api.getSettings()
			const entry = saved.servers.find((held) => serverId(held.host) === server)
			for (const name of entry?.channels ?? []) {
				if (!(roomKey(server, name) in chat.channels))
					await api.joinChannel(server, name, null)
			}
		} catch (error) {
			pushNotice('warning', describeError(error))
		}
	}

	onMount(async () => {
		try {
			const saved = await api.getSettings()
			applySettings(saved)
			await connectChannel()
			// A download an earlier run kept installs now, before the login: the
			// restart it ends in would only spend another login on the server's
			// count. Comes back only when there is nothing to install.
			await resumeUpdate()
			// The one place that already holds the settings, so auto-login neither
			// reads them again nor races the signal that carries them.
			void autoLogin(saved)
			// The count belongs to the nav, which is here whether or not the News
			// tab ever is, so the feed is asked for from the shell. Rust answers
			// from its own cache for the hour it trusts one, so most launches make
			// no request at all, and nothing is scheduled.
			void loadNews()
		} catch (error) {
			pushNotice('error', describeError(error))
		}
	})
	onMount(() => {
		// `listen` resolves after a round trip. Registering the cleanup on the
		// promise keeps it inside this component's owner — awaiting first would
		// leave the owner behind and the listener would never be removed.
		const pending = listen<SettingsEvent>('settings', (event) => {
			if ('changed' in event.payload) {
				applySettings(event.payload.changed)
				pushNotice('info', 'settings reloaded')
			} else {
				pushNotice(
					'warning',
					`settings file not applied: ${event.payload.invalid}`,
				)
			}
		})
		onCleanup(() => void pending.then((unlisten) => unlisten()))
	})

	return (
		<div class='shell'>
			<IconSprite />
			{/* The nav doubles as the title bar, because the transparent window
          has none: empty nav space drags the window, double-click maximizes.
          Only in an ordinary window — a fullscreen or overlaid one is not a
          thing to drag around. */}
			<nav
				class='nav'
				data-tauri-drag-region={!fullscreen() && !over() ? true : undefined}
			>
				<span class='brand'>
					modlobby
					<span class='stage' title='Alpha — Expect rough edges.'>
						alpha
					</span>
				</span>
				{/* The pages, folding into a menu as the window narrows. The row
            takes whatever is between the brand and the account, so the room
            card's coming and going moves nothing else. The way in is among
            them while there is no session: the corner's reconnect button
            resumes one that dropped; that link is for not having one. The
            room is gated on the phase like the lobby views are: a reconnect
            keeps myBattle. */}
				<NavTabs
					room={roomSession()?.phase === 'ready' ? myRoom() : undefined}
					loggedOut={!anyConnected()}
					unread={unread()}
					named={named()}
					news={unreadNews()}
					drag={!fullscreen() && !over()}
				/>
				{/* Small and out of the way, level with the account: the
            version is worth a glance, not a place in the row. An update
            puts its button beside it; the version itself never changes. */}
				<span class='version-slot'>
					<Show when={build()}>
						{(found) => (
							<>
								<Show when={hasUpdate()}>
									<button
										type='button'
										class='primary'
										title={updateTitle()}
										disabled={downloading() !== null}
										onClick={() => void installUpdate()}
									>
										{/* Says what the click costs: fetched already, it only
                        restarts; otherwise it fetches first. */}
										{waiting() ? 'Restart to Update' : 'Update and Restart'}
										<Show when={downloading() !== null}>
											{' '}
											· {downloading()}%
										</Show>
									</button>
								</Show>
								<button
									type='button'
									class='version'
									classList={{ steady: downloading() !== null }}
									title={versionTitle()}
									disabled={updating()}
									onClick={() => void checkUpdate()}
								>
									{found().version}
									<Show when={checking()}>
										<Thinking title='Looking for a newer version' />
									</Show>
								</button>
							</>
						)}
					</Show>
				</span>
				<Show when={mainSession()?.me} fallback={<Reconnect />}>
					<AccountMenu name={mainSession()?.me ?? ''} />
					{/* The server keeps this bit, so what it says is what everyone else
              sees — no local guess to drift out of step with it. */}
					<button
						class='chip-choice'
						classList={{ on: away() }}
						title={
							(away()
								? 'Everyone sees you as away'
								: 'Tell everyone you have stepped out') +
							(severalServers() ? ', on every server' : '')
						}
						onClick={() => void api.setAway(!away())}
					>
						Away
					</button>
					<button
						title={
							severalServers()
								? 'Log out of every server. Your name has each one on its own.'
								: 'Log out'
						}
						onClick={() => api.logout(null)}
					>
						Log out
					</button>
				</Show>
				{/* Not over a game. The page is a modal there, and its ways out are
            Back to game and the guarded Quit; a close in this corner would
            end the lobby under a game that still depends on it, and it sits
            exactly where a hand expects "back to game". */}
				<Show when={!over()}>
					<div class='win-controls'>
						<button
							class='win-btn'
							title={fullscreen() ? 'Windowed' : 'Full screen'}
							aria-label={fullscreen() ? 'Windowed' : 'Full screen'}
							onClick={() =>
								void api
									.toggleFullscreen()
									.then(setFullscreen)
									.catch((error) => pushNotice('warning', describeError(error)))
							}
						>
							<svg viewBox='0 0 12 12' aria-hidden='true'>
								<Show
									when={fullscreen()}
									fallback={
										// Corners pointing out: take the whole screen.
										<path d='M1 4V1h3M8 1h3v3M11 8v3H8M4 11H1V8' />
									}
								>
									{/* Corners pointing in: back to a window. */}
									<path d='M4 1v3H1M11 4H8V1M8 11V8h3M1 8h3v3' />
								</Show>
							</svg>
						</button>
						<button
							class='win-btn close'
							title='Close modlobby (a running game keeps going)'
							aria-label='Close modlobby'
							onClick={() => void api.shutdown()}
						>
							<svg viewBox='0 0 12 12' aria-hidden='true'>
								<path d='M2 2l8 8M10 2l-8 8' />
							</svg>
						</button>
					</div>
				</Show>
			</nav>
			<main class='main'>{props.children}</main>
			<Notices />
			<PlayerMenu />
			{/* The ways out, for the pages that have no room card: the card draws
          these itself, in its own column, so they are never in two corners
          at once. In the order you are likely to want them: carry on in the
          lobby (already true, so no button), stop playing, or go back. */}
			<Show when={over() && !roomOnScreen(route.pathname)}>
				<div class='overlay-chrome'>
					<GameActions />
					<button
						class='primary'
						title='Or press Escape'
						onClick={() => void api.overlayToggle()}
					>
						Back to game
					</button>
				</div>
			</Show>
		</div>
	)
}

function Notices() {
	const navigate = useNavigate()
	return (
		// Held while the pointer is in here, so that reading one does not race
		// its own timer -- which is the whole complaint about a corner like this.
		<div
			class='notices'
			onPointerEnter={() => holdNotices(true)}
			onPointerLeave={() => holdNotices(false)}
		>
			<For each={chat.notices.slice(-3)}>
				{(notice) => (
					<div class={`notice ${notice.level}`}>
						<span class='notice-text'>
							{/* Which server said it, once there is more than one to have. */}
							<Show when={severalServers() && notice.server}>
								{(server) => <b>{serverLabel(server())}: </b>}
							</Show>
							{notice.text}
						</span>
						{/* The way out of the corner: which of these appear, and where. */}
						<button
							class='notice-settings'
							title='Which notifications appear, and where'
							aria-label='Notification settings'
							onClick={() => navigate('/settings?section=notifications')}
						>
							<Glyph id='act-gear' />
						</button>
					</div>
				)}
			</For>
		</div>
	)
}
export function App() {
	return (
		<HashRouter root={Layout}>
			<Route path='/' component={Home} />
			<Route path='/login' component={Login} />
			<Route path='/battles' component={BattleList} />
			<Route path='/lan/host' component={LanHostForm} />
			<Route path='/chat' component={Chat} />
			<Route path='/news' component={News} />
			<Route path='/widgets' component={Widgets} />
			<Route path='/replays' component={Replays} />
			<Route path='/presets' component={PresetsPage} />
			<Route path='/skirmish' component={SkirmishRoom} />
			<Route path='/room' component={OnlineRoom} />
			{/* The tweak editor lives inside the room's setup pane, which is where
          the slots it edits are listed. It was reachable here as well, drawing
          the same component with none of that around it. */}
			<Route path='/room/tweaks' component={OnlineRoom} />
			<Route path='/settings' component={SettingsView} />
		</HashRouter>
	)
}
