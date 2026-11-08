import {
	For,
	type JSX,
	Show,
	createEffect,
	createMemo,
	createSignal,
	on,
	onCleanup,
} from 'solid-js'
import { FigurePair } from '../components/FigurePair'
import { Thinking } from '../components/Thinking'
import type { BotView } from '../ipc/bindings/BotView'
import type { Score } from '../ipc/bindings/Score'
import { describeError } from '../ipc/client'
import { age } from '../lib/age'
import { asker } from '../lib/asking'
import { challengePair, chance, percentRank, tenths } from '../lib/challenge'
import { isVsAi } from '../lib/battles'
import { NOTHING_YET, TOP, declinedText, headline, miniature } from '../lib/pve'
import { askDelay } from '../lib/stagger'
import {
	playerHandicap,
	reseated,
	type Seats,
	uncertainSizes,
} from '../lib/teamSize'
import { settings } from '../store/settings'
import { PveStats } from './PveStats'
import { useRoom, type RoomModel } from './room/model'

/**
 * What the pve.bar stats service says this room scores.
 *
 * The same numbers the in-game widget shows, asked for while there is still
 * time to change the answer — which is when "this is going to be a massacre"
 * is worth knowing.
 *
 * Nothing is asked for a room that is not PvE. The room's names go along so
 * the full panel can show each person's own record; the estimate itself is
 * for a generic team and does not use who is present.
 */
/**
 * How long the room has to have been still for a change to be asked about at
 * once. A change inside a run of them waits until the run has been still
 * this long: a host applying a preset changes a hundred settings in a couple
 * of minutes, each arriving as its own script tag, and only the end of that
 * is worth somebody else's service answering.
 */
export const QUIET_FOR = 2000
/** The least time between two asks from this client, whatever the room does. */
export const AT_MOST_EVERY = 1000
/**
 * How long a fresh ask leaves the last figures up. Most come back well inside
 * this, the memo's at once, and figures blinking to dots and back on every
 * arrival in the room say nothing. A cold start does outlast it, and then the
 * figures are worth taking down.
 */
export const DOTS_AFTER = 1000

/** The battle status of every member seated as a player, spectators left out. */
function seatedStatuses(room: RoomModel) {
	return (room.battle()?.members ?? []).flatMap((name) => {
		const status = room.users()[name]?.battleStatus
		return status?.player ? [status] : []
	})
}

/**
 * Everything the answer depends on except how many sit in the room and at
 * what handicap, as one string: what the per-size table cannot answer for. The
 * room's id comes first, which is how a new room is told from a changed one.
 *
 * Rust builds the request from the room; this only has to notice when any of
 * that changes. Reading each value here is what subscribes to it — the store
 * mutates the settings object in place, so watching the object itself would
 * never fire again after the first look.
 */
function setup(room: RoomModel): string | undefined {
	const my = room.my()
	const battle = room.battle()
	if (!my || !battle) return undefined
	const settings = Object.entries(my.scriptTags)
		.filter(([key]) => key.startsWith('game/modoptions/'))
		.map(([key, value]) => `${key}=${value}`)
		.sort()
	const bots = battle.bots.map(
		(bot) => `${bot.ai}@${bot.status.handicap} on ${bot.status.allyTeam}`,
	)
	// News of its own when read off the title: that lands as the join replay
	// ends, with nothing else in here moving.
	const started = my.gameRoster?.bots.map((bot) => bot.ai) ?? []
	// Which ally teams the humans are on, and the boxes each may start in,
	// decide the room's start-box geometry; the layout and the player limit,
	// its seat count.
	const humanTeams = [
		...new Set(seatedStatuses(room).map((status) => status.allyTeam)),
	].sort((a, b) => a - b)
	const boxes = battle.startRects
		.map(
			(rect) =>
				`box ${rect.allyTeam}: ${rect.left},${rect.top},${rect.right},${rect.bottom}`,
		)
		.sort()
	const layout = battle.layout
		? `${battle.layout.teams}x${battle.layout.teamSize}`
		: 'no layout'
	return [
		my.id,
		battle.mapName,
		`${layout}, at most ${battle.maxPlayers}`,
		`humans on ${humanTeams.join(',')}`,
		...bots,
		...started,
		...boxes,
		...settings,
	].join('\n')
}

