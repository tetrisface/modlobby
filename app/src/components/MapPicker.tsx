import { For, Show, createMemo, createResource, createSignal } from 'solid-js'
import type { MapFacts } from '../ipc/bindings/MapFacts'
import { api, describeError } from '../ipc/client'
import {
	CARD_TILE,
	ROW_TILE,
	mapFacts,
	mapNameFromFile,
	mapNames,
} from '../lib/maps'
import { localStore, readFlag, writeFlag } from '../lib/resize'
import { pushNotice } from '../store/chat'
import { MapPicture } from './MapPicture'
import { Select } from './Select'

/** As many as the grid draws before searching is the faster way to find one. */
const SHOWN = 300

/** Where the layout you last chose is kept, per viewer, not per room. */
const LIST_KEY = 'modlobby.maps.list'

/** Whether the index says nothing here: 0 for a number, empty for a name. */
const blank = (value: string | number): number =>
	value === 0 || value === '' ? 1 : 0

/** Numbers as numbers, names as names. */
function compare(a: string | number, b: string | number): number {
	if (typeof a === 'number' && typeof b === 'number') return a - b
	return String(a).localeCompare(String(b), undefined, { numeric: true })
}

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
 * A column: what it is called, what it shows, and what it sorts on.
 *
 * One set for both layouts. The grid has no headers to click, so its Sort
 * picker offers these same columns -- otherwise "sorted by size" would mean
 * two different orders depending on how you were looking.
 *
 * `of` is the sort key. Size is the area, width times height, because that is
 * what "bigger map" means: 8 x 24 and 24 x 8 are the same amount of ground,
 * and neither side alone says so. Numbers sort largest first by default,
 * names A to Z, which is what each is usually wanted as.
 */
type Column = {
	key: string
	label: string
	/** What the list's cell reads, and nothing where the index does not say. */
	cell: (entry: Entry) => string | null
	of: (entry: Entry) => string | number
	/** Biggest or most first, the way a number is usually asked for. */
	numeric: boolean
}

const COLUMNS = [
	{
		key: 'name',
		label: 'Name',
		cell: (entry) => entry.label,
		of: (entry) => entry.label.toLowerCase(),
		numeric: false,
	},
	{
		key: 'size',
		label: 'Size',
		cell: (entry) => size(entry.facts),
		of: area,
		numeric: true,
	},
	{
		key: 'players',
		label: 'Players',
		cell: (entry) => players(entry.facts),
		of: (entry) => entry.facts?.playersMax ?? 0,
		numeric: true,
	},
	{
		key: 'author',
		label: 'Author',
		cell: (entry) => entry.facts?.author || null,
		of: (entry) => (entry.facts?.author ?? '').toLowerCase(),
		numeric: false,
	},
] as const satisfies readonly Column[]

type SortKey = (typeof COLUMNS)[number]['key']

const columnOf = (key: SortKey): Column =>
	COLUMNS.find((column) => column.key === key) ?? COLUMNS[0]

