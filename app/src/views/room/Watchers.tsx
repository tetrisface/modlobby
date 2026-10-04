import {
	For,
	Show,
	createEffect,
	createSignal,
	onCleanup,
	onMount,
} from 'solid-js'
import { WatcherRow } from '../../components/PlayerRow'
import type { UserView } from '../../ipc/bindings/UserView'
import { localStore, readFlag, writeFlag } from '../../lib/resize'
import { isBoss } from '../../lib/roster'
import type { Skill } from '../../lib/skill'

/** What both cards need to draw a person. */
type People = {
	skillOf: (name: string) => Skill | null
	me: string | null
	isFriend: (name: string) => boolean
	boss: string | null
}

/**
 * Whether the stack's two cards stand abreast on the teams' row rather than
 * one over the other: when the teams leave two cards' room there, and one
 * over the other they would stand taller than the tallest team. The stack
 * would then be what sets the roster's height, and two columns halve it.
 */
export function standsAbreast(
	spare: number,
	stacked: number,
	tallestTeam: number,
): boolean {
	return spare >= 2 && stacked > tallestTeam
}

/**
 * The people watching: the join queue over the spectators.
 *
 * Three placements, by what the teams leave of their row (`spare`). With a
 * card's room there, the two are one stack hugging the right of the people
 * area -- abreast rather than one over the other when there is room for a
 * second card too and stacking them would outgrow the teams
 * (`standsAbreast`). With none, the stack dissolves (`display: contents`)
 * and the two are ordinary cards flowing after the last team, filling the
 * row's remaining slots exactly as another team would. The teams always
 * come first either way.
 *
 * `spare` is counted from the row's width, and the cards' own heights are
 * held against the teams' -- never where the stack itself landed -- so the
 * answer cannot depend on itself.
 *
 * A card spread across the width leaves the stack for a full-width row of
 * its own below the teams; whatever is not spread keeps its place. Each
 * card's form is remembered.
 */
export function WatcherStack(
	props: People & {
		/** Cards' room the teams leave on their row; none where they wrap. */
		spare: number
		/** In line, as the server gave it. Drawn only when somebody is. */
		queue: UserView[]
		/** As sorted: the host first, then by name. */
		spectators: UserView[]
		/** Listed after the spectators, dimmed: not placed by the server yet. */
		pending: UserView[]
		/** What the spectators' header says, which includes the queue. */
		spectatorCount: number
		/** Joins each card, as a team's Join takes a seat; absent where we are. */
		joinQueue?: Join
		joinSpectators?: Join
	},
) {
	const [queueSpread, setQueueSpread] = remembered('queue')
	const [spectatorsSpread, setSpectatorsSpread] = remembered('spectators')

	let root: HTMLDivElement | undefined
	const beside = () => props.spare >= 1
	const [abreast, setAbreast] = createSignal(false)
	/** Asks the cards what they measure, so the stylesheet stays the truth. */
	function measure() {
		const row = root?.parentElement
		if (!root || !row) return
		const teams = [...row.querySelectorAll<HTMLElement>(':scope > .team')]
		// Only the cards still in the stack; the stack's row gap is the teams'.
		const cards = [...root.querySelectorAll<HTMLElement>(':scope > .watchers')]
		const stacked =
			cards.reduce((sum, card) => sum + contentHeight(card), 0) +
			(cards.length - 1) * (parseFloat(getComputedStyle(row).rowGap) || 0)
		const tallest = Math.max(0, ...teams.map(contentHeight))
		setAbreast(
			cards.length === 2 && standsAbreast(props.spare, stacked, tallest),
		)
	}
	createEffect(() => {
		props.spare
		// A card's height is its rows: a watcher arriving, or a card spread
		// out of the stack, changes what stacking would cost.
		props.queue.length
		props.spectators.length
		props.pending.length
		queueSpread()
		spectatorsSpread()
		measure()
	})
	onMount(() => {
		const row = root?.parentElement
		if (!row || typeof ResizeObserver === 'undefined') return
		const watching = new ResizeObserver(measure)
		watching.observe(row)
		onCleanup(() => watching.disconnect())
	})

	const hasQueue = () => props.queue.length > 0
	const queueCard = () => (
		<WatcherCard
			kind='queue'
			users={props.queue}
			count={props.queue.length}
			spread={queueSpread()}
			onToggle={() => setQueueSpread(!queueSpread())}
			join={props.joinQueue}
			{...people(props)}
		/>
	)
	const spectatorsCard = () => (
		<WatcherCard
			kind='spectators'
			users={props.spectators}
			pending={props.pending}
			count={props.spectatorCount}
			spread={spectatorsSpread()}
			onToggle={() => setSpectatorsSpread(!spectatorsSpread())}
			join={props.joinSpectators}
			{...people(props)}
		/>
	)

	return (
		<>
			<div
				ref={root}
				class='watchers-stack'
				classList={{
					beside: beside(),
					abreast: abreast(),
					flow: !beside(),
					empty: (!hasQueue() || queueSpread()) && spectatorsSpread(),
				}}
			>
				<Show when={hasQueue() && !queueSpread()}>{queueCard()}</Show>
				<Show when={!spectatorsSpread()}>{spectatorsCard()}</Show>
			</div>
			<Show when={hasQueue() && queueSpread()}>{queueCard()}</Show>
			<Show when={spectatorsSpread()}>{spectatorsCard()}</Show>
		</>
	)
}

