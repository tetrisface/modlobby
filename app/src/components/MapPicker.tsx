import { For, Show, createMemo, createResource, createSignal } from 'solid-js'
import type { MapFacts } from '../ipc/bindings/MapFacts'
import { api, describeError } from '../ipc/client'
import { CARD_TILE, mapFacts, mapNames } from '../lib/maps'
import { pushNotice } from '../store/chat'
import { MapPicture } from './MapPicture'
import { Select } from './Select'

/** As many as the grid draws before searching is the faster way to find one. */
const SHOWN = 300

/** One map as the grid draws it. */
type Entry = {
	/** What a start script and `!map` both want. */
	value: string
	/** What the card says, which is the author's name for it where there is one. */
	label: string
	/** Absent for a map on disk that the published index does not know. */
	facts: MapFacts | null
	/** On this machine already: the room can play it without fetching. */
	held: boolean
}

/**
 * How the grid is ordered. Name is the one that never surprises; the others
 * are the two questions people actually open this asking -- how big, and how
 * many -- with the name breaking every tie so the order is stable.
 */
const SORTS = {
	name: { label: 'Name', of: () => 0 },
	size: { label: 'Largest', of: (e: Entry) => -area(e) },
	small: { label: 'Smallest', of: (e: Entry) => area(e) },
	players: {
		label: 'Most players',
		of: (e: Entry) => -(e.facts?.playersMax ?? 0),
	},
} as const

type SortKey = keyof typeof SORTS

function area(entry: Entry): number {
	const facts = entry.facts
	return facts ? facts.width * facts.height : 0
}

/** `16 × 16`, or nothing where the index does not say. */
function size(facts: MapFacts | null): string | null {
	if (!facts || facts.width === 0 || facts.height === 0) return null
	return `${facts.width} × ${facts.height}`
}

/** `2–8`, `8`, or nothing. A range of one is a number. */
function players(facts: MapFacts | null): string | null {
	if (!facts || facts.playersMax === 0) return null
	const { playersMin: low, playersMax: high } = facts
	return low > 0 && low !== high ? `${low}–${high}` : `${high}`
}

/**
 * The maps to play on: everything BAR publishes, with what is already on this
 * machine marked.
 *
 * Not just what is installed. The room this opens over may be somebody
 * else's, where picking a map is a `!map` to the host and what *we* have is
 * beside the point; and in a room of our own a map we lack is one download
 * away, which the room already offers -- and a first run has none at all,
 * so a list of what is installed would be an empty one. So the list is the
 * published index, and `held` is a fact about this machine rather than a
 * filter on it -- with On disk there for when it is the only thing that
 * matters.
 *
 * Maps on disk the index has never heard of are listed too, by file name,
 * since they are playable and nothing else would list them.
 */
export function MapPicker(props: {
	/** The room's map, so the one already chosen is marked. */
	current: string
	/**
	 * `installed` is whether the map is on this machine already: a pick from
	 * the published list is a request for it, and the caller is who knows
	 * whether asking means fetching.
	 */
	onPick: (springName: string, installed: boolean) => void
	onClose: () => void
}) {
	const [options] = createResource(() =>
		api.skirmishOptions().catch(() => null),
	)
	const [names] = createResource(mapNames)
	const [facts] = createResource(mapFacts)

	const [search, setSearch] = createSignal('')
	const [sort, setSort] = createSignal<SortKey>('name')
	const [heldOnly, setHeldOnly] = createSignal(false)

	const entries = createMemo((): Entry[] => {
		const index = names() ?? {}
		const known = facts() ?? {}
		const files = options()?.maps ?? []
		const held = new Set(files.map((file) => index[file] ?? file))
		const listed = new Map<string, Entry>()

		for (const [spring, about] of Object.entries(known))
			listed.set(spring, {
				value: spring,
				label: about.displayName || spring,
				facts: about,
				held: held.has(spring),
			})
		// On the disk and not in the index: still playable, still worth listing.
		for (const spring of held)
			if (!listed.has(spring))
				listed.set(spring, {
					value: spring,
					label: spring,
					facts: null,
					held: true,
				})
		return [...listed.values()]
	})

	const shown = createMemo(() => {
		const needle = search().trim().toLowerCase()
		const order = SORTS[sort()]
		const matching = entries().filter((entry) => {
			if (heldOnly() && !entry.held) return false
			if (!needle) return true
			return haystack(entry).includes(needle)
		})
		return matching
			.sort(
				(a, b) =>
					order.of(a) - order.of(b) ||
					a.label.localeCompare(b.label, undefined, { numeric: true }),
			)
			.slice(0, SHOWN)
	})

	const note = () =>
		entries().length === 0
			? 'No maps: nothing is installed and the published index could not be read.'
			: null

	return (
		<div class='sheet' onMouseDown={props.onClose}>
			<div
				class='sheet-card map-picker'
				onMouseDown={(event) => event.stopPropagation()}
			>
				<header class='ed-head'>
					<h2>Map</h2>
					<input
						class='search'
						placeholder={`Search ${entries().length}`}
						value={search()}
						onInput={(event) => setSearch(event.currentTarget.value)}
					/>
					<Select
						value={sort()}
						title='How the maps are ordered'
						onChange={(event) => setSort(event.currentTarget.value as SortKey)}
					>
						<For each={Object.entries(SORTS)}>
							{([key, how]) => <option value={key}>{how.label}</option>}
						</For>
					</Select>
					<label class='map-held' title='Only the maps already on this machine'>
						<input
							type='checkbox'
							checked={heldOnly()}
							onChange={(event) => setHeldOnly(event.currentTarget.checked)}
						/>
						On disk
					</label>
					<button type='button' onClick={props.onClose}>
						Close
					</button>
				</header>

				<Show when={note()}>
					{(said) => <p class='muted setup-note'>{said()}</p>}
				</Show>

				<div class='map-grid'>
					<For
						each={shown()}
						fallback={<p class='muted setup-empty'>Nothing matches.</p>}
					>
						{(entry) => (
							<button
								class='map-card'
								classList={{
									on: props.current === entry.value,
									absent: !entry.held,
								}}
								title={cardTitle(entry)}
								onClick={() => props.onPick(entry.value, entry.held)}
							>
								<MapPicture
									mapName={entry.value}
									width={CARD_TILE.width}
									height={CARD_TILE.height}
									lazy
								/>
								<span class='map-card-name'>{entry.label}</span>
								<span class='map-card-facts'>
									<Show when={size(entry.facts)}>
										{(dims) => <span class='map-dim'>{dims()}</span>}
									</Show>
									<Show when={players(entry.facts)}>
										{(many) => <span class='map-players'>{many()}p</span>}
									</Show>
									{/* Said only where it is news: a map you do not have is
                      one the room has to fetch before it can be played. */}
									<Show when={!entry.held}>
										<span class='map-absent'>not on disk</span>
									</Show>
								</span>
							</button>
						)}
					</For>
				</div>
			</div>
		</div>
	)
}

