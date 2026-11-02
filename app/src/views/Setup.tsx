import {
	For,
	createEffect,
	Show,
	createMemo,
	createResource,
	createSignal,
	on,
} from 'solid-js'
import { ResizeHandle } from '../components/ResizeHandle'
import { SearchBox } from '../components/SearchBox'
import { Segmented } from '../components/Segmented'
import { Query } from '../components/SettingRow'
import { api } from '../ipc/client'
import { createInspecting } from '../lib/inspecting'
import { createRecent } from '../lib/recent'
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
	MAP_TAB,
	MODDING_TAB,
	byRunOrder,
	changedByTab,
	changedCount,
	changedView,
	drawnOrder,
	keepOrder,
	latestChange,
	linksOf,
	locksOf,
	readModOptions,
	rowsByGroup,
	rowsByTab,
	searchRows,
	arrange,
	tabs,
	voteOn,
	wordsFor,
	DOCUMENTS,
	type Changed,
	type Drawn,
	type Grouping,
	type OptionLink,
	type Row,
	type RowNote,
	type RowOrder,
	type Section,
	type Tab,
} from '../lib/setup'
import { createOverview } from '../lib/overview'
import { sticky } from '../lib/sticky'
import {
	DOC_KEYS,
	SCRATCH,
	isDirty,
	isDocumentKey,
	titleOf,
} from '../lib/tweakspace'
import { tweakspaceFor } from '../store/tweakspaceInstance'
import { PasteBanner } from './PasteBanner'
import { Mods, runsMods } from './Mods'
import { Presets } from './Presets'
import { useRoom, type RoomModel } from './room/model'
import { canSet, setRefusal } from './room/move'
import { Inspector } from './setup/Inspector'
import { LinkButton, Rows } from './setup/Rows'
import { Tweaks } from './tweaks/Tweaks'

const TWEAK_GROUP = 'Tweak slots'

/** Shown before the game is installed, when there is no table to read. */
const NO_TABS: Tab = { key: '', name: '', desc: '', groups: [], intro: [] }

/** The tab across all tabs; `groups` is empty because it draws its own body. */
const ALL: Tab = {
	key: ALL_TAB,
	name: 'All',
	desc: "Every setting that differs from BAR's default, whichever tab it is in.",
	groups: [],
	intro: [],
}

/** The All tab's sections: one per tab, headed by the tab's name. */
const byTabName = (entries: Changed[]): Section[] =>
	entries.map(({ tab, rows }) => ({ name: tab.name, rows }))

const WIDTH_KEY = 'modlobby.setupWidth'
const SHOW_KEY = 'modlobby.setupShow'
const ORDER_KEY = 'modlobby.setupOrder'
const GROUP_KEY = 'modlobby.setupGroup'
/** How far below the list's top a heading has to have scrolled to be the one being read. */
const READ_LINE = 32

const sameKeys = (a: ReadonlySet<string>, b: ReadonlySet<string>) =>
	a.size === b.size && [...a].every((key) => b.has(key))

type ShowRows = 'changed' | 'all'

const SHOW = [
	{
		value: 'changed',
		label: 'Changed',
		title: "What differs from BAR's default",
	},
	{ value: 'all', label: 'All', title: 'Every setting' },
] as const

const GROUPS = [
	{ value: 'none', label: 'None', title: 'One list, no headings' },
	{ value: 'section', label: 'Section', title: "Under BAR's own headings" },
	{
		value: 'type',
		label: 'Type',
		title: 'Switches, choices, numbers, text and documents apart',
	},
] as const
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
 * are its third: the room's own where its host runs them, and elsewhere the
 * way to rooms that do.
 */
