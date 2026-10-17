import {
	For,
	Show,
	createEffect,
	createMemo,
	createSignal,
	on,
	onCleanup,
} from 'solid-js'
import { Thinking } from '../components/Thinking'
import type { Score } from '../ipc/bindings/Score'
import { describeError } from '../ipc/client'
import { age } from '../lib/age'
import { asker } from '../lib/asking'
import { NOTHING_YET, TOP, miniature } from '../lib/pve'
import { askDelay } from '../lib/stagger'
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
/** How long the room has to stop changing before it is worth asking again. */
export const QUIET_FOR = 3000
/** The least time between two asks from this client, whatever the room does. */
export const AT_MOST_EVERY = 2000

/**
 * Everything the answer depends on, as one string.
 *
 * Rust builds the request from the room; this only has to notice when any of
 * that changes. Reading each value here is what subscribes to it — the store
 * mutates the settings object in place, so watching the object itself would
 * never fire again after the first look.
 */
function fingerprint(room: RoomModel): string | undefined {
	const my = room.my()
	const battle = room.battle()
	if (!my || !battle) return undefined
	const settings = Object.entries(my.scriptTags)
		.filter(([key]) => key.startsWith('game/modoptions/'))
		.map(([key, value]) => `${key}=${value}`)
		.sort()
	const bots = battle.bots.map((bot) => `${bot.ai}@${bot.status.handicap}`)
	const seats = battle.members.map((name) => {
		const status = room.users()[name]?.battleStatus
		return status?.player ? String(status.handicap) : ''
	})
	return [
		my.id,
		battle.mapName,
		battle.playerCount,
		...bots,
		...seats,
		...settings,
	].join('\n')
}

/**
 * Whether this room has a PvE opponent in it at all.
 *
 * Decided here as well as in Rust, which stays authoritative for what is
 * sent. This one only decides whether to ask — without it a room with no AI
 * in it flashes "PvE" and a row of dots before the answer comes back null,
 * and every settings change in every ordinary room costs a round trip to
 * find out nothing.
 */
function isPve(room: RoomModel): boolean {
	const names = (room.battle()?.bots ?? [])
		.map((bot) => bot.ai.toLowerCase())
		.join(' ')
	const raptors = names.includes('raptor')
	const scavengers = names.includes('scavenger')
	// Both at once is not a setup the model knows, so it is not asked about.
	if (raptors && scavengers) return false
	return raptors || scavengers || names.includes('barb')
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
		props.score?.histogram.length ? miniature(props.score) : NOTHING_YET,
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
	const [asking, setAsking] = createSignal(false)
	const [failure, setFailure] = createSignal<string | null>(null)
	/** Whether the full panel is open over the room. */
	const [open, setOpen] = createSignal(false)

	/**
	 * Paced, because the service is a Lambda with a concurrency of one and a
	 * twenty-second cold start, and everyone in the room sees the same change
	 * at the same moment. One ask out at a time, a floor between asks, and a
	 * wait by seat rank so the room's clients arrive one after another rather
	 * than all at once. Rust reads the room afresh for each ask, so a single
	 * follow-up covers whatever changed while one was out.
	 */
	const asks = asker(
		async () => {
			setAsking(true)
			try {
				setScore(await room.io.pveScore())
				setFailure(null)
			} catch (err) {
				setFailure(describeError(err))
				console.warn('pve stats:', err)
			} finally {
				setAsking(false)
			}
		},
		{ floor: AT_MOST_EVERY, stagger: () => myStagger(room) },
	)

	/**
	 * Asked as soon as a room is in view, then again whenever it settles.
	 *
	 * A host applying a preset changes a hundred settings in a couple of
	 * minutes, and each one arrives as its own script tag. Asking per change
	 * would put a hundred requests on somebody else's service to answer a
	 * question whose answer only matters once the changes stop.
	 */
	let pending: ReturnType<typeof setTimeout> | undefined
	// A memo, so a change that leaves the string as it was — someone toggling
	// ready, say — does not count as the room changing.
	const print = createMemo(() => fingerprint(room))
	// The setting is part of the key: turning it on in a room asks once, and
	// turning it off drops whatever was waiting to go out.
	const watched = createMemo(() => (enabled() ? print() : undefined))
	createEffect(
		on(watched, (now, before) => {
			clearTimeout(pending)
			if (now === undefined || !isPve(room)) return
			const sameRoom = before?.split('\n')[0] === now.split('\n')[0]
			if (sameRoom) {
				pending = setTimeout(asks.ask, QUIET_FOR)
				return
			}
			// A new room: whatever the last one scored is not this one's.
			setScore(undefined)
			setFailure(null)
			asks.ask()
		}),
	)
	onCleanup(() => {
		clearTimeout(pending)
		asks.stop()
	})

	/** No answer yet, or a fresh one on its way. */
	const waiting = () => asking() || score() === undefined

	const percent = (value: number | null | undefined) =>
		value == null ? '—' : `${Math.round(value * 100)}%`

	const challenge = () => {
		const held = score()?.challenge
		return held == null ? '—' : `${held.toFixed(1)}/34`
	}

	/** A tooltip with how old the newest game behind the numbers is under it. */
	const dated = (text: string) => {
		const ago = age(score()?.dataUntil ?? '')
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
	const Slot = (props: { value: string }) => (
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
		<Show when={enabled() && isPve(room) && score() !== null}>
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
						<Miniature score={score() ?? undefined} />
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
							'Absolute difficulty on a 0-34 scale. 17 is an estimated even game for a representative human team; higher is harder. A dash means the service has not placed this setup among played games yet.',
						)}
					>
						<span class='pve-label'>Challenge</span>
						<span class='pve-figure'>
							<Slot value={challenge()} />
						</span>
					</span>

					<span
						class='pve-stat'
						title={dated(
							'Estimated chance a representative current BAR human team wins this map and setup. The people in this room are not part of that estimate.',
						)}
					>
						<span class='pve-label'>Win</span>
						<span class='pve-figure'>
							<Slot value={percent(score()?.winChance)} />
						</span>
					</span>

					<span
						class='pve-stat muted'
						title={dated(
							'Share of eligible played games against this opponent whose setup had a lower Challenge than this one. Counted per game, so a setup played often weighs more than one played once.',
						)}
					>
						<span class='pve-label'>Harder than</span>
						<span class='pve-figure'>
							<Slot
								value={
									score()?.percentile == null
										? '—'
										: `${Math.round(score()?.percentile ?? 0)}%`
								}
							/>
						</span>
					</span>

					<Show when={!waiting() && score()?.bestEffort}>
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
					score={score()}
					waiting={waiting()}
					failure={failure()}
					onClose={() => setOpen(false)}
				/>
			</Show>
		</Show>
	)
}
