import { Select } from '../components/Select'
import {
	For,
	Index,
	Match,
	createEffect,
	Show,
	Switch,
	createMemo,
	createResource,
	createSignal,
	type Accessor,
} from 'solid-js'
import { ResizeHandle } from '../components/ResizeHandle'
import { SearchBox } from '../components/SearchBox'
import { api, describeError } from '../ipc/client'
import {
	clamp,
	dragWidth,
	localStore,
	readWidth,
	writeWidth,
} from '../lib/resize'
import {
	ALL_TAB,
	EVERY_ROW,
	GENERAL_GROUP,
	MAP_TAB,
	MODDING_TAB,
	byRunOrder,
	changedByTab,
	changedCount,
	changedView,
	defaultText,
	displayText,
	isOn,
	isMapOption,
	readModOptions,
	label,
	rowsByGroup,
	rowsByTab,
	rowsOf,
	searchRows,
	tabs,
	type Changed,
	type Row,
	type Section,
	type Tab,
} from '../lib/setup'
import { SCRATCH, SLOT_KEYS, isDirty, slotOf, titleOf } from '../lib/tweakspace'
import { hostsMods } from '../lib/mutators'
import { pushNotice } from '../store/chat'
import { tweakspaceFor } from '../store/tweakspaceInstance'
import { PasteBanner } from './PasteBanner'
import { Mods } from './Mods'
import { Presets } from './Presets'
import { useRoom, type RoomModel } from './room/model'
import { canSet, setRefusal } from './room/move'
import { TweakRow } from './tweaks/TweakRow'
import { Tweaks } from './tweaks/Tweaks'

const TWEAK_GROUP = 'Tweak slots'

/** Shown before the game is installed, when there is no table to read. */
const NO_TABS: Tab = { key: '', name: '', desc: '', groups: [] }

/** The tab across all tabs; `groups` is empty because it draws its own body. */
const ALL: Tab = {
	key: ALL_TAB,
	name: 'All',
	desc: "Every setting that differs from BAR's default, whichever tab it is in.",
	groups: [],
}

/** The All tab's sections: one per tab, headed by the tab's name. */
const byTabName = (entries: Changed[]): Section[] =>
	entries.map(({ tab, rows }) => ({ name: tab.name, rows }))

const WIDTH_KEY = 'modlobby.setupWidth'
const NARROWEST = 420
/** What the rosters and the chat keep, however wide the pane is dragged. */
const ROOM_KEEPS = 480
/** `.setup-tabs`' side padding and the gap between tabs, as the stylesheet has them. */
const STRIP_PADDING = 14
const TAB_GAP = 2

/**
 * The room's settings, in BAR's own tabs and groups.
 *
 * Opens on All -- what differs from BAR's default, across every tab -- and
 * keeps the rest one click away: 221 options ship across these tabs, and the
 * four somebody changed are the four that decide how the game plays.
 * Everything is read-only until we hold a seat — SPADS grants `bSet` as
 * `battle,pv:player:stopped`, so a spectator can neither set a value nor call
 * a vote on one.
 *
 * The pane carries its own width: dragged by the grip on its left edge and
 * remembered, or, the first time, measured so that its tabs sit on one row.
 *
 * Presets share the pane as a second face of it: they are read from and
 * written to this room, so beside the settings is where they belong. Mods
 * are its third, in a room whose host runs them.
 */