/**
 * Who sits in the room, as Rust tells the service: counted off the seats,
 * which is the count the host announces one round trip later.
 */
function seats(room: RoomModel): Seats {
	const seated = seatedStatuses(room)
	return {
		players: seated.length,
		handicap: playerHandicap(seated.map((status) => status.handicap)),
	}
}

/**
 * Who the service is told is here, as Rust sends `player_names`: everyone but
 * the room's bots, sorted. The answer carries a record for each of them.
 */
function present(room: RoomModel): string[] {
	const users = room.users()
	return (room.battle()?.members ?? [])
		.filter((name) => !users[name]?.status.bot)
		.sort()
}

/** Whether two sorted lists of names are the same people. */
function samePeople(a: readonly string[], b: readonly string[]): boolean {
	return a.length === b.length && a.every((name, at) => name === b[at])
}

/**
 * Everything the answer depends on, as one string: the setup, then who is
 * here and where each sits. Any change to it is a reason to look again;
 * whether it is a reason to ask is for `needed` to say.
 */
function fingerprint(room: RoomModel): string | undefined {
	const base = setup(room)
	const battle = room.battle()
	if (base === undefined || !battle) return undefined
	const members = battle.members.map((name) => {
		const status = room.users()[name]?.battleStatus
		return status?.player
			? `${name}: ${status.handicap} on ${status.allyTeam}`
			: name
	})
	return [base, ...members].join('\n')
}

/** Whether these AIs are a PvE opponent. */
function namesOpponent(bots: readonly BotView[]): boolean {
	const names = bots.map((bot) => bot.ai.toLowerCase()).join(' ')
	const raptors = names.includes('raptor')
	const scavengers = names.includes('scavenger')
	// Both at once is not a setup the model knows, so it is not asked about.
	if (raptors && scavengers) return false
	return raptors || scavengers || names.includes('barb')
}

/**
 * Whether this room has a PvE opponent in it at all, or the game running in it
 * started with one: a leaver's AIs leave the room and play on.
 *
 * Decided here as well as in Rust, which stays authoritative for what is
 * sent. This one only decides whether to ask — without it a room with no AI
 * in it flashes "PvE" and a row of dots before the answer comes back null,
 * and every settings change in every ordinary room costs a round trip to
 * find out nothing.
 */
function isPve(room: RoomModel): boolean {
	return (
		namesOpponent(room.battle()?.bots ?? []) ||
		namesOpponent(room.my()?.gameRoster?.bots ?? [])
	)
}

/**
 * Whether the room has the row. A room's AIs come last in what the server says
 * on entering, and a row arriving with them would push the whole room down, so
 * until the server has finished the title speaks for them. It only ever draws:
 * asking waits for the AIs, which are what Rust reads.
 */
function hasRow(room: RoomModel): boolean {
	if (isPve(room)) return true
	const battle = room.battle()
	const my = room.my()
	if (!battle || !my || my.settled) return false
	return isVsAi(battle)
}

/**
 * Whether the setting allows asking at all. Rust answers `null` when it is
 * off, but that is a round trip and a row of dots for nothing; decided here
 * too so an off switch shows nothing and sends nothing. On until the settings
 * have arrived, which is the file's default.
 */
function enabled(): boolean {
	return settings()?.play.pveStats ?? true
}

/** This client's place in the room's asking order, as a wait. */
function myStagger(room: RoomModel): number {
	return askDelay({
		me: room.me(),
		members: room.battle()?.members ?? [],
		boss: room.my()?.boss ?? null,
		users: room.users(),
	})
}

