import {
	For,
	Show,
	createMemo,
	createSignal,
	onCleanup,
	onMount,
} from 'solid-js'
import { dropModel, type monaco } from '../../editor/monaco'
import type { Kind } from '../../ipc/bindings/Kind'
import { unknownUnits } from '../../lib/assist'
import { describeError } from '../../ipc/client'
import { when } from '../../lib/presets'
import {
	KINDS,
	compareChange,
	defaultCompare,
	draftId,
	draftNameFor,
	isDirty,
	resolveSide,
	searchSlots,
	sideOptions,
	slotKey,
	targetOf,
	type DocId,
	type Hit,
	type Side,
} from '../../lib/tweakspace'
import { pushNotice } from '../../store/chat'
import { tweakspaceFor } from '../../store/tweakspaceInstance'
import { useRoom } from '../room/model'
import { setRefusal } from '../room/move'
import { VoteDiff } from '../VoteDiff'
import { ComparePane, type SideText } from './ComparePane'
import { DocList } from './DocList'
import { EditorHost } from './EditorHost'
import { Outline } from './Outline'
import { Problems } from './Problems'
import { SearchPanel } from './SearchPanel'
import { SendBar, Toolbar, type Copyable } from './Toolbar'

/** What the notice calls what was copied. */
function copied(what: Copyable, kind: Kind): string {
	const { text, blob } = KINDS[kind]
	return {
		lua: text,
		minified: `Minified ${text}`,
		blob,
		command: '!bSet command',
	}[what]
}

/**
 * What takes Escape for itself before the window may: Monaco's widgets, a
 * menu, the search while it is being typed in.
 */
const WANTS_ESCAPE =
	'.tweak-full :is(.monaco-editor :is(.suggest-widget, .find-widget, .parameter-hints-widget, .rename-box).visible, .tweak-menu-list, .tweak-search:focus-within)'

/**
 * The workspace, composed: the bar, the editor, what the room is doing to the
 * open slot, and the send bar under it all. In the drafts editor the list of
 * drafts sits beside it; under a settings row the row is the heading. Reads
 * the store; everything below it gets props.
 */