export function Setup() {
	const room = useRoom()
	const [chosenPane, setPane] = createSignal<'setup' | 'presets' | 'mods'>(
		'setup',
	)
	/** A room whose host runs mods, or that loads any: the ones with the third face. */
	const modded = () =>
		hostsMods(room.my()?.scriptTags) || room.check().mutators.length > 0
	const pane = () =>
		chosenPane() === 'mods' && !modded() ? 'setup' : chosenPane()

	/**
	 * The game's own option table, read from the copy installed on this machine
	 * rather than shipped with the app — see `lib/setup`. Re-read when the room
	 * changes game, which is also what keeps it matching the version in play.
	 */
	const [catalogue] = createResource(
		() => room.battle()?.gameName,
		(game) => api.gameModOptions(game).catch(() => []),
	)
	const space = tweakspaceFor(room)
	/**
	 * Tweak slots past the twenty written from here that this room has used.
	 * BAR reads any index; these are shown, never written. Equal while the
	 * same slots, so the tabs are not remade on every room update.
	 */
	const extraSlots = createMemo(
		() =>
			Object.values(space.ws.docs)
				.filter(
					(doc) => doc.origin === 'slot' && !SLOT_KEYS.includes(doc.title),
				)
				.map((doc) => doc.title)
				.sort(byRunOrder),
		undefined,
		{ equals: (a, b) => a.join() === b.join() },
	)
	const TABS = createMemo(() => tabs(catalogue() ?? [], extraSlots()))

	/**
	 * The chosen tab is held as a key rather than as the tab itself: the table
	 * is re-read when the room changes game, and a held object would then be a
	 * tab from the previous catalogue.
	 */
	const [tabKey, setTabKey] = createSignal<string>(ALL_TAB)
	const tab = (): Tab => {
		if (tabKey() === ALL_TAB) return ALL
		return (
			TABS().find((entry) => entry.key === tabKey()) ?? TABS()[0] ?? NO_TABS
		)
	}
	const [group, setGroup] = createSignal<string | null>(null)

	/**
	 * The drafts editor fills the pane while it is open. Anything that picks
	 * something else to look at -- a tab, a group, its own back button --
	 * closes it.
	 */
	const [drafting, setDrafting] = createSignal(false)
	function openDrafts() {
		space.expand(null)
		space.open(SCRATCH)
		setDrafting(true)
	}
	function closeDrafts() {
		space.setFullscreen(false)
		setDrafting(false)
	}

	const values = createMemo(() => readModOptions(room.my()?.scriptTags))

	const editable = createMemo(() => canSet(room))

	const everywhere = createMemo(() => changedByTab(TABS(), values()))
	const total = createMemo(() =>
		everywhere().reduce((sum, entry) => sum + entry.rows.length, 0),
	)

	/**
	 * What a Changed view keeps though it is not a change: the open slot and
	 * any holding an unsent edit, so neither vanishes when somebody clears it.
	 * Equal while the same slots are in it, so typing does not redo the rows.
	 */
	const held = createMemo(
		() =>
			new Set(
				Object.values(space.ws.docs)
					.filter(
						(doc) =>
							doc.origin === 'slot' &&
							(doc.id === space.ws.expanded || isDirty(doc)),
					)
					.map((doc) => titleOf(doc.id)),
			),
		undefined,
		{
			equals: (a, b) => a.size === b.size && [...a].every((key) => b.has(key)),
		},
	)
	const changedRows = () => changedView(values(), held())

	const shown = createMemo(() => {
		const name = group()
		const found = tab().groups.find((entry) => entry.name === name)
		return found ? rowsOf(found, values()) : []
	})

	/**
	 * A search across every tab. While it holds something the body is the
	 * matches, under the tab each lives in; the tab and group that were open
	 * are untouched, so clearing it lands where you were.
	 */
	const [needle, setNeedle] = createSignal('')
	const searching = () => needle().trim() !== ''
	const found = createMemo(() => searchRows(TABS(), values(), needle()))

	/**
	 * Whether the Changed view is showing everything else as well. Reset with
	 * the tab: opening one means landing on what is changed in it.
	 */
	const [unchanged, setUnchanged] = createSignal(false)

	function open(next: Tab) {
		setTabKey(next.key)
		setGroup(null)
		setUnchanged(false)
		closeDrafts()
	}

	const [width, setWidth] = createSignal(readWidth(localStore(), WIDTH_KEY))
	/** Once a width has been chosen by hand, the tabs stop deciding it. */
	let chosen = width() !== null
	let host: HTMLElement | undefined
	let strip: HTMLDivElement | undefined
	const bounds = () => ({
		min: NARROWEST,
		max: Math.max(NARROWEST, window.innerWidth - ROOM_KEEPS),
	})

	/**
	 * As wide as it takes for the tabs and the search to sit on one row: their
	 * widths and the gaps between them, the strip's padding and the pane's
	 * border. Summed from the tabs themselves, so the answer is the same
	 * whether they are currently on one row or wrapped onto two.
	 */
	function fit() {
		if (chosen || !strip) return
		const tabs = [...strip.children] as HTMLElement[]
		if (tabs.length === 0) return
		const wanted =
			tabs.reduce((sum, tab) => sum + tab.offsetWidth, 0) +
			TAB_GAP * (tabs.length - 1) +
			STRIP_PADDING * 2 +
			1
		setWidth(clamp(Math.min(wanted, window.innerWidth / 2), bounds()))
	}

	// Whenever the tabs or their badges change -- and once the display font
	// is in, since a tab measured in the fallback face comes out narrower.
	createEffect(() => {
		TABS()
		total()
		if (chosen) return
		fit()
		void document.fonts?.ready.then(fit)
	})

	return (
		<aside
			class='setup'
			ref={host}
			style={{
				'--setup-width': width() === null ? undefined : `${width()}px`,
			}}
		>
			<ResizeHandle
				label='Resize the setup pane'
				onStart={() => width() ?? host?.getBoundingClientRect().width ?? 0}
				onMove={(start, x0, x) => setWidth(dragWidth(start, x0, x, bounds()))}
				onEnd={() => {
					const now = width()
					if (now === null) return
					chosen = true
					writeWidth(localStore(), WIDTH_KEY, now)
				}}
			/>

			<div class='setup-head'>
				<button
					class='pane-tab'
					classList={{ on: pane() === 'setup' }}
					onClick={() => setPane('setup')}
				>
					Setup
				</button>
				<button
					class='pane-tab'
					classList={{ on: pane() === 'presets' }}
					onClick={() => setPane('presets')}
				>
					Presets
				</button>
				<Show when={modded()}>
					<button
						class='pane-tab'
						classList={{ on: pane() === 'mods' }}
						onClick={() => setPane('mods')}
					>
						Mods
					</button>
				</Show>
				<Show when={pane() === 'setup'}>
					<span class='note'>{noteOfPane(room)}</span>
					<Show when={space.unsent() > 0}>
						<span
							class='doc-tag dirty setup-unsent'
							title='Slots edited here that the room has not been sent'
						>
							{space.unsent()} unsent
						</span>
					</Show>
				</Show>
			</div>

			{/* The paste banner is the chat throttle showing through; nothing is
          throttled where nothing is said. Above the pane switch, because
          loading a preset is a paste too -- and it is the pane you are on
          while you wait for one. */}
			<Show when={room.caps.spads}>
				<PasteBanner />
			</Show>

			<Show
				when={pane() === 'setup'}
				fallback={pane() === 'mods' ? <Mods /> : <Presets />}
			>
				<div class='setup-tabs' ref={strip}>
					<button
						class='setup-tab'
						classList={{ on: !searching() && tab().key === ALL_TAB }}
						title={ALL.desc}
						onClick={() => {
							setNeedle('')
							open(ALL)
						}}
					>
						{ALL.name}
						<Show when={total() > 0}>
							<span class='badge'>{total()}</span>
						</Show>
					</button>
					<For each={TABS()}>
						{(entry) => {
							const count = createMemo(() => changedCount(entry, values()))
							return (
								<button
									class='setup-tab'
									classList={{
										on: !searching() && tab().key === entry.key,
										ours: entry.key === MODDING_TAB || entry.key === MAP_TAB,
									}}
									title={entry.desc}
									onClick={() => {
										setNeedle('')
										open(entry)
									}}
								>
									{entry.name}
									<Show when={count() > 0}>
										<span class='badge'>{count()}</span>
									</Show>
								</button>
							)
						}}
					</For>
					<Show when={!drafting()}>
						<SearchBox
							class='setup-search'
							placeholder='Search settings'
							value={needle()}
							onInput={setNeedle}
						/>
					</Show>
				</div>

				<Show
					when={!drafting()}
					fallback={<Tweaks drafts onClose={closeDrafts} />}
				>
					<Show
						when={!searching()}
						fallback={
							<div class='setup-detail setup-found'>
								<Found found={found} needle={needle()} editable={editable()} />
							</div>
						}
					>
						<div class='setup-body'>
							<nav class='groups'>
								<Show
									when={tab().key !== ALL_TAB}
									fallback={
										<For each={everywhere()}>
											{(entry) => (
												<button class='group' onClick={() => open(entry.tab)}>
													{entry.tab.name}
													<span class='c'>{entry.rows.length}</span>
												</button>
											)}
										</For>
									}
								>
									<button
										class='group'
										classList={{ on: group() === null }}
										onClick={() => setGroup(null)}
									>
										Changed
										<span class='c'>{changedCount(tab(), values())}</span>
									</button>
									<For each={tab().groups}>
										{(entry) => (
											<button
												class='group'
												classList={{ on: group() === entry.name }}
												onClick={() => setGroup(entry.name)}
											>
												{entry.name || GENERAL_GROUP}
												<span class='c'>{entry.options.length}</span>
											</button>
										)}
									</For>
								</Show>
							</nav>

							<div class='setup-detail'>
								<Switch>
									<Match when={tab().key === ALL_TAB}>
										<Changes
											changed={() =>
												byTabName(rowsByTab(TABS(), values(), changedRows()))
											}
											all={() =>
												byTabName(rowsByTab(TABS(), values(), EVERY_ROW))
											}
											empty="Every setting is on BAR's default."
											unchanged={unchanged()}
											onToggle={() => setUnchanged(!unchanged())}
											editable={editable()}
											onDrafts={openDrafts}
										/>
									</Match>
									<Match when={group() === null}>
										<Changes
											changed={() =>
												rowsByGroup(tab(), values(), changedRows())
											}
											all={() => rowsByGroup(tab(), values(), EVERY_ROW)}
											empty="Every setting in this tab is on BAR's default."
											unchanged={unchanged()}
											onToggle={() => setUnchanged(!unchanged())}
											editable={editable()}
											onDrafts={openDrafts}
										/>
									</Match>
									<Match
										when={tab().key === MODDING_TAB && group() === TWEAK_GROUP}
									>
										<TweakSlots
											rows={shown()}
											editable={editable()}
											onDrafts={openDrafts}
										/>
									</Match>
									<Match when={true}>
										<Rows rows={shown()} editable={editable()} />
									</Match>
								</Switch>
							</div>
						</div>
					</Show>
				</Show>
			</Show>
		</aside>
	)
}

