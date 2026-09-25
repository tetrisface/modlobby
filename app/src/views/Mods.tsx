import { For, Show, createEffect, createMemo, createSignal } from 'solid-js'
import { ActionCell, CellButton } from '../components/ActionCell'
import { openExternal } from '../components/Linkify'
import { Glyph } from '../components/icons'
import { api, describeError } from '../ipc/client'
import { age, exactly } from '../lib/age'
import { reorderGesture } from '../lib/drag'
import {
	type Change,
	type ModSet,
	type Pick,
	type SourceLink,
	addPick,
	adopt,
	changeOf,
	command,
	editPick,
	movePick,
	offerOf,
	picksOf,
	removePick,
	sameList,
	sourceLinks,
	summary,
	updatedTo,
} from '../lib/mods'
import { offers } from '../lib/mutators'
import { pushNotice } from '../store/chat'
import { draftFor, forgetSet, modSets, setDraft } from '../store/mods'
import { useRoom } from './room/model'
import { bossing, modRefusal } from './room/move'

/**
 * What the room loads on top of its game: the pane's third face, in a room
 * whose host runs mods.
 *
 * The list is edited as a draft -- dragged into order, added to from what the
 * host offers or from a pasted GitHub page, trimmed, moved to a newer commit
 * -- and goes to the host as one command, so one vote covers the lot. A boss
 * is taken at their word; a seated player puts it to the room; anyone else
 * can still draft, and reads why it will not go.
 *
 * Nothing here is local state the room does not know: the draft is only ever
 * a difference from what the host announces, and clears itself once the host
 * announces it.
 */