/** The area of a map, in the units the feed gives its sides in. */
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
	const [down, setDown] = createSignal(false)
	const [heldOnly, setHeldOnly] = createSignal(false)
	/** Which way you were last looking at these, kept for the next time. */
	const [asList, setAsList] = createSignal(readFlag(localStore(), LIST_KEY))

	function showAs(list: boolean) {
		setAsList(list)
		writeFlag(localStore(), LIST_KEY, list)
	}

	/** Sorting by what you are already sorted by turns it round, as a table does. */
	function sortBy(key: SortKey) {
		if (key === sort()) return setDown(!down())
		setSort(key)
		setDown(columnOf(key).numeric)
	}

	const entries = createMemo((): Entry[] => {
		const index = names() ?? {}
		const known = facts() ?? {}
		const files = options()?.maps ?? []
		// A map nothing publishes is named by its file, which is a name the
		// engine cannot resolve until its underscores are spaces again.
		const held = new Set(
			files.map((file) => index[file] ?? mapNameFromFile(file)),
		)
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
		const column = columnOf(sort())
		const way = down() ? -1 : 1
		const matching = entries().filter((entry) => {
			if (heldOnly() && !entry.held) return false
			if (!needle) return true
			return haystack(entry).includes(needle)
		})
		return matching
			.sort((a, b) => {
				const [x, y] = [column.of(a), column.of(b)]
				// A map the index says nothing about is not the smallest one,
				// it is unknown -- so it sits at the bottom whichever way
				// round the column is, outside the reversal.
				return (
					blank(x) - blank(y) ||
					way * compare(x, y) ||
					// The name breaks every tie, so the order never flickers
					// between two maps the sorted column cannot tell apart.
					a.label.localeCompare(b.label, undefined, { numeric: true })
				)
			})
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
					{/* The list has headers to click, so it needs no picker; the
              grid has none, and gets the same columns here. */}
					<Show when={!asList()}>
						<Select
							value={sort()}
							title='How the maps are ordered'
							onChange={(event) => sortBy(event.currentTarget.value as SortKey)}
						>
							<For each={COLUMNS}>
								{(column) => (
									<option value={column.key}>
										{column.label} {down() && sort() === column.key ? '↓' : '↑'}
									</option>
								)}
							</For>
						</Select>
					</Show>
					<label class='map-held' title='Only the maps already on this machine'>
						<input
							type='checkbox'
							checked={heldOnly()}
							onChange={(event) => setHeldOnly(event.currentTarget.checked)}
						/>
						On disk
					</label>
					{/* Pictures to browse by, or columns to compare by. Both draw
              the same maps in the same order; only the shape differs. */}
					<div class='map-layout' role='group' aria-label='Layout'>
						<button
							type='button'
							class='chip-choice'
							classList={{ on: !asList() }}
							aria-pressed={!asList()}
							title='Pictures'
							onClick={() => showAs(false)}
						>
							Grid
						</button>
						<button
							type='button'
							class='chip-choice'
							classList={{ on: asList() }}
							aria-pressed={asList()}
							title='Columns you can sort by'
							onClick={() => showAs(true)}
						>
							List
						</button>
					</div>
					{/* A map nobody publishes -- anything outside BAR's pool --
              arrives by hand, and this is where it goes. */}
					<button
						type='button'
						title='Open the folder maps are installed in'
						onClick={() =>
							void api
								.openMapsDir()
								.catch((error) =>
									pushNotice('warning', `maps folder: ${describeError(error)}`),
								)
						}
					>
						Maps folder
					</button>
					<button type='button' onClick={props.onClose}>
						Close
					</button>
				</header>

				<Show when={note()}>
					{(said) => <p class='muted setup-note'>{said()}</p>}
				</Show>

				<Show when={asList()}>
					<div class='map-table'>
						<div class='map-row head'>
							<span class='map-row-pic' />
							<For each={COLUMNS}>
								{(column) => (
									<button
										type='button'
										class={`map-col ${column.key}`}
										classList={{ on: sort() === column.key }}
										aria-sort={
											sort() === column.key
												? down()
													? 'descending'
													: 'ascending'
												: 'none'
										}
										onClick={() => sortBy(column.key)}
									>
										{column.label}
										<Show when={sort() === column.key}>
											<span class='map-col-way'>{down() ? '↓' : '↑'}</span>
										</Show>
									</button>
								)}
							</For>
							<span class='map-col disk'>Disk</span>
						</div>
						<div class='map-rows'>
							<For
								each={shown()}
								fallback={<p class='muted setup-empty'>Nothing matches.</p>}
							>
								{(entry) => (
									<button
										class='map-row'
										classList={{
											on: props.current === entry.value,
											absent: !entry.held,
										}}
										title={cardTitle(entry)}
										onClick={() => props.onPick(entry.value, entry.held)}
									>
										<MapPicture
											class='map-row-pic'
											mapName={entry.value}
											width={ROW_TILE.width}
											height={ROW_TILE.height}
											lazy
										/>
										<For each={COLUMNS}>
											{(column) => (
												<span class={`map-cell ${column.key}`}>
													{column.cell(entry) ?? ''}
												</span>
											)}
										</For>
										<span class='map-cell disk'>{entry.held ? '✓' : ''}</span>
									</button>
								)}
							</For>
						</div>
					</div>
				</Show>

				<Show when={!asList()}>
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
				</Show>
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
