import { listen } from '@tauri-apps/api/event'
import {
	For,
	Show,
	createEffect,
	createMemo,
	createResource,
	createSignal,
	on,
	onCleanup,
} from 'solid-js'
import { ActionCell, CellButton } from '../components/ActionCell'
import { Ask } from '../components/Ask'
import {
	SectionChips,
	type Unavailable,
	sendable,
} from '../components/SectionChips'
import { type Segment, Segmented } from '../components/Segmented'
import { type DiffSide, SettingsDiff } from '../components/SettingsDiff'
import type { HistoryView } from '../ipc/bindings/HistoryView'
import type { ModOption } from '../ipc/bindings/ModOption'
import type { ScanProgress } from '../ipc/bindings/ScanProgress'
import type { Sections } from '../ipc/bindings/Sections'
import type { SettingRow } from '../ipc/bindings/SettingRow'
import { api, describeError } from '../ipc/client'
import {
	type Filter,
	type HistoryColumn,
	type Part,
	type Side,
	bestRole,
	departures,
	differingKeys,
	filterRows,
	playersOf,
	playersTip,
	retiredKeys,
	sideOf,
	sortRows,
} from '../lib/history'
import { loadOutcome, startBoxesKept, when } from '../lib/presets'
import { readModOptions } from '../lib/setup'
import { pushNotice } from '../store/chat'
import { lobby } from '../store/lobby'
import { useRoom } from './room/model'

/** The list's columns; one with no key is not sorted by. */
const COLUMNS: Array<[HistoryColumn | null, string, string]> = [
	['map', 'Map', 'The map it was played on'],
	[null, 'Players', 'Who else played it, the newest game first'],
	['played', 'When', 'When it was last played'],
	['games', 'Games', 'How many games were played with it'],
	['delta', 'Δ', 'How many settings differ from this room'],
]

const PARTS: readonly Segment<Part>[] = [
	{ value: 'all', label: 'All', title: 'Every setting, whatever part you had' },
	{ value: 'played', label: 'Played', title: 'Settings you played' },
	{
		value: 'spectated',
		label: 'Spectated',
		title: 'Settings you only watched, or came into after the game began',
	},
]

/**
 * Map, settings, room size, start boxes and AIs: what makes a game that game.
 * The reset is off, because it throws away what this room already has.
 */
const DEFAULT_SECTIONS: Sections = {
	map: true,
	modoptions: true,
	battle: true,
	startBoxes: true,
	bots: true,
	reset: false,
}

/** The games I played first: those are the settings worth having again. */
const DEFAULT_FILTER: Filter = {
	needle: '',
	thisMap: false,
	part: 'played',
}

const EMPTY: HistoryView = {
	settings: [],
	usual: {},
	blobs: {},
	ignoredPrefixes: [],
}

/**
 * The settings of games already played, read from the replays on this
 * machine: one row per distinct setting, with how many games it was played
 * in and how far it is from this room.
 *
 * Clicking a row compares it with the room; pinning a row compares the next
 * one clicked with it instead. A setting loads the way a preset does.
 */
