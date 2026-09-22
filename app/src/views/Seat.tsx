import { useNavigate } from '@solidjs/router'
import { Select } from '../components/Select'
import {
	For,
	Show,
	createEffect,
	createMemo,
	createSignal,
	onCleanup,
} from 'solid-js'
import { SideGlyph, SideIcon } from '../components/icons'
import type { AiChoice } from '../ipc/bindings/AiChoice'
import { api, describeError } from '../ipc/client'
import { DEFAULT_TEAMS, freeTeam, unusedBotName } from '../lib/roster'
import { pushNotice } from '../store/chat'
import { roomServer } from '../store/lobby'
import { applySettings, settings } from '../store/settings'
import { useRoom, type RoomModel } from './room/model'
import { posture, type Segment } from './room/posture'
import { setBonus as sendBonus } from './room/move'

/** The factions, then side 2, Random, which is none of them. */
const SIDES = [
	{ id: 0, label: 'Armada' },
	{ id: 1, label: 'Cortex' },
	{ id: 3, label: 'Legion' },
	{ id: 2, label: 'Random' },
]

/** The lowest team number nobody else in the room holds. */
function nextTeam(room: RoomModel): number {
	const battle = room.battle()
	return battle ? freeTeam(battle, room.users(), room.me()) : 0
}

/**
 * Turns the local network on, because somebody asked for it by pressing the
 * button that needs it. Already on is not an error and writes nothing.
 *
 * The setting stays the way to turn it *off* -- this only ever says yes, so
 * a press cannot quietly undo a deliberate no somewhere else.
 */
async function enableLan(): Promise<void> {
	const held = settings()
	if (!held || held.lan.enabled) return
	applySettings(
		await api.updateSettings({ ...held, lan: { ...held.lan, enabled: true } }),
	)
}

/** What `remember` remembers, kept current by what you actually do. */
async function remember(played: boolean) {
	try {
		applySettings(await api.rememberPlayed(played))
	} catch {
		// A preference we could not write is not worth interrupting a game for.
	}
}

/**
 * Sits on ally team `ally` — joining it, or moving there from another — and
 * makes playing what `remember` remembers. Sitting down from watching starts
 * unready; a move between sides keeps ready, since the game agreed to is the
 * same one. `ready` asks for a ready to follow the seat: the server clears
 * one sent with it, so the runtime sends it once the seat is answered.
 */
export async function sitOn(
	room: RoomModel,
	ally: number,
	ready = false,
): Promise<void> {
	await room.io.takeSeat(nextTeam(room), ally, ready)
	await remember(true)
}

/**
 * Playing rather than watching.
 *
 * Sitting down is what a lobby is for, so the seats are simply here. The
 * setting behind them exists for a client with nobody at the keyboard, and a
 * room of your own — passworded, or one SPADS says you boss — never consults
 * it at all.
 */
