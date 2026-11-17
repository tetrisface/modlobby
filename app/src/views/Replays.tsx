import { createVirtualizer } from '@tanstack/solid-virtual'
import { listen } from '@tauri-apps/api/event'
import {
	For,
	Show,
	createMemo,
	createResource,
	createSignal,
	onCleanup,
	onMount,
} from 'solid-js'
import { Ask } from '../components/Ask'
import { SyncIcon } from '../components/icons'
import type { EngineProgress } from '../ipc/bindings/EngineProgress'
import type { ReplayView } from '../ipc/bindings/ReplayView'
import { api, describeError } from '../ipc/client'
import { exactly } from '../lib/age'
import { span } from '../lib/running'
import { type WatchStep, watchStep } from '../lib/watch'
import { pushNotice } from '../store/chat'
import { lobby } from '../store/lobby'

const ROW_HEIGHT = 40

/**
 * Watch, which on its way is its own progress bar: the same box, filling
 * left to right, so the row holds still as it turns into one.
 */
function WatchButton(props: {
	step: WatchStep | null
	disabled: boolean
	onClick: () => void
}) {
	const fill = () => `${Math.round((props.step?.fraction ?? 0) * 100)}%`
	return (
		<button
			classList={{ watching: props.step !== null }}
			disabled={props.disabled}
			title={props.step?.title}
			style={{ '--fill': fill() }}
			onClick={() => props.onClick()}
		>
			<Show when={props.step} fallback='Watch'>
				{(step) => (
					<>
						<SyncIcon fraction={null} running label={step().title} />
						<span class='watch-step'>{step().label}</span>
					</>
				)}
			</Show>
		</button>
	)
}

/** `51m game · 37m wall`, or `null` for a game left before it ended. */
function lengthOf(replay: ReplayView): string | null {
	if (replay.gameSeconds === null || replay.wallSeconds === null) return null
	const minutes = (seconds: number) => span(Math.floor(seconds / 60))
	return `${minutes(replay.gameSeconds)} game · ${minutes(replay.wallSeconds)} wall`
}

/**
 * The replays on this machine.
 *
 * A data directory holds thousands, so the list is built from their names —
 * which carry the date, map and engine — and the few bytes of header that hold
 * the game's length; the rest of each file is never read to draw it. Playing
 * one hands the engine the file where it would otherwise be handed a
 * `spring://` URL.
 */