export function History(props: { catalogue: readonly ModOption[] }) {
	const room = useRoom()
	const [view, { mutate }] = createResource(async () => {
		try {
			return await api.gameHistory()
		} catch (error) {
			pushNotice('warning', `history: ${describeError(error)}`)
			return EMPTY
		}
	})
	const [progress, setProgress] = createSignal<ScanProgress | null>(null)
	const [scanning, setScanning] = createSignal(false)
	const [busy, setBusy] = createSignal(false)

	/** Reads the replays that are new or changed since the last look. */
	async function scan() {
		if (scanning()) return
		setScanning(true)
		try {
			mutate(await api.gameHistoryScan())
		} catch (error) {
			pushNotice('warning', `reading replays: ${describeError(error)}`)
		} finally {
			setScanning(false)
			setProgress(null)
		}
	}

	const pending = listen<ScanProgress>('game-history-scan', (event) =>
		setProgress(event.payload),
	)
	onCleanup(() => void pending.then((unlisten) => unlisten()))
	// The list already held shows at once; then a look for what is newer,
	// once that list is in so the older answer cannot land on the newer.
	let looked = false
	createEffect(() => {
		if (view() === undefined || looked) return
		looked = true
		void scan()
	})
	// A game just ended has written its replay.
	createEffect(
		on(
			() => lobby.engine.state,
			(state, before) => {
				if (before === 'running' && state !== 'running') void scan()
			},
			{ defer: true },
		),
	)

	const roomMap = () => room.battle()?.mapName ?? null
	/** Equal while the values are, so a room update that is about seats asks nothing. */
	const roomValues = createMemo(
		() => readModOptions(room.my()?.scriptTags),
		undefined,
		{
			equals: (a, b) => JSON.stringify(a) === JSON.stringify(b),
		},
	)
	const [roomHeld] = createResource(roomValues, async (values) => {
		try {
			return await api.gameHistoryValues(values)
		} catch {
			return { values, blobs: {} }
		}
	})
	const roomSide = (): Side => ({
		map: roomMap(),
		values: roomHeld()?.values ?? {},
	})
	const blobs = () => ({
		...(view() ?? EMPTY).blobs,
		...(roomHeld()?.blobs ?? {}),
	})

	const sides = createMemo(() => {
		const { settings, usual } = view() ?? EMPTY
		return new Map(settings.map((row) => [row.key, sideOf(row, usual)]))
	})
	const deltas = createMemo(() => {
		const from = roomSide()
		return new Map(
			[...sides()].map(([key, side]) => [
				key,
				differingKeys(props.catalogue, from, side).length +
					Number(side.map !== from.map),
			]),
		)
	})

	const [filter, setFilter] = createSignal<Filter>(DEFAULT_FILTER)
	const [column, setColumn] = createSignal<HistoryColumn>('played')
	const [descending, setDescending] = createSignal(true)
	const text = (row: SettingRow) =>
		`${row.map} ${departures(row, props.catalogue, blobs()).join(' ')}`
	const rows = createMemo(() =>
		sortRows(
			filterRows((view() ?? EMPTY).settings, filter(), roomMap(), text),
			column(),
			descending(),
			(row) => deltas().get(row.key) ?? 0,
		),
	)

	function head(next: HistoryColumn) {
		if (next === column()) return setDescending(!descending())
		setColumn(next)
		setDescending(next !== 'map' && next !== 'delta')
	}

	const [chosen, setChosen] = createSignal<string | null>(null)
	const [pinned, setPinned] = createSignal<string | null>(null)
	const [sections, setSections] = createSignal<Sections>(DEFAULT_SECTIONS)
	/**
	 * A past game names no SPADS preset, so a reset goes back to the one the
	 * room runs, which the host says in its own time.
	 */
	const unavailable = (): Unavailable =>
		room.caps.spads && !room.my()?.preset
			? { reset: 'The host has not said which preset it runs yet' }
			: {}
	const [asking, setAsking] = createSignal(false)
	const [now, setNow] = createSignal(Date.now())
	const tick = setInterval(() => setNow(Date.now()), 30_000)
	onCleanup(() => clearInterval(tick))

	const rowOf = (key: string | null) =>
		(view() ?? EMPTY).settings.find((row) => row.key === key)
	const selected = () => rowOf(chosen())

	const roomDiff = (): DiffSide => ({
		title: 'This room',
		side: roomSide(),
		blob: async (key) => roomValues()[key] ?? '',
	})
	const settingDiff = (row: SettingRow): DiffSide => {
		const side = sides().get(row.key) ?? { map: row.map, values: {} }
		return {
			title: `${row.map}, ${when(row.games[0]?.started ?? null, now())}`,
			side,
			blob: (key) => api.gameHistoryBlob(side.values[key] ?? ''),
		}
	}
	const left = () => {
		const held = rowOf(pinned())
		return held ? settingDiff(held) : roomDiff()
	}

	async function act(what: string, run: () => Promise<unknown>) {
		setBusy(true)
		try {
			await run()
		} catch (error) {
			pushNotice('warning', `${what}: ${describeError(error)}`)
		} finally {
			setBusy(false)
		}
	}

	const load = (row: SettingRow) =>
		act('load', async () => {
			const presets = room.presets()
			if (presets === null) return
			const skip = retiredKeys(
				props.catalogue,
				sides().get(row.key) ?? roomSide(),
			)
			const plan = await presets.applyHistory(
				row.key,
				sendable(sections(), unavailable()),
				skip,
			)
			pushNotice('info', `${row.map}: ${loadOutcome(plan, room.caps.spads)}`)
			if (plan.startBoxesUnsent)
				pushNotice('warning', startBoxesKept(room.caps.spads))
			if (skip.length > 0)
				pushNotice(
					'info',
					`left out ${skip.length} settings this game no longer has: ${skip.join(', ')}`,
				)
		})

	const save = (row: SettingRow, name: string) =>
		act('save', async () => {
			await api.historyToPreset(row.key, name)
			pushNotice('info', `saved as the preset ${name}`)
		})

	return (
		<section class='presets past-games'>
			<header class='toolbar'>
				<input
					class='search'
					placeholder='Search maps and tweaks'
					value={filter().needle}
					onInput={(event) =>
						setFilter({ ...filter(), needle: event.currentTarget.value })
					}
				/>
				<button
					class='primary'
					disabled={busy() || !selected() || room.presets() === null}
					title={
						room.presets() === null
							? 'Join a room first'
							: 'Put the selected setting into this room'
					}
					onClick={() => {
						const row = selected()
						if (row) void load(row)
					}}
				>
					Load
				</button>
				<button
					disabled={busy() || !selected()}
					title='Keep the selected setting as a preset'
					onClick={() => setAsking(true)}
				>
					Save as preset
				</button>
				<button
					disabled={scanning()}
					title='Read the replays again for games played since'
					onClick={() => void scan()}
				>
					<Show when={scanning()} fallback='Rescan'>
						<Show when={progress()} fallback='Reading…'>
							{(step) => `Reading ${step().done}/${step().total}`}
						</Show>
					</Show>
				</button>
			</header>

			<div class='preset-sections'>
				<button
					class='chip-choice'
					classList={{ on: filter().thisMap }}
					title="Only settings played on this room's map"
					onClick={() => setFilter({ ...filter(), thisMap: !filter().thisMap })}
				>
					This map
				</button>
				<Segmented
					label='Your part in the games'
					value={filter().part}
					options={PARTS}
					onChange={(part) => setFilter({ ...filter(), part })}
				/>
			</div>
			<SectionChips
				sections={sections()}
				onChange={setSections}
				unavailable={unavailable()}
			/>

			<div class='preset-table'>
				<div class='preset-rows past-rows'>
					<div class='preset-row head'>
						<For each={COLUMNS}>
							{([key, label, hint]) =>
								key === null ? (
									<span class='preset-head past-head' title={hint}>
										{label}
									</span>
								) : (
									<button
										class='preset-head'
										classList={{ on: column() === key }}
										title={hint}
										onClick={() => head(key)}
									>
										{label}
										<Show when={column() === key}>
											<span class='arrow'>{descending() ? ' ↓' : ' ↑'}</span>
										</Show>
									</button>
								)
							}
						</For>
						<span />
						<span />
					</div>
					<For
						each={rows()}
						fallback={
							<p class='muted setup-empty'>
								<Show
									when={(view()?.settings.length ?? 0) > 0}
									fallback='No games yet: each replay this machine records adds its setting here.'
								>
									Nothing matches.
								</Show>
							</p>
						}
					>
						{(row) => {
							const unusual = () => departures(row, props.catalogue, blobs())
							const ais = [...new Set(row.games.flatMap((game) => game.ais))]
							const players = playersOf(row)
							return (
								<div
									class='preset-row past-row'
									classList={{
										on: chosen() === row.key,
										pinned: pinned() === row.key,
									}}
									title={playersTip(players)}
									onClick={() => setChosen(row.key)}
									onDblClick={() => void load(row)}
								>
									<span class='preset-map past-map'>
										{row.map}
										<Show when={unusual().length > 0}>
											<small class='muted past-unusual'>
												{unusual().join(', ')}
											</small>
										</Show>
									</span>
									<span class='muted past-players'>{players.join(', ')}</span>
									<span class='tabular muted'>
										{when(row.games[0]?.started ?? null, now())}
									</span>
									<span class='tabular muted'>×{row.games.length}</span>
									{/* Until the room is read there is nothing to measure from. */}
									<span class='tabular past-delta'>
										{roomHeld() === undefined
											? '…'
											: (deltas().get(row.key) ?? 0)}
									</span>
									<span class='past-marks muted'>
										<Show when={bestRole(row) === 'spectated'}>
											<span
												class='doc-tag'
												title='You only watched these games'
											>
												watched
											</span>
										</Show>
										<Show when={bestRole(row) === 'late'}>
											<span
												class='doc-tag'
												title='You came into these games after they began, so their replays do not list you'
											>
												late
											</span>
										</Show>
										<Show when={ais.length > 0}>
											<span title={ais.join(', ')}>{ais.join(', ')}</span>
										</Show>
									</span>
									<ActionCell filled={pinned() === row.key}>
										<CellButton
											icon='act-compare'
											title={
												pinned() === row.key
													? 'Compare with this room again'
													: 'Compare other settings with this one'
											}
											onClick={(event) => {
												event.stopPropagation()
												setPinned(pinned() === row.key ? null : row.key)
											}}
										/>
									</ActionCell>
								</div>
							)
						}}
					</For>
				</div>
			</div>

			<div class='past-split'>
				<Show
					when={selected()}
					fallback={
						<p class='muted setup-empty'>
							Pick a setting to see how it differs from this room.
						</p>
					}
				>
					{(row) => (
						<>
							<Show when={pinned() !== null}>
								<div class='past-compare'>
									<button
										disabled={pinned() === row().key}
										title='Swap the two sides'
										onClick={() => {
											const held = pinned()
											setPinned(row().key)
											setChosen(held)
										}}
									>
										Swap
									</button>
									<button
										title='Compare with this room again'
										onClick={() => setPinned(null)}
									>
										Compare with this room
									</button>
								</div>
							</Show>
							<SettingsDiff
								catalogue={props.catalogue}
								left={left()}
								right={settingDiff(row())}
								blobs={blobs()}
							/>
						</>
					)}
				</Show>
			</div>

			<Show when={asking() && selected()}>
				{(row) => (
					<Ask
						title='Save as preset'
						hint={`The setting played on ${row().map}`}
						confirm='Save'
						onCancel={() => setAsking(false)}
						onAnswer={(name) => {
							setAsking(false)
							void save(row(), name)
						}}
					/>
				)}
			</Show>
		</section>
	)
}