/**
 * What the pane says a change will do, which is not the same sentence when
 * there is a host to persuade and when there is not -- or why it would not
 * be taken at all.
 */
function noteOfPane(room: RoomModel): string {
	const why = setRefusal(room)
	if (why !== null) return why
	if (room.caps.spads) return 'A change is proposed to the host'
	return 'A change takes effect here'
}

/**
 * What is changed, under the heading it belongs to -- and, on request,
 * everything else beside it. On the All tab the headings are tabs; inside a
 * tab they are its groups.
 *
 * The toggle reveals rows in place. It used to open the tab's first group
 * instead, which lost the changed rows it had been showing and, on Modding,
 * landed on the slot grid rather than the rows it had promised.
 *
 * `Index`, not `For`: the rows are made again whenever the room says
 * anything, and a row keyed by identity would be a new row each time --
 * which, for one with the editor open under it, is a new editor.
 */
function Changes(props: {
	changed: Accessor<Section[]>
	all: Accessor<Section[]>
	/** What to say when nothing here is changed. */
	empty: string
	unchanged: boolean
	onToggle: () => void
	editable: boolean
	onDrafts: () => void
}) {
	const shown = () => (props.unchanged ? props.all() : props.changed())
	/** Changed rows, or every row once the unchanged are shown too. */
	const count = (rows: Row[]) =>
		props.unchanged ? rows.length : rows.filter((row) => row.changed).length
	return (
		<>
			<Show
				when={shown().length > 0}
				fallback={<p class='muted setup-empty'>{props.empty}</p>}
			>
				<Index each={shown()}>
					{(entry) => (
						<>
							<SectionHead
								name={entry().name}
								count={count(entry().rows)}
								onDrafts={props.onDrafts}
							/>
							<Rows rows={entry().rows} editable={props.editable} />
						</>
					)}
				</Index>
			</Show>
			<button class='setup-reveal' onClick={props.onToggle}>
				{props.unchanged ? 'Hide unchanged' : 'Show unchanged'}
			</button>
		</>
	)
}