export function Replays() {
	const [search, setSearch] = createSignal('')
	const [saving, setSaving] = createSignal<ReplayView | null>(null)
	let scrollRef: HTMLDivElement | undefined

	// `when` is the local time, formatted once here rather than on every
	// keystroke of the search across thousands of rows.
	const [replays, { refetch }] = createResource(async () => {
		try {
			const found = await api.listReplays()
			return found.map((replay) => ({
				...replay,
				when: exactly(replay.playedAt),
				length: lengthOf(replay),
			}))
		} catch (error) {
			pushNotice('warning', describeError(error))
			return []
		}
	})

	const rows = createMemo(() => {
		const words = search().toLowerCase().split(/\s+/).filter(Boolean)
		const all = replays() ?? []
		if (words.length === 0) return all
		return all.filter((replay) => {
			const haystack =
				`${replay.map} ${replay.when} ${replay.engine}`.toLowerCase()
			return words.every((word) => haystack.includes(word))
		})
	})

	const virtualizer = createVirtualizer({
		get count() {
			return rows().length
		},
		getScrollElement: () => scrollRef ?? null,
		estimateSize: () => ROW_HEIGHT,
		overscan: 10,
	})

	// A Watch is one request from the click to the replay starting: the
	// engine is fetched first, then the runtime fetches the game and map and
	// answers once the replay is up. `watching` is the replay it is for.
	const [watching, setWatching] = createSignal<string | null>(null)
	const [engine, setEngine] = createSignal<EngineProgress | null>(null)
	onMount(() => {
		const pending = listen<EngineProgress>('engine-download', (event) =>
			setEngine(event.payload),
		)
		onCleanup(() => void pending.then((unlisten) => unlisten()))
	})

	async function play(replay: ReplayView) {
		if (watching() !== null) return
		setEngine(null)
		setWatching(replay.path)
		try {
			await api.playReplay(replay.path)
		} catch (error) {
			pushNotice('warning', describeError(error))
		} finally {
			setWatching(null)
		}
	}

	return (
		<section class='battles'>
			<header class='toolbar'>
				<input
					class='search'
					placeholder='Search map, date, engine'
					value={search()}
					onInput={(e) => setSearch(e.currentTarget.value)}
				/>
				<button onClick={() => void refetch()}>Rescan</button>
				<button
					type='button'
					title='Open the folder new games record their replays in'
					onClick={() =>
						void api
							.openReplaysDir()
							.catch((error) =>
								pushNotice(
									'warning',
									`replays folder: ${describeError(error)}`,
								),
							)
					}
				>
					Replays folder
				</button>
				<span class='spacer' />
				<span class='muted count'>
					{rows().length} replays
					<Show when={rows().length !== (replays()?.length ?? 0)}>
						{' '}
						of {replays()?.length}
					</Show>
				</span>
			</header>

			<div class='list' ref={scrollRef}>
				<Show
					when={rows().length > 0}
					fallback={
						<p class='muted empty-list'>
							<Show
								when={(replays()?.length ?? 0) > 0}
								fallback='No replays in the BAR data directory yet.'
							>
								Nothing matches that search.
							</Show>
						</p>
					}
				>
					<div
						style={{
							height: `${virtualizer.getTotalSize()}px`,
							position: 'relative',
						}}
					>
						<For each={virtualizer.getVirtualItems()}>
							{(item) => {
								const replay = () => rows()[item.index]
								return (
									<Show when={replay()}>
										{(r) => (
											<div
												class='replay-row'
												style={{
													position: 'absolute',
													top: `${item.start}px`,
													height: `${ROW_HEIGHT}px`,
													width: '100%',
												}}
												onDblClick={() => void play(r())}
											>
												<span class='col-when'>{r().when}</span>
												<span class='col-map' title={r().map}>
													{r().map}
												</span>
												<span
													class='col-length'
													title={
														r().length === null
															? 'Left before the game ended; the engine records no length then'
															: 'Game time stops for pauses and counts the whole game even for a late joiner; wall time is the clock'
													}
												>
													{r().length ?? '–'}
												</span>
												<span class='col-engine'>{r().engine}</span>
												<span class='col-size'>
													{Math.round(r().bytes / 1024)} kB
												</span>
												{/* Every replay carries the start script that made
                            the game, which is a whole room setup — and the
                            only way to get one from a game you were not in. */}
												<button
													title="Save this game's settings as a preset"
													onClick={() => setSaving(r())}
												>
													To preset
												</button>
												<WatchButton
													step={
														watching() === r().path
															? watchStep(r().engine, engine(), lobby.download)
															: null
													}
													disabled={
														lobby.engine.state === 'running' ||
														watching() !== null
													}
													onClick={() => void play(r())}
												/>
											</div>
										)}
									</Show>
								)
							}}
						</For>
					</div>
				</Show>
			</div>

			<Show when={saving()}>
				{(replay) => (
					<Ask
						title='Save as preset'
						hint={`The map, the modoptions and the start boxes from ${replay().map}.`}
						initial={`${replay().map} ${exactly(replay().playedAt)}`}
						confirm='Save'
						onCancel={() => setSaving(null)}
						onAnswer={(name) => {
							const path = replay().path
							setSaving(null)
							void api
								.presetFromReplay(path, name)
								.then(() =>
									pushNotice(
										'info',
										`saved ${name}; it is under Presets in the room`,
									),
								)
								.catch((error) => pushNotice('warning', describeError(error)))
						}}
					/>
				)}
			</Show>
		</section>
	)
}