export function Workspace(props: { drafts: boolean; onClose?: () => void }) {
	const room = useRoom()
	const space = tweakspaceFor(room)
	const [busy, setBusy] = createSignal(false)
	const doc = space.active
	const jump = (line: number, column = 1) =>
		space.setGoto({ id: doc().id, line, column, length: 0 })

	/** Whichever editor is showing -- the one, or Compare's right side. */
	let editor: monaco.editor.ICodeEditor | undefined
	const palette = () => {
		editor?.focus()
		void editor?.getAction('editor.action.quickCommand')?.run()
	}

	// ---- searching every tweak ----

	const found = createMemo(() =>
		space.ws.search.open ? searchSlots(space.ws, space.ws.search.query) : [],
	)
	let searchInput: HTMLInputElement | undefined
	/** Ctrl+Shift+F and the bar's button alike: open it, or put it away. */
	function toggleSearch() {
		if (space.ws.search.open) return closeSearch()
		space.setSearch({ open: true })
		searchInput?.focus()
		searchInput?.select()
	}
	function closeSearch() {
		space.setSearch({ open: false })
		editor?.focus()
	}
	/**
	 * A match, gone to. In another tweak it opens that one -- its own row, or
	 * in the drafts editor in place -- and the editor there goes to it.
	 */
	function pick(id: DocId, hit: Hit) {
		if (id !== space.ws.active) {
			if (props.drafts) space.open(id)
			else space.expand(id)
		}
		space.setGoto({
			id,
			line: hit.line,
			column: hit.column,
			length: hit.length,
		})
	}

	/** Why the room would refuse a change from us, if it would. */
	const refusal = createMemo(() => setRefusal(room))

	/** The vote in progress, when it proposes the open slot. */
	const proposal = createMemo(() => {
		const vote = room.my()?.vote
		const open = doc()
		if (vote?.proposal.type !== 'setOption' || open.origin !== 'slot')
			return null
		return vote.proposal.key === open.title ? vote.proposal.value : null
	})

	const history = () => room.my()?.history ?? []

	// A minute is the resolution `when` writes, so it is the rate it needs.
	const [now, setNow] = createSignal(Date.now())
	const tick = setInterval(() => setNow(Date.now()), 30_000)
	onCleanup(() => clearInterval(tick))

	/** Unit keys this game does not have -- only meaningful in a units table. */
	const warnings = createMemo(() =>
		doc().kind === 'units'
			? unknownUnits(space.check()?.outline ?? [], space.assist().units)
			: [],
	)
	const changes = createMemo(() => {
		const open = doc()
		if (open.origin !== 'slot') return []
		return history()
			.filter((change) => change.key === open.title)
			.reverse()
	})

	/** Drafts of the open document's kind: a units table is no defs tweak. */
	const drafts = createMemo(() =>
		Object.values(space.ws.docs)
			.filter((entry) => entry.origin === 'draft' && entry.kind === doc().kind)
			.map((entry) => ({ title: entry.title, name: entry.name }))
			.sort((a, b) => a.title.localeCompare(b.title)),
	)

	/** A side's text, decoding a blob on the way. */
	async function resolve(side: Side): Promise<SideText | null> {
		const found = resolveSide(space.ws, side, history(), proposal())
		if (!found) return null
		if ('lua' in found)
			return { label: found.label, kind: found.kind, text: found.lua }
		const view = await space.decode(found.blob, found.kind).catch(() => null)
		return {
			label: found.label,
			kind: found.kind,
			text: view?.formatted ?? found.blob,
		}
	}

	const toggleCompare = () =>
		space.setCompare(
			space.ws.compare ? null : defaultCompare(space.ws, history()),
		)

	async function act(what: string, run: () => Promise<void>) {
		setBusy(true)
		try {
			await run()
		} catch (error) {
			pushNotice('warning', `${what}: ${describeError(error)}`)
		} finally {
			setBusy(false)
		}
	}

	const copy = (what: Copyable) =>
		act('copy', async () => {
			const ready = space.prepared()
			const text = {
				lua: doc().buffer,
				minified: ready?.minified,
				blob: ready?.blob,
				command: ready?.command,
			}[what]
			if (text === undefined) return
			await navigator.clipboard.writeText(text)
			pushNotice('info', `${copied(what, doc().kind)} copied`)
		})

	const save = (name: string) =>
		act('save draft', async () => {
			await space.saveDraft(name)
			pushNotice('info', `saved draft "${name}"`)
		})

	const send = (direct: boolean) =>
		act('send', async () => {
			const slot = targetOf(space.ws)
			const out = await space.send(direct)
			if (out && slot)
				pushNotice(
					'info',
					`sent ${out.gauge.command} chars to ${slotKey(slot)}`,
				)
		})

	/** What Ctrl+Enter does: the send bar's own button, when it could be pressed. */
	function sendNow() {
		const ready = space.prepared()
		const open = doc()
		if (busy() || refusal() !== null || ready === null) return
		if (open.origin === 'slot' && !isDirty(open)) return
		if (room.caps.spads && !ready.gauge.fits) return
		void send(true)
	}

	const remove = (name: string) =>
		act('delete draft', async () => {
			await space.deleteDraft(name)
			dropModel(draftId(name))
		})

	// Escape leaves fullscreen. Caught before the overlay's own Escape handler
	// and left alone when a Monaco widget or a menu is open and wants it.
	onMount(() => {
		const keys = (event: KeyboardEvent) => {
			if (event.key !== 'Escape' || !space.ws.fullscreen) return
			if (event.defaultPrevented || document.querySelector(WANTS_ESCAPE)) return
			event.preventDefault()
			space.setFullscreen(false)
		}
		// Ctrl+Shift+F: every tweak, searched, wherever focus is; again, put away.
		const search = (event: KeyboardEvent) => {
			if (!(event.ctrlKey || event.metaKey) || !event.shiftKey || event.altKey)
				return
			if (event.key.toLowerCase() !== 'f') return
			event.preventDefault()
			event.stopPropagation()
			toggleSearch()
		}
		window.addEventListener('keydown', keys, true)
		window.addEventListener('keydown', search, true)
		onCleanup(() => {
			window.removeEventListener('keydown', keys, true)
			window.removeEventListener('keydown', search, true)
		})
	})

	return (
		<section class='tweaks' classList={{ desk: props.drafts }}>
			<Show when={props.drafts}>
				<DocList
					items={space.items()}
					active={space.ws.active}
					filter={space.ws.filter}
					onSelect={space.open}
					onFilter={space.setFilter}
				/>
			</Show>

			<div class='tweak-main'>
				<Toolbar
					doc={doc()}
					prepared={space.prepared()}
					busy={busy()}
					fullscreen={space.ws.fullscreen}
					comparing={space.ws.compare !== null}
					searching={space.ws.search.open}
					heading={props.drafts}
					drafts={drafts()}
					onClose={props.onClose}
					onFormat={() => void act('format', () => space.format(doc().id))}
					onReset={() => space.reset(doc().id)}
					onSave={(name) => void save(name)}
					onLoad={space.loadDraft}
					onDelete={(name) => void remove(name)}
					onFullscreen={space.setFullscreen}
					onCompare={toggleCompare}
					onCopy={(what) => void copy(what)}
					onSearch={toggleSearch}
					onPalette={palette}
					onDone={() => (props.drafts ? props.onClose?.() : space.expand(null))}
					closeTitle={
						props.drafts
							? 'Back to the settings'
							: 'Fold the editor away; what you typed is kept'
					}
				/>

				{/* The search sits left of the line numbers, as an editor's own does. */}
				<div class='tweak-body'>
					<Show when={space.ws.search.open}>
						<SearchPanel
							query={space.ws.search.query}
							found={found()}
							active={space.ws.active}
							ref={(input) => (searchInput = input)}
							onQuery={(query) => space.setSearch({ query })}
							onPick={pick}
							onClose={closeSearch}
						/>
					</Show>
					<Show
						when={space.ws.compare}
						fallback={
							<EditorHost
								doc={doc()}
								problems={space.check()?.problems ?? []}
								warnings={warnings()}
								assist={space.assist()}
								goto={space.goto()}
								onArrived={() => space.setGoto(null)}
								onEditor={(next) => (editor = next)}
								minimap={space.ws.fullscreen}
								outline={space.check()?.outline ?? []}
								format={(text) => space.formatText(text, doc().kind)}
								onEdit={space.edit}
								onSave={() => void save(draftNameFor(doc()))}
								onSend={sendNow}
							/>
						}
					>
						{(compare) => (
							<ComparePane
								compare={compare()}
								options={sideOptions(space.ws, history(), proposal(), now())}
								resolve={resolve}
								diff={space.diffText}
								onChange={space.setCompare}
								onClose={() => space.setCompare(null)}
								onEditor={(next) => (editor = next)}
							/>
						)}
					</Show>
				</div>

				<Problems
					problems={space.check()?.problems ?? []}
					warnings={warnings()}
					notes={doc().notes}
					onGoto={jump}
				/>

				<Outline symbols={space.check()?.outline ?? []} onGoto={jump} />

				<Show when={proposal()}>
					{(value) => (
						<div class='tweak-extra'>
							<VoteDiff
								kind={doc().kind}
								current={doc().blob ?? ''}
								proposed={value()}
								title='A vote proposes this slot'
							/>
							<button
								class='link'
								onClick={() =>
									space.setCompare({
										left: { vote: true },
										right: { doc: doc().id, text: 'buffer' },
									})
								}
							>
								Compare it with your edit
							</button>
						</div>
					)}
				</Show>

				<Show when={changes().length > 0}>
					<details class='tweak-extra history'>
						<summary>Changes this session · {changes().length}</summary>
						<For each={changes()}>
							{(change) => (
								<div class='history-row'>
									<span>
										{when(change.at, now())} by {change.by ?? 'someone'} ·{' '}
										{change.from.length} → {change.to.length} chars · #
										{change.seq}
									</span>
									<button
										class='link'
										onClick={() =>
											space.setCompare(compareChange(history(), change.seq))
										}
									>
										Compare
									</button>
								</div>
							)}
						</For>
					</details>
				</Show>

				<SendBar
					doc={doc()}
					prepared={space.prepared()}
					problem={space.problem()}
					busy={busy()}
					refusal={refusal()}
					spads={room.caps.spads}
					target={space.ws.target}
					minify={space.ws.minify}
					onMinify={space.setMinify}
					onTarget={space.setTarget}
					onSend={(direct) => void send(direct)}
					onClear={() => void act('clear', () => space.clear())}
					onCopy={(what) => void copy(what)}
				/>
			</div>
		</section>
	)
}