export function Seat() {
	const [busy, setBusy] = createSignal(false)
	const navigate = useNavigate()

	const room = useRoom()
	const battleOf = createMemo(room.battle)
	const me = createMemo(() => {
		const name = room.me()
		return name === null ? undefined : room.users()[name]
	})
	const seat = () => me()?.battleStatus
	/** Our own newest word on the seat, ahead of the server's. */
	const wished = () => room.my()?.seatOnItsWay ?? null
	const seated = () => wished()?.player ?? seat()?.player ?? false
	const running = () => room.running() !== null
	/** The three postures as drawn; see `posture`. */
	const p = createMemo(() => posture(room))
	const held = (segment: Segment) => segment.look === 'held'
	const heldUntil = createMemo(() => p().heldUntil)
	/**
	 * When the flood window began holding our status, by our clock. The
	 * hairline drains from there to `heldUntil`, and keeps its place when the
	 * pending segment changes under it.
	 */
	const heldSince = createMemo<number | null>(
		(since) => (heldUntil() === null ? null : (since ?? Date.now())),
		null,
	)
	const heldStyle = createMemo(() => {
		const since = heldSince()
		const until = heldUntil()
		if (since === null || until === null) return undefined
		return {
			'--held-total': `${Math.max(until - since, 1)}ms`,
			'--held-elapsed': `${Date.now() - since}ms`,
		}
	})
	/** A pending segment's hairline, while the window holds the press. */
	const hold = (segment: Segment) =>
		segment.pending && heldUntil() !== null ? heldStyle() : undefined
	/**
	 * Our place in the join queue, from one, or null while not in it. A full
	 * room answers Join by keeping us a spectator and queueing us itself, so
	 * this is what the button has to read to say what happened.
	 */
	const queued = createMemo((): number | null => {
		const name = room.me()
		const queue = battleOf()?.queue ?? []
		const index = name === null ? -1 : queue.indexOf(name)
		return index < 0 || seated() ? null : index + 1
	})

	/**
	 * Ally teams already in use, plus the next free one — you can join a side or
	 * open a new one, and nothing else would mean anything.
	 */
	/** Ally teams somebody is already sitting on. */
	const usedAllies = createMemo(() => {
		const battle = battleOf()
		const used = new Set<number>()
		if (!battle) return used
		for (const name of battle.members) {
			const status = room.users()[name]?.battleStatus
			if (status?.player) used.add(status.allyTeam)
		}
		for (const bot of battle.bots) used.add(bot.status.allyTeam)
		return used
	})

	/**
	 * The teams the roster draws: the first `DEFAULT_TEAMS` always, and any
	 * somebody sits on. One of these is joined even while empty, since it is
	 * already on screen; only a team past them is new.
	 */
	const drawnAllies = createMemo(
		() =>
			new Set([
				...Array.from({ length: DEFAULT_TEAMS }, (_, ally) => ally),
				...usedAllies(),
			]),
	)

	const allyTeams = createMemo(() => {
		const battle = battleOf()
		if (!battle) return [0]
		const used = usedAllies()
		const highest = used.size === 0 ? -1 : Math.max(...used)
		// Every team the room draws, and then one more to open a new one. Gaps
		// are offered as well: an empty team between two full ones is a seat you
		// can take, not a hole in the list -- and the room is already drawing it.
		const drawn = Math.max(
			battle.layout?.teams ?? 0,
			highest + 1,
			DEFAULT_TEAMS,
		)
		return Array.from({ length: drawn + 1 }, (_, ally) => ally)
	})

	/**
	 * The ally team a seat would join.
	 *
	 * Against people, the emptiest side somebody is already on. Against AIs
	 * the sides are not alike: the usual room is one team of people against
	 * one of AIs, so the place to go is wherever the most people are -- and,
	 * before anybody has sat down, wherever the AIs are not.
	 */
	function freeAlly(): number {
		const battle = battleOf()
		if (!battle) return 0
		const people = new Map<number, number>()
		for (const name of battle.members) {
			const status = room.users()[name]?.battleStatus
			if (status?.player)
				people.set(status.allyTeam, (people.get(status.allyTeam) ?? 0) + 1)
		}
		const count = (ally: number) => people.get(ally) ?? 0
		const teams = allyTeams()
		const peopled = teams.filter((ally) => people.has(ally))
		const aiSides = new Set(battle.bots.map((bot) => bot.status.allyTeam))

		if (aiSides.size > 0) {
			if (peopled.length > 0)
				return peopled.reduce((best, ally) =>
					count(ally) > count(best) ? ally : best,
				)
			return teams.find((ally) => !aiSides.has(ally)) ?? teams[0] ?? 0
		}
		// The empty sides are all equally new; prefer one somebody is on.
		if (peopled.length === 0) return teams[0] ?? 0
		return peopled.reduce((best, ally) =>
			count(ally) < count(best) ? ally : best,
		)
	}

	/**
	 * The lowest ally team nobody sits on -- where an opponent goes. The list
	 * always ends in one more than the highest team held, so there is one.
	 */
	const emptyAlly = () =>
		allyTeams().find((ally) => !usedAllies().has(ally)) ?? 0

	/**
	 * Sits down on arrival when that is the posture, once per room.
	 *
	 * Once, decided on arrival whichever way: leaving your seat is not to be
	 * undone, and a seat you took yourself is not to be taken again behind
	 * you. Both change what `remember` remembers, and the next room agrees
	 * with what you just did.
	 *
	 * The team is picked from the members known at that moment, which on a busy
	 * room may be a moment before the last of them has arrived. Two people can
	 * therefore land on one team number, exactly as they can when a person
	 * clicks the button the instant they walk in — and it matters as little,
	 * because SPADS assigns teams itself when the game starts.
	 */
	let seatedIn: number | undefined
	createEffect(() => {
		const battle = battleOf()
		const play = settings()?.play
		if (!battle || !play || !room.caps.plays) return
		if (seatedIn === battle.id) return
		seatedIn = battle.id
		if (seated()) return
		const wanted =
			play.joinAs === 'remember' ? play.lastWasPlayer : play.joinAs === 'player'
		if (!wanted) return
		void act('take a seat', () => room.io.takeSeat(nextTeam(room), freeAlly()))
	})

	/** Runs one action, telling the user why it did not happen; true if it did. */
	async function act(what: string, run: () => Promise<void>): Promise<boolean> {
		setBusy(true)
		try {
			await run()
			return true
		} catch (error) {
			pushNotice('warning', `${what}: ${describeError(error)}`)
			return false
		} finally {
			setBusy(false)
		}
	}

	/** What the seat picker shows: the side we hold, or nothing while watching. */
	const current = () =>
		seated() ? String(wished()?.allyTeam ?? seat()?.allyTeam ?? 0) : ''

	/**
	 * Sits or moves as picked. A refused pick snaps the picker back, since the
	 * row it landed on never came true.
	 */
	async function pickSeat(picker: HTMLSelectElement) {
		const choice = Number(picker.value)
		const done = await act('take a seat', () => sitOn(room, choice))
		if (!done) picker.value = current()
	}

	/** Stands up. The one thing a seat holder does that is not about where. */
	const spectate = () =>
		act('spectate', async () => {
			await room.io.releaseSeat()
			await remember(false)
		})

	// ---- the posture control -------------------------------------------
	// Pressing a segment says "this is the posture I want". A lower one steps
	// back to it; Ready also toggles off on a second press, as in Chobby.

	const watching = () => !seated() && queued() === null
	const armed = () => room.my()?.preReady ?? false

	function watch() {
		if (queued() !== null)
			return act('leave the queue', () => room.io.sayBattle('$leaveq'))
		if (seated()) return spectate()
	}

	function play() {
		if (watching()) return act('take a seat', () => sitOn(room, freeAlly()))
		if (held(p().ready)) return act('ready', () => room.io.setReady(false))
	}

	function ready() {
		// One press for "I'm in": the seat, and a ready once it is answered.
		if (watching())
			return act('take a seat', () => sitOn(room, freeAlly(), true))
		// Nothing to ready right now, so a ready for when there is.
		if (queued() !== null || running())
			return act('ready in advance', () => room.io.setPreReady(!armed()))
		return act('ready', () => room.io.setReady(!held(p().ready)))
	}

	/** A press the server has not answered: on its way, or held a moment. */
	const pendingTitle = () =>
		heldUntil() === null
			? 'On its way to the server'
			: 'Held a moment: the room takes five changes in eight seconds'
	const watchTitle = () =>
		p().watch.pending
			? pendingTitle()
			: queued() !== null
				? 'Leave the queue and keep watching'
				: seated()
					? 'Give the seat up'
					: 'Watching'
	const playTitle = () =>
		p().play.pending
			? pendingTitle()
			: queued() !== null
				? 'Waiting for a seat'
				: watching()
					? 'Take a seat on the emptiest team'
					: held(p().ready)
						? 'Step back to not ready'
						: 'Playing'
	const readyTitle = () => {
		const segment = p().ready
		if (segment.pending) return pendingTitle()
		if (watching()) return 'Take a seat and ready up'
		if (queued() !== null)
			return armed()
				? 'Armed: ready the moment the queue seats you, this once. Press to take it back'
				: 'Ready the moment the queue seats you, this once'
		if (running())
			return armed()
				? 'Armed: ready again once this game ends. Press to take it back'
				: 'Ready again once this game ends'
		if (segment.waiting) return 'Everyone else is ready'
		return held(segment) ? 'Ready. Press to unready' : 'Not ready'
	}

	return (
		<div class='seat'>
			{/* A room whose game cannot be played here is one to watch and talk in:
          every control below ends, sooner or later, in the engine being
          started on somebody else's game. */}
			<Show
				when={room.caps.plays}
				fallback={
					<span class='muted'>
						Spectating. This build has no engine that may play on the server.
					</span>
				}
			>
				{/* The three postures, read left to right as commitment: which you
				    hold, and which is the step after it. Ready keeps its label and
				    changes colour, as Chobby's does: yellow while the room asks it,
				    green once given, dashed while armed for later. */}
				<div class='choice posture' role='group' aria-label='Posture'>
					<button
						type='button'
						class='watch'
						classList={{
							on: held(p().watch),
							pending: p().watch.pending,
							held: hold(p().watch) !== undefined,
						}}
						style={hold(p().watch)}
						aria-pressed={held(p().watch)}
						disabled={busy()}
						title={watchTitle()}
						onClick={() => void watch()}
					>
						Watch
					</button>
					<button
						type='button'
						class='play'
						classList={{
							on: held(p().play),
							next: p().play.look === 'next',
							pending: p().play.pending,
							held: hold(p().play) !== undefined,
						}}
						style={hold(p().play)}
						aria-pressed={held(p().play)}
						disabled={busy()}
						title={playTitle()}
						onClick={() => void play()}
					>
						Play
						<Show when={p().play.note}>
							{(note) => <span class='posture-note'>{note()}</span>}
						</Show>
					</button>
					{/* Ready is a thing you say to somebody; a room nobody waits on
					    has no such segment. */}
					<Show when={room.caps.ready}>
						<button
							type='button'
							class='ready posture-ready'
							classList={{
								on: held(p().ready),
								asked: p().ready.look === 'asked',
								armed: p().ready.look === 'armed',
								pending: p().ready.pending,
								held: hold(p().ready) !== undefined,
								waiting: p().ready.waiting,
							}}
							style={hold(p().ready)}
							aria-pressed={held(p().ready)}
							disabled={busy()}
							title={readyTitle()}
							onClick={() => void ready()}
						>
							Ready
							<Show when={p().ready.note}>
								{(note) => <span class='posture-note'>{note()}</span>}
							</Show>
						</button>
					</Show>
				</div>

				{/* Where to sit. The picker stays whether you are seated or
				    watching, so nothing moves. */}
				<Select
					value={current()}
					disabled={busy()}
					onChange={(e) => void pickSeat(e.currentTarget)}
				>
					<Show when={!seated()}>
						<option value='' disabled hidden>
							Team
						</option>
					</Show>
					<For each={allyTeams()}>
						{(ally) => (
							<option value={String(ally)}>
								{current() === String(ally)
									? `Team ${ally + 1}`
									: drawnAllies().has(ally)
										? `Join team ${ally + 1}`
										: `New team ${ally + 1}`}
							</option>
						)}
					</For>
				</Select>

				<Show when={seated()}>
					{/* The chosen faction's mark is drawn over the picker's value, so the
					    closed box shows it whether or not the list itself can. */}
					<span class='select-iconed'>
						<SideIcon side={seat()?.side ?? 0} />
						<Select
							class='rich'
							value={String(seat()?.side ?? 0)}
							disabled={busy()}
							onChange={(e) =>
								act('faction', () =>
									room.io.setSide(Number(e.currentTarget.value)),
								)
							}
						>
							<For each={SIDES}>
								{(side) => (
									<option value={String(side.id)}>
										<SideGlyph side={side.id} />
										{side.label}
									</option>
								)}
							</For>
						</Select>
					</span>
				</Show>

				<AddAi
					busy={busy()}
					act={act}
					freeTeam={() => nextTeam(room)}
					emptyAlly={emptyAlly}
					allyTeams={allyTeams}
				/>
			</Show>

			<span class='spacer' />
			{/* The skirmish is a game set up and not yet played; this is that
          same setup with a door in it. Kept off the online room's Host
          buttons on purpose: somebody who only plays on the server should
          never have to decide what a LAN is. */}
			<Show when={room.caps.opensToLan}>
				<button
					disabled={busy()}
					title='Open this game to people on your network'
					onClick={() =>
						act('host on the LAN', async () => {
							// Turning it on is the point of pressing this: the
							// switch in Settings is for turning it back off.
							await enableLan()
							navigate('/lan/host')
						})
					}
				>
					Host on LAN
				</button>
			</Show>
			{/* Both halves of Chobby's Host button: an empty autohost is a listed
          room you boss, `!privatehost` is a passworded one made on request.
          Chobby asks for a region; the runtime measures instead, and says
          which room it chose and how far away it is. Both are rooms on the
          server, so neither is offered where there is no server. */}
			<Show when={room.caps.spads}>
				<button
					disabled={busy()}
					onClick={() =>
						act('host a room', async () => {
							// Already standing in a room: the view swaps to the new one as
							// soon as the host lets us in. Said out loud because until then
							// the old room is still on screen and nothing looks to happen.
							const server = roomServer()
							if (server === undefined) throw new Error('not in a room')
							await api.hostPublic(server)
							pushNotice(
								'info',
								'took a room; opening it when the host answers',
							)
						})
					}
				>
					Host a public room
				</button>
				<button
					disabled={busy()}
					onClick={() =>
						act('host a room', async () => {
							const server = roomServer()
							if (server === undefined) throw new Error('not in a room')
							const manager = await api.requestPrivateHost(server)
							pushNotice(
								'info',
								`asked ${manager} for a private room; joining when it opens`,
							)
						})
					}
				>
					Private room
				</button>
			</Show>
		</div>
	)
}

