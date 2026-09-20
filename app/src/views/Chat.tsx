import { A } from '@solidjs/router'
import { createVirtualizer } from '@tanstack/solid-virtual'
import {
	For,
	Show,
	createEffect,
	createMemo,
	createSignal,
	onMount,
} from 'solid-js'
import type { ChatLine } from '../ipc/bindings/ChatLine'
import { api, describeError } from '../ipc/client'
import {
	BATTLE_ROOM,
	SERVER_ROOM,
	byActivity,
	chat,
	ensureRoom,
	closePrivate,
	isPrivate,
	parseKey,
	partner,
	privateRoom,
	pushNotice,
	pushSystem,
	openChannels,
	openPrivates,
	roomKey,
	roomName,
	serverRoom,
	watchRoom,
} from '../store/chat'
import { Composer } from '../components/Composer'
import { Linkify } from '../components/Linkify'
import { showPlayerMenu } from '../components/PlayerMenu'
import { Glyph } from '../components/icons'
import { isMuted, rememberChannel, toggleMute } from '../store/channels'
import { TabStrip, type Tab as StripTab } from '../components/TabStrip'
import { ordered } from '../lib/reorder'
import { clashes } from '../lib/servers'
import {
	anyReady,
	lobby,
	mainServer,
	myRoom,
	roomServer,
	sessions,
	severalServers,
} from '../store/lobby'
import { serverLabel, settings } from '../store/settings'

/**
 * Channels and private messages.
 *
 * The room we are in is listed here too, so every conversation is reachable
 * from one place, but it stays in the battle room as well — you should not
 * have to leave the players to read what they are saying.
 */
const ROSTER_ROW = 19

/** A muted room's count, drawn softly — unless it names you. */
const quiet = (room: string) => isMuted(room) && !chat.named[room]

/**
 * The server a room's words go to: the one in its key, or for the battle
 * room — one across every server — the server the room is on.
 */
const serverOf = (key: string) =>
	parseKey(key).server ?? roomServer() ?? mainServer()

/** Whether the person a private room is with is on its server now. */
const online = (key: string) => {
	const { server } = parseKey(key)
	return server !== null && partner(key) in (lobby.servers[server]?.users ?? {})
}

/** Whether a person on `server` is on its friends list. */
const befriended = (server: string | undefined, name: string) =>
	server !== undefined &&
	(lobby.servers[server]?.friends.friends.includes(name) ?? false)

/** Who we are on the server a room is on. */
const meIn = (key: string) => {
	const server = serverOf(key)
	return server === undefined ? null : (lobby.servers[server]?.me ?? null)
}

/** How a room reads in a tab or a heading. */
function label(key: string): string {
	if (key === BATTLE_ROOM) return 'Battle room'
	if (roomName(key) === SERVER_ROOM) return 'Server'
	return roomName(key)
}