/**
 * A heading over rows, with what it counts. The tweak slots' heading also
 * opens the drafts editor: an unslotted tweak, and the drafts beside it.
 */
function SectionHead(props: {
	name: string
	count: number
	onDrafts: () => void
}) {
	return (
		<div class='setup-section'>
			<span>{props.name}</span>
			<Show when={props.name === TWEAK_GROUP}>
				<button
					class='setup-drafts'
					title='Write a tweak for no slot yet, next to your drafts'
					onClick={props.onDrafts}
				>
					Editor
				</button>
			</Show>
			<span class='count'>{props.count}</span>
		</div>
	)
}

/** What the search turned up, under the tab each row lives in. */
function Found(props: {
	found: Accessor<Changed[]>
	needle: string
	editable: boolean
}) {
	return (
		<Show
			when={props.found().length > 0}
			fallback={
				<p class='muted setup-empty'>
					Nothing matches "{props.needle.trim()}".
				</p>
			}
		>
			<For each={props.found()}>
				{(entry) => (
					<>
						<div class='setup-section'>
							<span>{entry.tab.name}</span>
							<span class='count'>{entry.rows.length}</span>
						</div>
						<Rows rows={entry.rows} editable={props.editable} />
					</>
				)}
			</For>
		</Show>
	)
}