export function Mods() {
	const room = useRoom()
	const offered = createMemo(() => offers(room.my()?.scriptTags))
	const current = createMemo(() => picksOf(room.check().mutators, offered()))
	const roomId = () => room.my()?.id ?? 0
	const draft = () => draftFor(roomId())
	const shown = () => draft() ?? current()
	const drafting = () => draft() !== null
	const edit = (next: Pick[]) => setDraft(roomId(), next)

	// The host announced what the draft asked for: it is the room's now.
	createEffect(() => {
		const held = draft()
		if (held && sameList(held, current())) setDraft(roomId(), null)
	})

	const refusal = () => modRefusal(room)
	const [problem, setProblem] = createSignal<string | null>(null)

	function add(text: string): boolean {
		const next = addPick(shown(), text, offered())
		if ('problem' in next) {
			setProblem(next.problem)
			return false
		}
		setProblem(null)
		edit(next.picks)
		return true
	}

	function apply() {
		const held = draft()
		if (!held) return
		void room.io
			.sayBattle(command(held))
			.catch((error) => pushNotice('warning', describeError(error)))
	}

	/**
	 * Asks GitHub for the newest commit of each of these mods and moves only
	 * the ones it would move; the rest stay as the room has them, and are
	 * said to be current. Whatever else changed in the draft meanwhile stays.
	 */
	const [checking, setChecking] = createSignal(false)
	async function update(picks: readonly Pick[]) {
		setChecking(true)
		const moved = new Map<string, Pick>()
		const already: string[] = []
		for (const pick of picks) {
			if (!pick.repo) continue
			try {
				// ponytail: the default branch; a host entry that tracks another branch is checked against the wrong one
				const newest = await api.newestCommit(pick.repo, null)
				const next = updatedTo(pick, newest, current())
				moved.set(pick.repo, next.pick)
				if (next.already) already.push(pick.label)
			} catch (error) {
				pushNotice('warning', `${pick.label}: ${describeError(error)}`)
			}
		}
		setChecking(false)
		const next = shown().map(
			(pick) => (pick.repo && moved.get(pick.repo)) || pick,
		)
		if (drafting() || !sameList(next, current())) edit(next)
		if (already.length > 0)
			pushNotice('info', `Already at the newest commit: ${already.join(', ')}`)
	}

	/**
	 * A row in flight and where it would land. The list is drawn in that order
	 * while it is -- the others making room -- and the draft changes only on
	 * the drop.
	 */
	const [flying, setFlying] = createSignal<{ from: number; to: number } | null>(
		null,
	)
	const listed = () => {
		const held = flying()
		return held ? movePick(shown(), held.from, held.to) : shown()
	}

	/**
	 * The rows opened to what they say they do, by repository or name, so an
	 * update from the host -- which remakes every row -- leaves them open.
	 */
	const [opened, setOpened] = createSignal<ReadonlySet<string>>(new Set())
	const keyOf = (pick: Pick) => pick.repo ?? pick.ref
	function toggle(pick: Pick) {
		const next = new Set(opened())
		if (!next.delete(keyOf(pick))) next.add(keyOf(pick))
		setOpened(next)
	}

	return (
		<div class='mods'>
			<div class='toolbar'>
				<span class='note'>
					{refusal() ?? 'Loaded in this order, each on top of the last'}
				</span>
				<span class='spacer' />
				<Show when={shown().some((pick) => pick.repo)}>
					<button
						class='chip-choice'
						title='Move every mod from GitHub to the newest commit of the branch it follows'
						disabled={checking()}
						onClick={() => void update(shown())}
					>
						Update all
					</button>
				</Show>
			</div>

			<div class='mods-body'>
				<div class='setup-section'>
					<span>Loaded</span>
					<span class='count'>{shown().length}</span>
				</div>
				<div class='mod-rows loaded'>
					<For
						each={listed()}
						fallback={
							<p class='muted setup-empty'>Nothing on top of the game.</p>
						}
					>
						{(pick, index) => (
							<Row
								pick={pick}
								change={drafting() ? changeOf(pick, current()) : 'same'}
								open={opened().has(keyOf(pick))}
								onToggle={() => toggle(pick)}
								onPress={reorderGesture({
									from: index,
									onOver: setFlying,
									onDrop: (from, to) => edit(movePick(shown(), from, to)),
									onTap: () => pick.description && toggle(pick),
								})}
								checking={checking()}
								onUpdate={() => void update([pick])}
								onEdit={(text) => {
									const next = editPick(pick, text)
									if (next === null) return false
									edit(
										shown().map((held, at) => (at === index() ? next : held)),
									)
									return true
								}}
								onRemove={() => edit(removePick(shown(), index()))}
							/>
						)}
					</For>
				</div>

				<AddRow
					problem={problem()}
					onAdd={add}
					onTyping={() => setProblem(null)}
				/>

				<Show when={offered().length > 0}>
					<div
						class='setup-section'
						title='What games on this host played, newest first. Adding one loads it at the commit it was played at.'
					>
						<span>Used recently on this host</span>
					</div>
					<div class='mod-offers'>
						<For each={offered()}>
							{(offer) => {
								const held = () => offerOf(shown(), offer)
								return (
									<button
										class='chip-choice'
										classList={{ on: held() !== undefined }}
										disabled={held() !== undefined}
										title={
											held()
												? `${held()?.label} is in the list`
												: offer.source
													? `Add ${offer.name}, at the commit it was played at\n${offer.source.replace(/^github:/, '')}`
													: `Add ${offer.name}`
										}
										onClick={() => add(offer.name)}
									>
										{offer.name}
										<Show when={offer.date}>
											{(date) => <span class='mod-age'>{age(date())}</span>}
										</Show>
									</button>
								)
							}}
						</For>
					</div>
				</Show>

				<Show when={modSets().length > 0}>
					<div
						class='setup-section'
						title='The combinations of mods your games were played with, newest first, in the order and at the commits last played'
					>
						<span>Played recently</span>
						<span class='count'>{modSets().length}</span>
					</div>
					<div class='mod-rows sets'>
						<For each={modSets()}>
							{(set) => (
								<SetRow
									set={set}
									onUse={() => {
										setProblem(null)
										edit(adopt(set.mods, current(), offered()))
									}}
									onForget={() => forgetSet(set.at)}
								/>
							)}
						</For>
					</div>
				</Show>
			</div>

			<Show when={draft()}>
				{(held) => (
					<footer class='mod-footer'>
						<span class='mod-summary'>
							<For each={summary(held(), current())} fallback='no change'>
								{(part) => (
									<span class={`mod-change ${part.change}`}>{part.words}</span>
								)}
							</For>
						</span>
						<code class='mod-command' title='What is said to the host'>
							{command(held())}
						</code>
						<span class='spacer' />
						<button onClick={() => setDraft(roomId(), null)}>Discard</button>
						<button
							class='primary'
							disabled={refusal() !== null}
							title={refusal() ?? command(held())}
							onClick={apply}
						>
							{bossing(room) ? 'Apply' : 'Vote to apply'}
						</button>
					</footer>
				)}
			</Show>
		</div>
	)
}