/** Everything a search looks through, lowercased once per entry. */
function haystack(entry: Entry): string {
	const facts = entry.facts
	return [
		entry.label,
		entry.value,
		facts?.author ?? '',
		...(facts?.terrain ?? []),
		...(facts?.tags ?? []),
	]
		.join(' ')
		.toLowerCase()
}

function cardTitle(entry: Entry): string {
	const facts = entry.facts
	const parts = [entry.value]
	if (facts?.author) parts.push(`by ${facts.author}`)
	const terrain = facts?.terrain.join(', ')
	if (terrain) parts.push(terrain)
	return parts.join(' · ')
}

/**
 * The games or engines this machine has.
 *
 * Newest first, which is what rapid's own ordering makes them and what
 * somebody looking for "the current one" wants at the top.
 */
export function VersionPicker(props: {
	what: 'Game' | 'Engine'
	current: string
	onPick: (version: string) => void
	onClose: () => void
}) {
	const [options] = createResource(() =>
		api.skirmishOptions().catch(() => null),
	)
	const versions = createMemo(() => {
		const held = props.what === 'Game' ? options()?.games : options()?.engines
		return (held ?? []).map((value) => ({ value, label: value }))
	})

	const [search, setSearch] = createSignal('')
	const shown = createMemo(() => {
		const needle = search().trim().toLowerCase()
		const matching = needle
			? versions().filter((entry) => entry.label.toLowerCase().includes(needle))
			: versions()
		return matching.slice(0, SHOWN)
	})

	return (
		<div class='sheet' onMouseDown={props.onClose}>
			<div
				class='sheet-card map-picker'
				onMouseDown={(event) => event.stopPropagation()}
			>
				<header class='ed-head'>
					<h2>{props.what}</h2>
					<input
						class='search'
						placeholder={`Search ${versions().length}`}
						value={search()}
						onInput={(event) => setSearch(event.currentTarget.value)}
					/>
					<button type='button' onClick={props.onClose}>
						Close
					</button>
				</header>

				<Show when={versions().length === 0}>
					<p class='muted setup-note'>
						Nothing is installed to play {props.what === 'Game' ? 'with' : 'on'}
						. The room offers a download for what it needs.
					</p>
				</Show>

				<div class='map-list'>
					<For
						each={shown()}
						fallback={<p class='muted setup-empty'>Nothing matches.</p>}
					>
						{(entry) => (
							<button
								class='room-tab'
								classList={{ on: props.current === entry.value }}
								onClick={() => props.onPick(entry.value)}
							>
								<span class='room-name'>{entry.label}</span>
							</button>
						)}
					</For>
				</div>
			</div>
		</div>
	)
}

/** Picks it, saying why if the room would not take it. */
export async function picked(
	set: (name: string) => Promise<void>,
	what: string,
	name: string,
): Promise<void> {
	try {
		await set(name)
	} catch (error) {
		pushNotice('warning', `${what}: ${describeError(error)}`)
	}
}