export function Chat() {
	const [room, setRoom] = createSignal(BATTLE_ROOM)
	const [showDirectory, setShowDirectory] = createSignal(false)
	const [findPerson, setFindPerson] = createSignal('')
	const [showMembers, setShowMembers] = createSignal(false)
	/**
	 * A channel we have asked to join. The server answers a join with the
	 * channel's state rather than an acknowledgement, so the reader is taken
	 * there when it arrives — asking to join and then staying where you were is
	 * not what anyone means by it.
	 */
	const [joining, setJoining] = createSignal<string | null>(null)
	let log: HTMLDivElement | undefined

	// These read the store, so they recompute as channels and people come and go.
	const channels = createMemo(() => {
		chat.channels
		return openChannels()
	})
	const privates = createMemo(() => {
		chat.rooms
		return openPrivates()
	})
	/**
	 * The People list, by activity. Sorted apart from `privates`, which also
	 * orders the tab strip: a tab should not move under the pointer because
	 * somebody spoke.
	 */
	const recentPrivates = createMemo(() =>
		[...privates()].sort(byActivity(online)),
	)
	/**
	 * Muted people, as conversations: muting is by name, so each is opened on
	 * the server they are on now, else the one a view means by default.
	 */
	const mutedPeople = () =>
		(settings()?.chat.muted ?? []).filter(isPrivate).flatMap((room) => {
			const on =
				sessions().find(([, session]) => room.slice(1) in session.users)?.[0] ??
				mainServer()
			return on === undefined ? [] : [roomKey(on, room)]
		})
	/**
	 * Online users matching the search, on every server, friends first, capped
	 * so it stays a list.
	 */
	const people = createMemo(() => {
		const needle = findPerson().trim().toLowerCase()
		if (needle.length < 2) return []
		return sessions()
			.flatMap(([server, session]) =>
				Object.keys(session.users)
					.filter((name) => name.toLowerCase().includes(needle))
					.map((name) => ({
						server,
						name,
						friend: session.friends.friends.includes(name),
					})),
			)
			.sort(
				(a, b) =>
					Number(b.friend) - Number(a.friend) || a.name.localeCompare(b.name),
			)
			.slice(0, 30)
	})

	/**
	 * Friends on every server, as conversations: online ones first — an
	 * offline friend is not one you can talk to — and among those, whoever
	 * spoke last.
	 */
	const friends = createMemo(() =>
		sessions()
			.flatMap(([server, session]) =>
				session.friends.friends.map((name) => privateRoom(server, name)),
			)
			.sort(byActivity(online)),
	)
	/** Who is asking, and who is ignored, on every server. */
	const requests = () =>
		sessions().flatMap(([server, session]) =>
			session.friends.requests.map((name) => ({ server, name })),
		)
	const ignored = () =>
		sessions().flatMap(([server, session]) =>
			session.friends.ignored.map((name) => ({ server, name })),
		)

	/** Each server's own room: its message of the day and its broadcasts. */
	const serverRooms = createMemo(() =>
		sessions().map(([server]) => serverRoom(server)),
	)

	const rooms = createMemo(() => [
		BATTLE_ROOM,
		...serverRooms(),
		...channels(),
		...privates(),
	])

	/** Every server's channel directory, as one list. */
	const directory = () =>
		Object.entries(chat.directory).flatMap(([server, entries]) =>
			entries.map((entry) => ({
				...entry,
				server,
				key: roomKey(server, entry.name),
			})),
		)

	/**
	 * Rooms and people on show that more than one server has — a `main` on
	 * each, a `bob` on each — and so the only ones that carry their server's
	 * name. People count as `@name`, so a person and a channel never clash.
	 */
	const clashing = createMemo(() =>
		clashes([
			...[...rooms(), ...friends(), ...mutedPeople()]
				.map(parseKey)
				.map(({ server, room }) => ({ server, name: room })),
			...[...requests(), ...ignored(), ...people()].map(({ server, name }) => ({
				server,
				name: `@${name}`,
			})),
			...directory().map(({ server, name }) => ({ server, name })),
		]),
	)
	/** ` · BAR` after a room or person another server has too, else nothing. */
	const tagOn = (server: string | null, room: string) =>
		server !== null && clashing().has(room) ? ` · ${serverLabel(server)}` : ''
	const tag = (key: string) => tagOn(parseKey(key).server, roomName(key))

	/**
	 * The reader's own tab order, kept for this session only.
	 *
	 * Not persisted: where you like your conversations depends on which ones are
	 * open, and half of them are people who happened to message you today. A
	 * saved order would mostly describe a room you are no longer in.
	 */
	const [order, setOrder] = createSignal<string[]>([])

	/** What is open, in the reader's order, with new arrivals at the end. */
	const tabs = createMemo<StripTab[]>(() =>
		ordered(rooms(), order()).map((key) => ({
			key,
			label: label(key) + tag(key),
			badge: chat.unread[key],
			urgent: chat.named[key],
			quiet: quiet(key),
			// The battle room and the server are always there; a channel or a person
			// is something you opened and can close.
			closable: key !== BATTLE_ROOM && roomName(key) !== SERVER_ROOM,
			title:
				(isPrivate(key) ? `Messages with ${partner(key)}` : roomName(key)) +
				tag(key),
		})),
	)

	async function close(key: string) {
		if (isPrivate(key)) {
			// A conversation with a person is only ours; nothing to tell the server.
			closePrivate(key)
			return
		}
		await act('leave', async () => {
			const { server } = parseKey(key)
			if (server === null) return
			await api.leaveChannel(server, roomName(key))
			await rememberChannel(server, roomName(key), false)
		})
	}

	const lines = () => chat.rooms[room()] ?? []
	const members = () => chat.channels[room()]?.members ?? []
	/** Friends first, then alphabetical — the same order as everywhere else. */
	const sortedMembers = createMemo(() => {
		const server = serverOf(room())
		return [...members()].sort((a, b) => {
			const known =
				Number(befriended(server, b)) - Number(befriended(server, a))
			return known || a.localeCompare(b)
		})
	})

	// The server never announces a friendship changing, so the list is asked
	// for when this view opens.
	onMount(() => void act('friends', () => api.refreshFriends()))

	createEffect(() => {
		watchRoom(room())
	})
	createEffect(() => {
		lines().length
		log?.scrollTo({ top: log.scrollHeight })
	})

	createEffect(() => {
		const wanted = joining()
		if (wanted !== null && wanted in chat.channels) {
			setRoom(wanted)
			setJoining(null)
		}
	})

	/** A room that has gone away leaves the reader somewhere that still exists. */
	createEffect(() => {
		if (!rooms().includes(room())) setRoom(BATTLE_ROOM)
	})

	/** Every server that is up, asked for its channel directory. */
	async function listEverywhere() {
		for (const [server, session] of sessions())
			if (session.phase === 'ready') await api.listChannels(server)
	}

	async function act(what: string, run: () => Promise<void>) {
		try {
			await run()
		} catch (error) {
			pushNotice('warning', `${what}: ${describeError(error)}`)
		}
	}

	/**
	 * A command typed in the battle room goes to a server the reader did not
	 * name — the room's, else the first. Worth a line once there are several.
	 */
	function saidOn(server: string) {
		if (parseKey(room()).server === null && severalServers())
			pushSystem(room(), `on ${serverLabel(server)}`)
	}

	/**
	 * Slash commands, which is how every lobby client has spelled these since
	 * the protocol was written. `/me` is left alone: the server has its own verb
	 * for an emote, so it travels as text and is turned into `SAYEX` below.
	 */
	async function run(input: string) {
		const [word, ...rest] = input.slice(1).split(' ')
		const argument = rest.join(' ').trim()
		const command = (word ?? '').toLowerCase()

		// A command goes to the server of the room it was typed in.
		const server = serverOf(room())
		switch (command) {
			case 'join': {
				if (!argument) return pushSystem(room(), 'usage: /join <channel>')
				if (server === undefined) return pushSystem(room(), 'not logged in')
				const key = roomKey(server, argument)
				// teiserver does not answer a join for a channel you are already in,
				// so without this the command would look like it did nothing.
				if (key in chat.channels) return setRoom(key)
				setJoining(key)
				saidOn(server)
				return act('join', async () => {
					await api.joinChannel(server, argument, null)
					await rememberChannel(server, argument, true)
				})
			}
			case 'leave': {
				const target =
					argument && server !== undefined ? roomKey(server, argument) : room()
				if (
					target === BATTLE_ROOM ||
					isPrivate(target) ||
					roomName(target) === SERVER_ROOM
				)
					return pushSystem(room(), 'that is not a channel')
				return act('leave', async () => {
					const on = parseKey(target).server
					if (on === null) return
					await api.leaveChannel(on, roomName(target))
					await rememberChannel(
						parseKey(target).server,
						roomName(target),
						false,
					)
				})
			}
			case 'msg':
			case 'pm': {
				const [who, ...words] = rest
				const body = words.join(' ').trim()
				if (!who) return pushSystem(room(), 'usage: /msg <user> <message>')
				if (server === undefined) return pushSystem(room(), 'not logged in')
				// The conversation has to exist before it can be selected, or the
				// guard below sends the reader straight back to the battle room.
				saidOn(server)
				ensureRoom(privateRoom(server, who))
				setRoom(privateRoom(server, who))
				if (body) return act('message', () => api.sayPrivate(server, who, body))
				return
			}
			case 'ignore':
			case 'unignore': {
				if (!argument) return pushSystem(room(), `usage: /${command} <user>`)
				if (server === undefined) return pushSystem(room(), 'not logged in')
				return act(command, () => api.friendAction(server, command, argument))
			}
			case 'channels':
				setShowDirectory(true)
				return act('channels', listEverywhere)
			case 'me':
				// Handled by the server; fall through to sending it verbatim.
				return send(input)
			default:
				return pushSystem(room(), `no such command: /${command}`)
		}
	}

	async function send(body: string) {
		const where = room()
		if (roomName(where) === SERVER_ROOM)
			return pushSystem(where, 'nobody is listening in here')
		if (where === BATTLE_ROOM) return act('say', () => api.sayBattle(body))
		const on = parseKey(where).server
		if (on === null) return pushSystem(where, 'not logged in')
		if (isPrivate(where))
			return act('say', () => api.sayPrivate(on, partner(where), body))
		return act('say', () => api.sayChannel(on, roomName(where), body))
	}

	function submit(line: string) {
		const input = line.trim()
		if (!input) return
		if (input.startsWith('/') && !input.startsWith('/me ')) {
			void run(input)
			return
		}
		void send(input)
	}

	/**
	 * Whose names Tab may finish here: the channel's members, or the people in
	 * the room, or — in a private conversation — the one person in it.
	 */
	const nameable = () => {
		const where = room()
		if (isPrivate(where)) return [partner(where)]
		if (where === BATTLE_ROOM) return myRoom()?.members ?? []
		return members()
	}

	const title = () => {
		const where = room()
		return (isPrivate(where) ? partner(where) : label(where)) + tag(where)
	}

	return (
		<section class='chat-view'>
			<aside class='room-list'>
				<div class='room-list-head'>
					<span class='filter-label'>Rooms</span>
					<button
						class='chip-choice'
						classList={{ on: showDirectory() }}
						onClick={() => {
							setShowDirectory(!showDirectory())
							if (!showDirectory()) return
							void act('channels', listEverywhere)
						}}
					>
						Browse
					</button>
				</div>

				{/* Everyone online is already in the store, so finding someone is a
            filter rather than a request — and it beats `/msg` with an exact
            name when the name you want is `[Crd]XxStormKittyxX`. */}
				<input
					class='find-person'
					placeholder='Find someone'
					value={findPerson()}
					onInput={(e) => setFindPerson(e.currentTarget.value)}
				/>
				<Show when={findPerson().trim().length > 1}>
					<div class='room-list-head'>
						<span class='filter-label'>Matches</span>
					</div>
					<For
						each={people()}
						fallback={<p class='muted setup-empty'>Nobody by that name.</p>}
					>
						{(person) => (
							<button
								class='room-tab'
								onClick={() => {
									const key = privateRoom(person.server, person.name)
									ensureRoom(key)
									setRoom(key)
									setFindPerson('')
								}}
							>
								<span class='room-name'>
									{person.name}
									{tagOn(person.server, `@${person.name}`)}
								</span>
								<Show when={person.friend}>
									<span class='room-count'>friend</span>
								</Show>
							</button>
						)}
					</For>
				</Show>

				<Tab
					room={BATTLE_ROOM}
					label='Battle room'
					on={room() === BATTLE_ROOM}
					onClick={setRoom}
				/>
				<For each={serverRooms()}>
					{(key) => (
						<Tab
							room={key}
							label={label(key) + tag(key)}
							on={room() === key}
							onClick={setRoom}
						/>
					)}
				</For>

				{/* Channels and people are listed apart, because a channel and a
            person can carry the same name and mean different conversations. */}
				<Show when={channels().length > 0}>
					<div class='room-list-head'>
						<span class='filter-label'>Channels</span>
					</div>
					<For each={channels()}>
						{(key) => (
							<Tab
								room={key}
								label={roomName(key) + tag(key)}
								on={room() === key}
								onClick={setRoom}
							/>
						)}
					</For>
				</Show>

				<Show when={requests().length > 0}>
					<div class='room-list-head'>
						<span class='filter-label'>Wants to be friends</span>
					</div>
					<For each={requests()}>
						{({ server, name }) => (
							<div class='friend-request'>
								<span class='room-name'>
									{name}
									{tagOn(server, `@${name}`)}
								</span>
								<button
									class='chip-choice'
									title={`Accept ${name}`}
									onClick={() =>
										void act('accept', () =>
											api.friendAction(server, 'accept', name),
										)
									}
								>
									Yes
								</button>
								<button
									class='chip-choice'
									title={`Decline ${name}`}
									onClick={() =>
										void act('decline', () =>
											api.friendAction(server, 'decline', name),
										)
									}
								>
									No
								</button>
							</div>
						)}
					</For>
				</Show>

				<Show when={friends().length > 0}>
					<div class='room-list-head'>
						<span class='filter-label'>Friends</span>
					</div>
					<For each={friends()}>
						{(key) => (
							<div class='room-row'>
								<button
									class='room-tab friend'
									onClick={() => {
										ensureRoom(key)
										setRoom(key)
									}}
								>
									<span class='room-name'>{partner(key) + tag(key)}</span>
									<Show when={online(key)}>
										<span class='room-count'>online</span>
									</Show>
								</button>
								<MuteToggle room={key} label={partner(key)} />
							</div>
						)}
					</For>
				</Show>

				<Show when={ignored().length > 0}>
					<div class='room-list-head'>
						<span class='filter-label'>Ignored</span>
					</div>
					<For each={ignored()}>
						{({ server, name }) => (
							<div class='friend-request'>
								<span class='room-name muted'>
									{name}
									{tagOn(server, `@${name}`)}
								</span>
								<button
									class='chip-choice'
									title={`Stop ignoring ${name}`}
									onClick={() =>
										void act('unignore', () =>
											api.friendAction(server, 'unignore', name),
										)
									}
								>
									Undo
								</button>
							</div>
						)}
					</For>
				</Show>

				<Show when={privates().length > 0}>
					<div class='room-list-head'>
						<span class='filter-label'>People</span>
					</div>
					<For each={recentPrivates()}>
						{(key) => (
							<Tab
								room={key}
								label={partner(key) + tag(key)}
								on={room() === key}
								onClick={setRoom}
							/>
						)}
					</For>
				</Show>

				{/* Where a muted person can still be found once their conversation
            is closed. Muted channels need no such place: they stay listed. */}
				<Show when={mutedPeople().length > 0}>
					<details class='room-group'>
						<summary class='room-list-head'>
							<span class='filter-label'>Muted · {mutedPeople().length}</span>
						</summary>
						<For each={mutedPeople()}>
							{(key) => (
								<Tab
									room={key}
									label={partner(key) + tag(key)}
									on={room() === key}
									onClick={() => {
										ensureRoom(key)
										setRoom(key)
									}}
								/>
							)}
						</For>
					</details>
				</Show>

				<Show when={showDirectory()}>
					<div class='room-list-head'>
						<span class='filter-label'>All channels</span>
					</div>
					<For
						each={directory()}
						fallback={<p class='muted setup-empty'>Asking the server…</p>}
					>
						{(entry) => (
							<button
								class='room-tab'
								disabled={entry.key in chat.channels}
								onClick={() => {
									setJoining(entry.key)
									void act('join', async () => {
										await api.joinChannel(entry.server, entry.name, null)
										await rememberChannel(entry.server, entry.name, true)
									})
								}}
							>
								<span class='room-name'>
									{entry.name}
									{tagOn(entry.server, entry.name)}
								</span>
								<span class='room-count'>{entry.members}</span>
							</button>
						)}
					</For>
				</Show>
			</aside>

			<div class='chat-main'>
				{/* What is open, in the reader's order. The list on the left is for
            finding a conversation; this is for living in the ones you have. */}
				<TabStrip
					tabs={tabs()}
					active={room()}
					onSelect={setRoom}
					onClose={(key) => void close(key)}
					onReorder={setOrder}
				/>
				<header class='chat-head'>
					<h1>{title()}</h1>
					<Show when={members().length > 0}>
						<button
							class='link'
							title='Who is in this channel'
							onClick={() => setShowMembers(!showMembers())}
						>
							{members().length} here
						</button>
					</Show>
					<span class='spacer' />
					<button
						title='Keep this room out of the count on the Chat tab. A line that names you still counts.'
						onClick={() => void toggleMute(room())}
					>
						{isMuted(room()) ? 'Unmute' : 'Mute'}
					</button>
					<Show
						when={
							!isPrivate(room()) &&
							room() !== BATTLE_ROOM &&
							roomName(room()) !== SERVER_ROOM
						}
					>
						<button
							onClick={() =>
								void act('leave', async () => {
									const where = roomName(room())
									const on = parseKey(room()).server
									if (on === null) return
									await api.leaveChannel(on, where)
									await rememberChannel(on, where, false)
								})
							}
						>
							Leave
						</button>
					</Show>
				</header>

				{/* The roster is a column beside the log rather than a list above it:
            a channel with two hundred people in it would otherwise push the
            conversation off the screen. */}
				<div class='chat-body' classList={{ roster: showMembers() }}>
					<div class='chat-log' ref={log}>
						<For
							each={lines()}
							fallback={
								<p class='muted setup-empty'>
									{/* The commands need a server to answer them, so offering
                      them to somebody with no session is offering nothing. */}
									<Show
										when={anyReady()}
										fallback={
											<>
												<A href='/login'>Log in</A> to join a channel or message
												someone.
											</>
										}
									>
										Nothing here yet. <code>/join &lt;channel&gt;</code> or{' '}
										<code>/msg &lt;user&gt; …</code>
									</Show>
								</p>
							}
						>
							{(line) => (
								<Line line={line} me={meIn(room())} server={serverOf(room())} />
							)}
						</For>
					</div>

					<Show when={showMembers() && members().length > 0}>
						<Roster names={sortedMembers()} server={serverOf(room())} />
					</Show>
				</div>

				<Composer
					placeholder={
						room() === BATTLE_ROOM
							? 'Say something, or a !command'
							: 'Say something, or /join /leave /msg /ignore /channels'
					}
					names={nameable}
					onSend={submit}
				/>
			</div>
		</section>
	)
}