/**
 * What a draft's marker says about a row, in its tooltip and, beside the same
 * colour, in the footer's summary.
 */
const CHANGE_WORDS: Record<Change, string | null> = {
	same: null,
	added: 'Added by this draft',
	moving: 'Goes to another commit when the draft is applied',
}

/**
 * A place on GitHub, opened in the browser. A button, so a press on it is a
 * click and not the start of a drag.
 */
function SourceButton(props: { link: SourceLink }) {
	return (
		<button
			type='button'
			class='mod-link'
			title={
				props.link.date
					? `${props.link.tip}\ncommitted ${exactly(props.link.date)}`
					: props.link.tip
			}
			onClick={() => void openExternal(props.link.url)}
		>
			{props.link.date
				? `${props.link.text} · ${age(props.link.date)}`
				: props.link.text}
			<Glyph id='act-external' />
		</button>
	)
}

/**
 * One mod: dragged into load order, with where it comes from under its name
 * and, from GitHub, the means to move it or point it elsewhere. The commit
 * itself stays in the tooltips.
 */
function Row(props: {
	pick: Pick
	change: Change
	/** Opened to what it says it does. */
	open: boolean
	onToggle: () => void
	/** A press anywhere but on a control picks the row up; a click opens it. */
	onPress: (event: PointerEvent) => void
	/** While GitHub is being asked, nothing else is asked of it. */
	checking: boolean
	onUpdate: () => void
	/** Whether the text named a repository; the row keeps asking until it does. */
	onEdit: (text: string) => boolean
	onRemove: () => void
}) {
	const [editing, setEditing] = createSignal(false)
	const [refused, setRefused] = createSignal(false)
	const tip = () =>
		[
			CHANGE_WORDS[props.change],
			props.pick.date && `committed ${exactly(props.pick.date)}`,
			props.pick.description
				? 'Click for what it does; drag to change the load order'
				: 'Drag to change the load order',
		]
			.filter(Boolean)
			.join('\n')

	function commit(text: string) {
		if (text.trim() === '' || text.trim() === props.pick.ref) {
			setEditing(false)
			return
		}
		if (props.onEdit(text)) {
			setEditing(false)
			setRefused(false)
		} else {
			setRefused(true)
		}
	}

	return (
		<div
			class='mod-row movable'
			classList={{ [props.change]: true, open: props.open }}
			title={tip()}
			onPointerDown={props.onPress}
		>
			<span class='mark' />
			<span class='mod-grip' aria-hidden='true'>
				⠿
			</span>
			<span class='mod-what'>
				<span class='mod-title'>
					<span class='mod-name'>{props.pick.label}</span>
					<Show when={props.pick.description}>
						<button
							type='button'
							class='mod-expand'
							aria-expanded={props.open}
							title={props.open ? 'Hide what it does' : 'Show what it does'}
							onClick={() => props.onToggle()}
						>
							<Glyph id='act-expand' />
						</button>
					</Show>
				</span>
				<Show
					when={editing()}
					fallback={
						<Show when={sourceLinks(props.pick)}>
							{(links) => (
								<span class='mod-source'>
									<SourceButton link={links().at} />
									<Show when={links().to}>
										{(to) => (
											<>
												<span class='mod-arrow'>→</span>
												<SourceButton link={to()} />
											</>
										)}
									</Show>
								</span>
							)}
						</Show>
					}
				>
					<input
						class='mod-source-edit'
						classList={{ refused: refused() }}
						value={props.pick.ref}
						placeholder='owner/repo, owner/repo@branch, or the page on GitHub'
						ref={(field) => queueMicrotask(() => field.select())}
						onKeyDown={(event) => {
							if (event.key === 'Enter') commit(event.currentTarget.value)
							if (event.key === 'Escape') setEditing(false)
						}}
						onBlur={(event) => commit(event.currentTarget.value)}
					/>
				</Show>
			</span>
			<span class='mod-age'>{props.pick.date ? age(props.pick.date) : ''}</span>
			<ActionCell>
				<Show when={props.pick.repo}>
					<CellButton
						icon='act-upgrade'
						title='Move to the newest commit of the branch it follows'
						label={`Update ${props.pick.label}`}
						disabled={props.checking || props.pick.ref === props.pick.repo}
						onClick={props.onUpdate}
					/>
					<CellButton
						icon='act-pen'
						title='Point it at another branch, commit or repository'
						label={`Edit where ${props.pick.label} comes from`}
						onClick={() => setEditing(true)}
					/>
				</Show>
				<CellButton
					class='danger'
					icon='act-trash'
					title='Remove'
					label={`Remove ${props.pick.label}`}
					onClick={props.onRemove}
				/>
			</ActionCell>
			<Show when={props.open && props.pick.description}>
				{(text) => <p class='mod-description'>{text()}</p>}
			</Show>
		</div>
	)
}