export function Setup() {
	const room = useRoom()
	const [pane, setPane] = createSignal<'setup' | 'presets' | 'mods'>('setup')
	// A room whose host runs mods opens on them: they are what the room is
	// about, and why a room on the mods server was taken. Once, as the host
	// says so, since its tags land after the room does.
	createEffect(
		on(
			() => runsMods(room),
			(runs, ran) => {
				if (runs && !ran) setPane('mods')
			},
		),
	)

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
				.filter((doc) => doc.origin === 'slot' && !DOC_KEYS.includes(doc.title))
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

	/**
	 * The drafts editor fills the pane while it is open. Anything that picks
	 * something else to look at -- a tab, its own back button -- closes it.
	 */
	const drafting = () => space.ws.desk
	const openDrafts = () => space.openDesk(SCRATCH)
	const closeDrafts = () => space.closeDesk()

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
		{ equals: sameKeys },
	)
	/** The row a press or a pointer is on; an open editor is not its row to pick. */
	const keyAt = (target: EventTarget | null): string | null => {
		const element = target as Element
		if (element.closest('.tweak-inline')) return null
		return element.closest<HTMLElement>('[data-key]')?.dataset.key ?? null
	}

	/** Which row the strip under the list describes. */
	const inspecting = createInspecting()

	/**
	 * The row the pointer rests on, and the list as it was drawn when it came
	 * to rest there. While it rests the order holds and the row stays shown:
	 * a change made to the row would otherwise sort it away from under the
	 * hand, or drop it from a Changed view it no longer belongs in. Moving
	 * off the row lets the list settle.
	 */
	const [resting, setResting] = createSignal<{
		key: string
		drawn: Drawn
	} | null>(null)
	function rest(event: MouseEvent) {
		const key =
			(event.target as Element).closest<HTMLElement>('[data-key]')?.dataset
				.key ?? null
		inspecting.point(keyAt(event.target))
		if (key === (resting()?.key ?? null)) return
		setResting(key === null ? null : { key, drawn: drawnOrder(listed()) })
	}
	const kept = createMemo(
		() => {
			const keys = new Set(held())
			const rest = resting()
			if (rest !== null) keys.add(rest.key)
			return keys
		},
		undefined,
		{ equals: sameKeys },
	)
	const changedRows = () => changedView(values(), kept())

	/**
	 * A search across every tab. While it holds something the list is the
	 * matches, under the tab each lives in; the tab that was open is
	 * untouched, so clearing it lands where you were.
	 */
	const [needle, setNeedle] = createSignal('')
	const searching = () => needle().trim() !== ''
	const found = createMemo(() => searchRows(TABS(), values(), needle()))
	createEffect(on(needle, () => list?.scrollTo?.({ top: 0 }), { defer: true }))

	/**
	 * What the rows show and in what order: one choice each for every tab,
	 * kept between sessions. A stored value this build does not know reads as
	 * the default.
	 */
	const [show, setShow] = sticky<ShowRows>(SHOW_KEY, 'changed')
	const [order, setOrder] = sticky<RowOrder>(ORDER_KEY, 'changed')
	const [grouped, setGrouped] = sticky<Grouping>(GROUP_KEY, 'section')
	const shownRows = (): ShowRows => (show() === 'all' ? 'all' : 'changed')
	const rowOrder = (): RowOrder => (order() === 'name' ? 'name' : 'changed')
	const grouping = (): Grouping =>
		grouped() === 'none' || grouped() === 'type' ? grouped() : 'section'

	/** The rows Show lets through. */
	const filter = () => (shownRows() === 'all' ? EVERY_ROW : changedRows())
	/** Sections as Group and Sort have them. */
	const arranged = (sections: Section[]) =>
		arrange(sections, grouping(), rowOrder())

	/** The list: what a search found, the All tab's tabs or a tab's groups, as Show, Group and Sort have them. */
	const listed = createMemo(() => {
		if (searching()) return arranged(byTabName(found()))
		return arranged(
			tab().key === ALL_TAB
				? byTabName(rowsByTab(TABS(), values(), filter()))
				: rowsByGroup(tab(), values(), filter()),
		)
	})
	/** The list as drawn: `listed`, in the order it had while the pointer rests on a row. */
	const shown = createMemo(() => {
		const rest = resting()
		return rest === null ? listed() : keepOrder(listed(), rest.drawn)
	})
	/** The list's headings, which the overview beside it names. A search has no overview. */
	const headings = createMemo(
		() =>
			searching()
				? []
				: shown()
						.map((section) => section.name)
						.filter((name) => name !== ''),
		undefined,
		{ equals: (a, b) => a.join('\n') === b.join('\n') },
	)

	/** The row the strip describes, as drawn right now; none once it has gone from the list. */
	const inspected = createMemo(() => {
		const key = inspecting.key()
		if (key === null) return null
		for (const section of shown()) {
			const row = section.rows.find((entry) => entry.option.key === key)
			if (row) return row
		}
		return null
	})

	const history = () => room.my()?.history ?? []
	const recent = createRecent(history)
	const locks = createMemo(() => locksOf(catalogue() ?? [], values()))
	const links = createMemo(() => linksOf(catalogue() ?? []))
	const tabLinks = () => links().bySection.get(tab().key) ?? []

	/** What the pane knows about a row beyond its value. */
	const noteOf = (row: Row): RowNote => {
		const { key } = row.option
		const proposal = voteOn(room.my()?.vote, key)
		return {
			recent: recent().has(key),
			vote:
				proposal === null
					? null
					: isDocumentKey(key)
						? ''
						: wordsFor(row.option, proposal.value),
			lock: locks().get(key) ?? null,
			on: inspecting.pinned() === key,
			links: links().byKey.get(key) ?? [],
		}
	}

	/** The list's own scrolling page, which the overview reads and moves. */
	let list: HTMLDivElement | undefined
	const sectionNamed = (name: string) =>
		[...(list?.querySelectorAll<HTMLElement>('[data-section]') ?? [])].find(
			(section) => section.dataset.section === name,
		)
	const overview = createOverview({
		page: () => list,
		ids: headings,
		start: sectionNamed,
		sectionOf: (target) =>
			target.closest<HTMLElement>('[data-section]')?.dataset.section || null,
		line: READ_LINE,
	})

	/** Sort's choices: with only changes shown, "changed first" comes to BAR's order. */
	const orders = () =>
		[
			{
				value: 'changed',
				label: 'Changed first',
				title:
					shownRows() === 'changed'
						? "Everything shown differs from BAR's default, so this is BAR's own order"
						: "What differs from BAR's default on top, the rest in BAR's order",
			},
			{ value: 'name', label: 'A–Z', title: 'By name' },
		] as const

	/**
	 * The foot of the list. A choice that does not apply to what is open is
	 * greyed with the reason, never taken away.
	 */
	const showFixed = () =>
		searching() ? 'A search looks through every setting' : null
	const viewBar = () => (
		<div class='setup-bar'>
			<span class='setup-bar-pair'>
				<span class='setup-bar-label'>Show</span>
				<Segmented
					label='Show'
					value={shownRows()}
					options={SHOW}
					onChange={setShow}
					disabled={showFixed() !== null}
					title={showFixed() ?? undefined}
				/>
			</span>
			<span class='setup-bar-pair'>
				<span class='setup-bar-label'>Sort</span>
				<Segmented
					label='Sort'
					value={rowOrder()}
					options={orders()}
					onChange={setOrder}
				/>
			</span>
			<span class='setup-bar-pair'>
				<span class='setup-bar-label'>Group</span>
				<Segmented
					label='Group'
					value={grouping()}
					options={GROUPS}
					onChange={setGrouped}
				/>
			</span>
		</div>
	)
	function open(next: Tab) {
		setTabKey(next.key)
		list?.scrollTo?.({ top: 0 })
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
	function fitTabs() {
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
		fitTabs()
		void document.fonts?.ready.then(fitTabs)
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
					Options
				</button>
				<button
					class='pane-tab'
					classList={{ on: pane() === 'presets' }}
					onClick={() => setPane('presets')}
				>
					Presets
				</button>
				<button
					class='pane-tab'
					classList={{ on: pane() === 'mods' }}
					onClick={() => setPane('mods')}
				>
					Mods
				</button>
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
					<div class='setup-body' classList={{ bare: headings().length === 0 }}>
						{/* The list at a glance, as Settings has its page: every
							    heading in it, the one being read marked, and a press
							    to go there. Nothing to name when nothing is headed. */}
						<Show when={headings().length > 0}>
							<nav class='groups' aria-label='Sections'>
								<For each={headings()}>
									{(name) => (
										<button
											type='button'
											class='group'
											classList={{ on: overview.marked() === name }}
											aria-current={
												overview.marked() === name ? 'location' : undefined
											}
											onClick={() => overview.jump(name, 'smooth')}
										>
											{name}
										</button>
									)}
								</For>
							</nav>
						</Show>

						<div class='setup-list' onMouseLeave={() => inspecting.leave()}>
							<Query.Provider value={needle}>
								<div
									class='setup-detail'
									ref={list}
									onScroll={overview.spy}
									onScrollEnd={overview.settle}
									onWheel={overview.settle}
									onMouseOver={(event) => {
										overview.point(event)
										rest(event)
									}}
									onMouseLeave={() => {
										overview.leave()
										setResting(null)
										inspecting.stop()
									}}
									onClick={(event) => {
										const key = keyAt(event.target)
										if (key === null) return
										const control = (event.target as Element).closest(
											'input, select, button, a',
										)
										if (control) inspecting.pin(key)
										else inspecting.toggle(key)
									}}
									onFocusIn={(event) => {
										const key = keyAt(event.target)
										if (key !== null) inspecting.pin(key)
									}}
								>
									<Show
										when={
											!searching() &&
											(tab().intro.length > 0 || tabLinks().length > 0)
										}
									>
										<TabIntro lines={tab().intro} links={tabLinks()} />
									</Show>
									<Sections
										sections={shown()}
										empty={
											searching()
												? `Nothing matches "${needle().trim()}".`
												: tab().key === ALL_TAB
													? "Every setting is on BAR's default."
													: "Every setting in this tab is on BAR's default."
										}
										editable={editable()}
										onDrafts={openDrafts}
										note={noteOf}
									/>
								</div>
								{/* An open editor is the row's own detail, and wants the room. */}
								<Show when={space.ws.expanded === null}>
									<Inspector
										row={inspected()}
										fallback={
											tab().desc ||
											'Rest the pointer on a setting to read about it.'
										}
										refusal={setRefusal(room)}
										change={latestChange(
											history(),
											inspected()?.option.key ?? '',
										)}
										vote={room.my()?.vote ?? null}
										lock={locks().get(inspected()?.option.key ?? '') ?? null}
										links={
											links().byKey.get(inspected()?.option.key ?? '') ?? []
										}
									/>
								</Show>
							</Query.Provider>
							{viewBar()}
						</div>
					</div>
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
 * What BAR writes at the head of a tab, a warning reading as one, and any link
 * about the tab as a whole. Those go by its first line, which says what the tab is.
 */
function TabIntro(props: { lines: string[]; links: OptionLink[] }) {
	const icons = () => (
		<For each={props.links}>{(link) => <LinkButton link={link} />}</For>
	)
	return (
		<div class='setup-intro'>
			<Show when={props.lines.length === 0}>
				<p>{icons()}</p>
			</Show>
			<For each={props.lines}>
				{(line, at) => (
					<p classList={{ 'setup-note': line.startsWith('Warning') }}>
						{line}
						<Show when={at() === 0}>{icons()}</Show>
					</p>
				)}
			</For>
		</div>
	)
}

/**
 * Rows under their headings: on the All tab and in a search the tabs, inside a
 * tab its groups -- or by type, or no headings at all, as Group has them. A
 * section with no name is drawn without a heading.
 *
 * Showing all reveals rows in place. It used to open the tab's first group
 * instead, which lost the changed rows it had been showing and, on Modding,
 * landed on the slot grid rather than the rows it had promised.
 *
 * Headings are kept by name, as `Rows` keeps rows by key: the sections are
 * made again whenever the room says anything, and sorting moves them.
 */
function Sections(props: {
	sections: Section[]
	/** What to say when there is nothing to draw. */
	empty: string
	editable: boolean
	onDrafts: () => void
	note: (row: Row) => RowNote
}) {
	const byName = createMemo(
		() => new Map(props.sections.map((section) => [section.name, section])),
	)
	return (
		<Show
			when={props.sections.length > 0}
			fallback={<p class='muted setup-empty'>{props.empty}</p>}
		>
			<For each={props.sections.map((section) => section.name)}>
				{(name) => (
					<Show when={byName().get(name)}>
						{(entry) => (
							<div class='setup-group' data-section={name || undefined}>
								<Show when={name !== ''}>
									<SectionHead name={name} onDrafts={props.onDrafts} />
								</Show>
								<Rows
									rows={entry().rows}
									editable={props.editable}
									note={props.note}
								/>
							</div>
						)}
					</Show>
				)}
			</For>
		</Show>
	)
}

/**
 * A heading over rows. The tweak slots' heading -- the documents', when rows
 * go by type -- also opens the drafts editor: an unslotted tweak, and the
 * drafts beside it.
 */
function SectionHead(props: { name: string; onDrafts: () => void }) {
	return (
		<div class='setup-section'>
			<span>{props.name}</span>
			<Show when={props.name === TWEAK_GROUP || props.name === DOCUMENTS}>
				<button
					class='setup-drafts'
					title='Write a tweak for no slot yet, next to your drafts'
					onClick={props.onDrafts}
				>
					Editor
				</button>
			</Show>
		</div>
	)
}