function Tab(props: {
	room: string
	label: string
	on: boolean
	onClick: (room: string) => void
}) {
	return (
		<div class='room-row'>
			<button
				class='room-tab'
				classList={{ on: props.on }}
				onClick={() => props.onClick(props.room)}
			>
				<span class='room-name'>{props.label}</span>
				<Show when={chat.unread[props.room]}>
					<span
						class='badge'
						classList={{
							named: chat.named[props.room],
							quiet: quiet(props.room),
						}}
					>
						{chat.unread[props.room]}
					</span>
				</Show>
			</button>
			<MuteToggle room={props.room} label={props.label} />
		</div>
	)
}

/** The bell beside a room: faint until pointed at, in full while muted. */
function MuteToggle(props: { room: string; label: string }) {
	return (
		<button
			class='room-mute'
			classList={{ on: isMuted(props.room) }}
			title={
				isMuted(props.room)
					? `Count ${props.label} on the Chat tab again`
					: `Keep ${props.label} out of the count on the Chat tab`
			}
			aria-label={`Mute ${props.label}`}
			aria-pressed={isMuted(props.room)}
			onClick={() => void toggleMute(props.room)}
		>
			<Glyph id='act-mute' />
		</button>
	)
}

function Line(props: {
	line: ChatLine
	me: string | null
	/** The server the line was said on. */
	server: string | undefined
}) {
	const mine = () => props.line.from === props.me
	return (
		<div
			class={`line ${props.line.kind}`}
			classList={{ mine: mine(), named: props.line.mention }}
		>
			<span class='at' title={new Date(props.line.at * 1000).toLocaleString()}>
				{clock(props.line.at)}
			</span>
			<span
				class='from'
				onClick={(event) =>
					// A system line's "from" names the app or the server, not a person
					// there is anything to be done about.
					props.line.kind !== 'system' &&
					props.line.kind !== 'motd' &&
					props.line.from &&
					showPlayerMenu(props.line.from, event, { server: props.server })
				}
			>
				{props.line.from}
			</span>
			<span class='text'>
				<Linkify text={props.line.text} />
			</span>
		</div>
	)
}