/** Where a name the host offers, or a GitHub page, is typed or pasted in. */
function AddRow(props: {
	problem: string | null
	onAdd: (text: string) => boolean
	onTyping: () => void
}) {
	const [text, setText] = createSignal('')
	function submit() {
		if (text().trim() === '') return
		if (props.onAdd(text())) setText('')
	}
	return (
		<div class='mod-add'>
			<input
				value={text()}
				placeholder='owner/repo, owner/repo@branch or @commit, or a GitHub link'
				title={
					'A GitHub repository at the newest commit of its default branch (owner/repo),\n' +
					'at a branch or commit (owner/repo@main, owner/repo@9108a17),\n' +
					'or a link to its page, a branch or a commit.\n' +
					'A name used recently on this host works too.'
				}
				onInput={(event) => {
					setText(event.currentTarget.value)
					props.onTyping()
				}}
				onKeyDown={(event) => {
					if (event.key === 'Enter') submit()
				}}
			/>
			<button disabled={text().trim() === ''} onClick={submit}>
				Add
			</button>
			<Show when={props.problem}>
				{(why) => <span class='mod-problem'>{why()}</span>}
			</Show>
		</div>
	)
}

/** A set a room loaded before, ready to become the draft. */
function SetRow(props: {
	set: ModSet
	onUse: () => void
	onForget: () => void
}) {
	const names = () => props.set.mods.map((mod) => mod.label).join(' + ')
	return (
		<div
			class='mod-row set'
			title={`${names()}\nloaded ${exactly(props.set.at)}`}
		>
			<span class='mark' />
			<span class='mod-what'>
				<span class='mod-name'>{names()}</span>
			</span>
			<span class='mod-age'>{age(props.set.at)}</span>
			<ActionCell>
				<CellButton
					icon='act-reconnect'
					title='Load this set again'
					label={`Load ${names()} again`}
					onClick={props.onUse}
				>
					Use
				</CellButton>
				<CellButton
					class='danger'
					icon='act-trash'
					title='Forget'
					label={`Forget ${names()}`}
					onClick={props.onForget}
				/>
			</ActionCell>
		</div>
	)
}