/**
 * Asking for an AI from somewhere other than the seat bar: a team's own menu
 * asks for one on that team.
 *
 * A signal rather than a prop because the sheet lives inside the seat bar and
 * the team headers are three components away, with nothing else to say to
 * each other -- the same arrangement `PlayerMenu` uses for the row menus.
 */
const [addTo, setAddTo] = createSignal<number | null>(null)
const [offered, setOffered] = createSignal(false)

/** Whether there is an Add AI sheet on screen to open at all. */
export const canAddAi = offered

/** Opens it, set to `allyTeam`. */
export function showAddAi(allyTeam: number): void {
	setAddTo(allyTeam)
}

/** Colours the engine can tell apart at a glance, as 0xBBGGRR. */
const BOT_COLOURS = [0x4b73f2, 0x3fd07f, 0x2fb8f0, 0x9e5ce8, 0x50a0ff, 0x8fd04b]

/**
 * An AI for the room.
 *
 * The AI runs on this machine when the game starts, which is why the choices
 * are what is installed here: the AIs the engine ships, then the ones the
 * room's game implements in Lua — Scavengers and Raptors, for BAR. There is
 * nothing to offer until that list has been read. Whether the room takes it
 * is the host's call — SPADS answers a refusal in chat, where it can be seen.
 *
 * A Lua AI is a game mode rather than an opponent: the room holds one, and a
 * bonus means nothing to it, so neither is asked for.
 */