/**
 * What a card's contents take. The row stretches every card to its tallest
 * item, so a card's box says how tall the row is, not how tall the card
 * would stand on its own; its last child's foot does.
 */
function contentHeight(card: HTMLElement): number {
	const last = card.lastElementChild
	if (!last) return card.offsetHeight
	return last.getBoundingClientRect().bottom - card.getBoundingClientRect().top
}

const people = (p: People): People => ({
	skillOf: p.skillOf,
	me: p.me,
	isFriend: p.isFriend,
	boss: p.boss,
})

/** A card's Join: what it does, said on hover. */
export type Join = { title: string; run: () => void }

/** A card's form, kept across rooms and runs. */
function remembered(kind: 'queue' | 'spectators') {
	const key = `modlobby.room.${kind}Spread`
	const [spread, set] = createSignal(readFlag(localStore(), key))
	return [
		spread,
		(on: boolean) => {
			set(on)
			writeFlag(localStore(), key, on)
		},
	] as const
}

/**
 * One card of watchers. One name per row, the way a team reads; the
 * chevron under the header spreads it across the full width, its names in
 * aligned columns read across, for a crowd too long to scan as one column.
 */
export function WatcherCard(
	props: People & {
		kind: 'queue' | 'spectators'
		users: UserView[]
		pending?: UserView[]
		count: number
		spread: boolean
		onToggle: () => void
		join?: Join
	},
) {
	const queue = () => props.kind === 'queue'
	const title = () => (queue() ? 'Join queue' : 'Spectators')

	const row = (user: UserView, place?: number, pending?: boolean) => (
		<WatcherRow
			user={user}
			skill={props.skillOf(user.name)}
			me={user.name === props.me}
			friend={props.isFriend(user.name)}
			boss={isBoss(props.boss, user.name)}
			pending={pending}
			place={place}
		/>
	)

	return (
		<section
			class={`watchers ${props.kind}`}
			classList={{ spread: props.spread }}
		>
			<header class='team-head'>
				<span class='name'>{title()}</span>
				<span class='count'>{props.count}</span>
				<Show when={props.join}>
					{(join) => (
						<button
							class='team-join'
							title={join().title}
							onClick={() => join().run()}
						>
							Join
						</button>
					)}
				</Show>
			</header>
			<button
				class='spread-toggle'
				aria-expanded={props.spread}
				title={props.spread ? 'Stack in one column' : 'Spread across the width'}
				onClick={props.onToggle}
			>
				<svg viewBox='0 0 8 8' aria-hidden='true'>
					<path d='M2 1.5 L5.5 4 L2 6.5' />
				</svg>
			</button>
			<div class='names'>
				<For each={props.users}>
					{(user, index) => row(user, queue() ? index() + 1 : undefined)}
				</For>
				<For each={props.pending ?? []}>
					{(user) => row(user, undefined, true)}
				</For>
			</div>
		</section>
	)
}