/**
 * The histogram the button opens, at icon size: games bars, and a yellow tick
 * where this setup sits. The tick is a fixed two screen pixels however the
 * icon stretches, which keeps it crisp and keeps it visible over the bars.
 *
 * With no answer to draw (the service failed), the same bars at zero.
 */
function Miniature(props: { score: Score | undefined }) {
	const drawn = createMemo(() =>
		// Marked where the row's full-colour figure is: every seat taken.
		props.score?.histogram.length
			? miniature(headline(props.score))
			: NOTHING_YET,
	)
	return (
		<svg
			class='pve-miniature'
			viewBox={`0 0 ${TOP} 1`}
			preserveAspectRatio='none'
			aria-hidden='true'
		>
			<For each={drawn().bars}>
				{(bar) => (
					<rect
						x={bar.lo + 0.25}
						y={1 - Math.max(0.05, bar.height)}
						width={Math.max(0, bar.hi - bar.lo - 0.5)}
						height={Math.max(0.05, bar.height)}
					/>
				)}
			</For>
			<Show when={drawn().at}>
				{(at) => (
					<line
						class='at'
						x1={at()}
						x2={at()}
						y1={0}
						y2={1}
						vector-effect='non-scaling-stroke'
					/>
				)}
			</Show>
		</svg>
	)
}