function AddAi(props: {
	busy: boolean
	act: (what: string, run: () => Promise<void>) => Promise<boolean>
	freeTeam: () => number
	/** The next team nobody is on: the sheet starts there, an opponent's place. */
	emptyAlly: () => number
	/** The ally teams this room draws, plus the next one that could be opened. */
	allyTeams: () => number[]
}) {
	const room = useRoom()
	const [ais, setAis] = createSignal<AiChoice[]>([])
	const [ai, setAi] = createSignal('')
	const [open, setOpen] = createSignal(false)
	const [count, setCount] = createSignal(1)
	const [bonus, setBonus] = createSignal(0)
	const [ally, setAlly] = createSignal<number | null>(null)

	createEffect(() => {
		// The room's game, whose Lua AIs are part of the offer.
		const name = room.battle()?.gameName
		if (name === undefined) return
		api
			.gameAis(name)
			.then((choices) => {
				setAis(choices)
				const first = choices[0]
				if (!ai() && first) setAi(first.name)
			})
			// No data directory means no AIs to run; the control just stays away.
			.catch(() => setAis([]))
	})

	// What the team menus have to know: whether asking for one would open
	// anything. False again the moment the room -- and this -- goes away.
	createEffect(() => setOffered(ais().length > 0))
	onCleanup(() => setOffered(false))

	createEffect(() => {
		const wanted = addTo()
		if (wanted === null) return
		setAlly(wanted)
		setOpen(true)
		setAddTo(null)
	})

	const chosen = () => ais().find((choice) => choice.name === ai())
	const gameMode = () => chosen()?.lua ?? false
	/** The team the sheet is set to, until picked the next empty one. */
	const team = () => ally() ?? props.emptyAlly()

	const colour = () =>
		BOT_COLOURS[Math.floor(Math.random() * BOT_COLOURS.length)] as number

	async function add() {
		const claimed = new Set<string>()
		const wanted = gameMode() ? 0 : bonus()
		const many = gameMode() ? 1 : count()
		for (let n = 0; n < many; n++) {
			const name = unusedBotName(room.battle(), ai(), claimed)
			claimed.add(name)
			const seat = props.freeTeam() + n
			const tint = colour()
			await room.io.addBot(name, ai(), seat, team(), tint)
			// `ADDBOT` carries no bonus, so one is a second message.
			if (wanted > 0)
				await sendBonus(
					room,
					{
						kind: 'bot',
						name,
						mine: true,
						team: seat,
						handicap: 0,
						colour: tint,
					},
					wanted,
					team(),
				)
		}
	}

	return (
		<Show when={ais().length > 0}>
			<button
				disabled={props.busy}
				title='The AI plays from this machine'
				onClick={() => setOpen(true)}
			>
				Add AI
			</button>

			<Show when={open()}>
				<div class='sheet' onMouseDown={() => setOpen(false)}>
					<form
						class='sheet-card add-ai'
						onMouseDown={(event) => event.stopPropagation()}
						onSubmit={(event) => {
							event.preventDefault()
							setOpen(false)
							void props.act('add an AI', add)
						}}
					>
						<h2>Add AI</h2>
						<label>
							Which
							<Select
								value={ai()}
								onChange={(e) => setAi(e.currentTarget.value)}
							>
								<For each={ais()}>
									{(choice) => (
										<option value={choice.name}>{choice.name}</option>
									)}
								</For>
							</Select>
						</label>
						<label>
							Team
							<Select
								value={String(team())}
								onChange={(e) => setAlly(Number(e.currentTarget.value))}
							>
								<For each={props.allyTeams()}>
									{(one) => <option value={String(one)}>Team {one + 1}</option>}
								</For>
							</Select>
						</label>
						<Show when={!gameMode()}>
							<label>
								How many
								<input
									type='number'
									min={1}
									max={16}
									value={count()}
									onInput={(e) => {
										const many = Number(e.currentTarget.value)
										if (Number.isFinite(many))
											setCount(Math.max(1, Math.min(16, Math.round(many))))
									}}
								/>
							</label>
							<label>
								Bonus
								<input
									type='range'
									min={0}
									max={100}
									step={5}
									value={bonus()}
									onInput={(e) => setBonus(Number(e.currentTarget.value))}
								/>
								<output>{bonus()}%</output>
							</label>
						</Show>
						<p class='muted'>
							<Show when={!gameMode()}>
								A resource bonus, the same one a host gives with{' '}
								<code>!force … bonus</code>.{' '}
							</Show>
							Every AI added here runs on this machine when the game starts.
						</p>
						<div class='sheet-actions'>
							<button type='button' onClick={() => setOpen(false)}>
								Cancel
							</button>
							<button type='submit' class='primary' disabled={!ai()}>
								Add
							</button>
						</div>
					</form>
				</div>
			</Show>
		</Show>
	)
}