/** `14:07` — the hour and minute is all a backlog needs. */
function clock(at: number): string {
	if (!at) return ''
	return new Date(at * 1000).toLocaleTimeString([], {
		hour: '2-digit',
		minute: '2-digit',
	})
}

/**
 * Who is in a channel.
 *
 * `#main` holds most of the server — seventeen hundred people on a quiet
 * evening — so the rows are virtualised like the battle list. Drawing them all
 * does not merely cost time: the column grows to their full height and pushes
 * the conversation off the screen.
 */
function Roster(props: { names: string[]; server: string | undefined }) {
	const session = () =>
		props.server === undefined ? undefined : lobby.servers[props.server]
	let scrollRef: HTMLElement | undefined

	const virtualizer = createVirtualizer({
		get count() {
			return props.names.length
		},
		getScrollElement: () => scrollRef ?? null,
		estimateSize: () => ROSTER_ROW,
		overscan: 12,
	})

	return (
		<aside class='chat-roster' ref={scrollRef}>
			<div
				style={{
					height: `${virtualizer.getTotalSize()}px`,
					position: 'relative',
					width: '100%',
				}}
			>
				<For each={virtualizer.getVirtualItems()}>
					{(item) => {
						const name = () => props.names[item.index]
						return (
							<Show when={name()}>
								{(who) => (
									<button
										class='pname'
										classList={{
											me: who() === session()?.me,
											friend: befriended(props.server, who()),
										}}
										style={{
											position: 'absolute',
											top: `${item.start}px`,
											height: `${ROSTER_ROW}px`,
											width: '100%',
										}}
										onClick={(event) =>
											showPlayerMenu(who(), event, { server: props.server })
										}
									>
										{who()}
									</button>
								)}
							</Show>
						)
					}}
				</For>
			</div>
		</aside>
	)
}