/**
 * Settings as rows. A base64url slot -- a tweak, or the start-box override --
 * is a `TweakRow` wherever it appears: its blob to read or paste over, and the
 * editor one press away.
 *
 * Exported because an AI's own options are the same kind of table -- BAR's
 * `modoptions.lua` and an engine AI's `AIOptions.lua` are one format -- and
 * drawing them the same way is the whole reason they can be. `set` is where
 * a changed value goes; the room's own settings are the default.
 */
export function Rows(props: {
	rows: Row[]
	editable: boolean
	set?: (key: string, value: string) => Promise<void>
}) {
	return (
		<div class='setup-rows'>
			<Show
				when={props.rows.length > 0}
				fallback={<p class='muted setup-empty'>Nothing here.</p>}
			>
				<Index each={props.rows}>
					{(row) => (
						<Show
							when={slotOf(row().option.key) === null}
							fallback={<TweakRow row={row()} />}
						>
							<div class='opt' classList={{ changed: row().changed }}>
								<span class='mark' />
								<span class='k' title={row().option.desc ?? ''}>
									{label(row().option)}
								</span>
								<Switch fallback={<span class='v'>{displayText(row())}</span>}>
									<Match when={isMapOption(row().option)}>
										<MapValue row={row()} />
									</Match>
									<Match when={props.editable}>
										<Control row={row()} set={props.set} />
									</Match>
								</Switch>
							</div>
						</Show>
					)}
				</Index>
			</Show>
		</div>
	)
}

/**
 * A map-metadata row's value in words. The blob is base64url(zlib(json)) and
 * says nothing; Rust decodes it (`boxes::describe_map_option`) and answers
 * with what it holds. These two are the ones SPADS sets from the map's
 * metadata, and read-only; the override is somebody's own, and a `TweakRow`.
 */
function MapValue(props: { row: Row }) {
	const [words] = createResource(
		() => [props.row.option.key, props.row.current ?? ''] as const,
		([key, raw]) => api.describeMapOption(key, raw).catch(() => null),
	)
	return (
		<span class='v' title={props.row.current ?? ''}>
			{words.loading ? '…' : (words() ?? displayText(props.row))}
		</span>
	)
}

/**
 * One editable setting. The value is sent when it is committed, never on
 * every keystroke: each send is a chat command the whole room sees.
 */
function Control(props: {
	row: Row
	set?: (key: string, value: string) => Promise<void>
}) {
	const room = useRoom()
	const value = () => props.row.current ?? defaultText(props.row.option)

	async function set(next: string) {
		if (next === value()) return
		try {
			const put = props.set ?? room.io.setOption
			await put(props.row.option.key, next)
		} catch (error) {
			pushNotice('warning', `${props.row.option.key}: ${describeError(error)}`)
		}
	}

	return (
		<Switch fallback={<span class='v'>{displayText(props.row)}</span>}>
			<Match when={props.row.option.type === 'bool'}>
				<input
					class='v-edit'
					type='checkbox'
					checked={isOn(value())}
					onChange={(event) =>
						void set(event.currentTarget.checked ? '1' : '0')
					}
				/>
			</Match>
			<Match when={props.row.option.type === 'number'}>
				<input
					class='v-edit'
					type='number'
					value={value()}
					min={props.row.option.min ?? undefined}
					max={props.row.option.max ?? undefined}
					step={props.row.option.step ?? undefined}
					onChange={(event) => void set(event.currentTarget.value)}
				/>
			</Match>
			<Match when={props.row.option.type === 'list'}>
				<Select
					class='v-edit'
					value={value()}
					onChange={(event) => void set(event.currentTarget.value)}
				>
					<For each={props.row.option.items ?? []}>
						{(item) => <option value={item.key}>{item.name}</option>}
					</For>
				</Select>
			</Match>
		</Switch>
	)
}

/** The twenty slots as rows, every one of them, filled or not. */
function TweakSlots(props: {
	rows: Row[]
	editable: boolean
	onDrafts: () => void
}) {
	const room = useRoom()
	return (
		<>
			<SectionHead
				name={TWEAK_GROUP}
				count={props.rows.filter((row) => row.changed).length}
				onDrafts={props.onDrafts}
			/>
			<Rows rows={props.rows} editable={props.editable} />
			<Show
				when={room.caps.spads}
				fallback={
					<div class='setup-note'>
						The editor formats and diffs a tweak before it goes in. Nothing
						leaves this machine.
					</div>
				}
			>
				<Show when={!props.editable}>
					<div class='setup-note'>
						The editor still formats and compares a tweak, and copies the
						command for somebody who can set it.
					</div>
				</Show>
			</Show>
		</>
	)
}