export function PveScore() {
	const room = useRoom()
	/** `undefined` before any answer; `null` when Rust declined to ask. */
	const [score, setScore] = createSignal<Score | null | undefined>(undefined)
	/** The room's `setup` when the answer in `score` was asked for. */
	const [askedSetup, setAskedSetup] = createSignal<string | undefined>()
	/** Who was `present` when the answer in `score` was asked for. */
	const [askedPresent, setAskedPresent] = createSignal<string[] | undefined>()
	/** An ask has been out longer than `DOTS_AFTER`. */
	const [overdue, setOverdue] = createSignal(false)
	const [failure, setFailure] = createSignal<string | null>(null)
	/** Whether the full panel is open over the room. */
	const [open, setOpen] = createSignal(false)

	/**
	 * The answer as it reads for the room right now: the answer itself while
	 * the room is as it was asked about, that size's figures from its table
	 * when only the number of players moved, and `null` when the table cannot
	 * say — the setup or the handicap changed — and the room needs asking.
	 */
	const current = createMemo(() => {
		const held = score()
		if (!held || askedSetup() !== setup(room)) return null
		return reseated(held, seats(room))
	})
	/** What the row shows: the current answer, else the last one until the next. */
	const shown = () => current() ?? score()

	/**
	 * No answer yet, or a fresh one long on its way for a room the last answer
	 * cannot speak for.
	 */
	const waiting = () =>
		(overdue() && current() === null) || score() === undefined

	/**
	 * Whether the room needs asking about: the answer held falls short of it.
	 * A head count its table covers does not; a failed ask does. Someone
	 * arriving or leaving does only while the full panel is open: the records
	 * there are the one thing who is here changes.
	 */
	const needed = () =>
		enabled() &&
		(current() === null ||
			failure() !== null ||
			(open() && !samePeople(askedPresent() ?? [], present(room))))

	/**
	 * Paced, because the service is a Lambda with a concurrency of one and a
	 * twenty-second cold start, and everyone in the room sees the same change
	 * at the same moment. One ask out at a time, at once after a quiet spell
	 * but once at the end of a run of changes, a floor between asks, and a
	 * wait by seat rank so the room's clients arrive one after another rather
	 * than all at once. Rust reads the room afresh for each ask, so a single
	 * follow-up covers whatever changed while one was out.
	 */
	const asks = asker(
		async () => {
			const asked = room.my()?.id
			const askedFor = setup(room)
			const askedWho = present(room)
			const due = setTimeout(() => setOverdue(true), DOTS_AFTER)
			try {
				const answer = await room.io.pveScore()
				// A room since left is not this one, and a cold start outlasts a visit.
				if (room.my()?.id !== asked) return
				setScore(answer)
				setAskedSetup(askedFor)
				setAskedPresent(askedWho)
				setFailure(null)
			} catch (err) {
				console.warn('pve stats:', err)
				if (room.my()?.id !== asked) return
				setFailure(describeError(err))
			} finally {
				clearTimeout(due)
				setOverdue(false)
			}
		},
		{
			quiet: QUIET_FOR,
			floor: AT_MOST_EVERY,
			stagger: () => myStagger(room),
		},
		needed,
	)

	/** Asked as soon as a room is in view, then as it changes; `asks` paces it. */
	// A memo, so a change that leaves the string as it was — someone toggling
	// ready, say — does not count as the room changing.
	const print = createMemo(() => fingerprint(room))
	// The setting is part of the key: turning it on in a room asks once, and
	// turning it off drops whatever was waiting to go out.
	const watched = createMemo(() => (enabled() ? print() : undefined))
	createEffect(
		on(watched, (now, before) => {
			if (now === undefined) {
				asks.forget()
				return
			}
			const sameRoom = before?.split('\n')[0] === now.split('\n')[0]
			// A new room: whatever the last one scored is not this one's, and
			// whatever changed in the last one is not its history. Before asking
			// whether it is PvE, since a room's AIs arrive after the room.
			if (!sameRoom) {
				setScore(undefined)
				setFailure(null)
				asks.forget()
			}
			if (!isPve(room)) return
			asks.ask()
		}),
	)
	// Opening the panel is news to its records: whoever came or went while it
	// was shut has not been asked about.
	createEffect(
		on(
			open,
			(isOpen) => {
				if (isOpen && watched() !== undefined && isPve(room)) asks.ask()
			},
			{ defer: true },
		),
	)
	onCleanup(() => asks.stop())

	/**
	 * A figure as seated, then with every seat taken, the way the battle list
	 * writes its challenge: `now–full`, the seated one muted. One figure for a
	 * full room, a game in progress, or an older service.
	 */
	const pair = (
		now: number | null | undefined,
		full: number | null | undefined,
		format: (value: number) => string,
		unit = '',
	): JSX.Element =>
		now == null ? (
			'—'
		) : (
			<FigurePair
				parts={challengePair(now, full ?? null, format)}
				unit={unit}
			/>
		)

	/** `12.0–15.1/34`: to the decimal here, whole numbers in the battle list. */
	const challenge = () =>
		pair(shown()?.challenge, shown()?.full?.challenge, tenths, '/34')
	const win = () => pair(shown()?.winChance, shown()?.full?.winChance, chance)
	const harder = () =>
		pair(shown()?.percentile, shown()?.full?.percentile, percentRank)

	/** Under a figure's explanation: which number is which, once there are two. */
	const bothSeats = (
		now: number | null | undefined,
		full: number | null | undefined,
		format: (value: number) => string,
	) => {
		const held = shown()
		if (now == null || full == null || !held?.full) return ''
		const players = held.seated == null ? '' : ` (${held.seated})`
		return `\n\nAs seated now${players}: ${format(now)}\nWith all ${held.full.players} seats taken: ${format(full)}`
	}

	/** Sizes the model saw too few games at to stand behind, as a chip's words. */
	const uncertain = () => {
		const held = shown()
		const sizes = held ? uncertainSizes(held) : []
		if (sizes.length === 0) return ''
		const players = sizes.map((size) => `${size} players`).join(' and ')
		return `Almost no games have been played with ${players}, so that figure is an uncertain projection: the model extrapolates from smaller teams rather than measuring this one.`
	}

	/** Why there is no figure at all: the opponent is not one the service scores. */
	const declined = () => {
		const held = shown()
		return held ? declinedText(held) : ''
	}

	/** A tooltip with how old the newest game behind the numbers is under it. */
	const dated = (text: string) => {
		const ago = age(shown()?.dataUntil ?? '')
		return ago ? `${text}\nData updated ${ago}` : text
	}

	/**
	 * A number's place in the row, kept while the number is being asked for.
	 *
	 * The dots sit where the figure will land, so the row neither disappears
	 * nor jumps when an answer arrives; a figure appearing from nowhere reads
	 * as a glitch. Every slot waits the same way — none of them may claim
	 * something about the setup before the service has said it. Still dots:
	 * this panel is in view for as long as the room is, and a cycle there is
	 * noise; a dash afterwards is the answer "none".
	 */
	const Slot = (props: { value: JSX.Element }) => (
		<b>
			<Show
				when={!waiting()}
				fallback={<Thinking still title='asking how hard this setup is' />}
			>
				{props.value}
			</Show>
		</b>
	)

	return (
		<Show when={enabled() && hasRow(room) && score() !== null}>
			{/* The whole row opens the panel: the figures are a summary of it.
			    The button stays the way in from the keyboard. */}
			<div class='pve-score' onClick={() => setOpen(true)}>
				<button
					type='button'
					class='pve-open'
					title='PvE stats: the histogram, the match evidence and each person’s record'
					aria-label='Open PvE stats'
				>
					<span class='filter-label'>PvE Stats</span>
					{/* Waits the way the figures beside it do: still dots, in a box
					    the size of the icon so nothing shifts when it lands. */}
					<Show
						when={!waiting() || failure()}
						fallback={
							<span class='pve-miniature'>
								<Thinking still title='asking how hard this setup is' />
							</span>
						}
					>
						<Miniature score={shown() ?? undefined} />
					</Show>
				</button>

				<Show
					when={!failure()}
					fallback={
						<span class='muted' title={failure() ?? ''}>
							unavailable
						</span>
					}
				>
					<span
						class='pve-stat'
						title={dated(
							`Absolute difficulty on a 0-34 scale. 17 is an estimated even game for a representative human team; higher is harder. A dash means the service has not placed this setup among played games yet.${bothSeats(shown()?.challenge, shown()?.full?.challenge, tenths)}`,
						)}
					>
						<span class='pve-label'>Challenge</span>
						<span class='pve-figure wide'>
							<Slot value={challenge()} />
						</span>
					</span>

					<span
						class='pve-stat'
						title={dated(
							`Estimated chance a representative current BAR human team wins this map and setup. The people in this room are not part of that estimate.${bothSeats(shown()?.winChance, shown()?.full?.winChance, chance)}`,
						)}
					>
						<span class='pve-label'>Win</span>
						<span class='pve-figure'>
							<Slot value={win()} />
						</span>
					</span>

					<span
						class='pve-stat muted'
						title={dated(
							`Share of eligible played games against this opponent whose setup had a lower Challenge than this one. Counted per game, so a setup played often weighs more than one played once.${bothSeats(shown()?.percentile, shown()?.full?.percentile, percentRank)}`,
						)}
					>
						<span class='pve-label'>Harder than</span>
						<span class='pve-figure'>
							<Slot value={harder()} />
						</span>
					</span>

					<Show when={!waiting() && declined()}>
						<span class='chip warn' title={declined()}>
							unscored
						</span>
					</Show>

					<Show when={!waiting() && uncertain()}>
						<span class='chip warn' title={uncertain()}>
							uncertain projection
						</span>
					</Show>

					<Show when={!waiting() && shown()?.bestEffort}>
						<span
							class='chip warn'
							title='This room uses settings the service has not catalogued, so these are best-effort estimates rather than an exact match.'
						>
							best effort
						</span>
					</Show>
				</Show>
			</div>
			{/* Outside the row, which opens it on any click: a click inside the
			    panel, Close included, must not travel up and open it again. */}
			<Show when={open()}>
				<PveStats
					score={shown()}
					waiting={waiting()}
					failure={failure()}
					present={present(room)}
					asked={askedPresent() ?? present(room)}
					onClose={() => setOpen(false)}
				/>
			</Show>
		</Show>
	)
}
