import { Select } from '../../components/Select'
import { For, Show, createEffect, createSignal, type JSX } from 'solid-js'
import { dismiss } from '../../components/dismiss'
import { Glyph } from '../../components/icons'
import type { Prepared } from '../../ipc/bindings/Prepared'
import { TWEAK_SLOTS } from '../../lib/setup'
import {
	KINDS,
	SLOT_KEYS,
	draftNameFor,
	isDirty,
	kindOf,
	type Doc,
} from '../../lib/tweakspace'

export type Copyable = 'lua' | 'minified' | 'blob' | 'command'

/** A draft as the menu lists it: its file name, and the header it goes by. */
export type DraftEntry = { title: string; name: string | null }

/**
 * What can be done to the open document's text: shape it, keep it as a
 * draft, copy it out, or look at it another way.
 *
 * Everything here is a prop: the bar knows nothing about the store or Rust,
 * which is what lets a test press each button and see the right call.
 */
export function Toolbar(props: {
	doc: Doc
	prepared: Prepared | null
	busy: boolean
	fullscreen: boolean
	/** Whether the main area is the comparison rather than the editor. */
	comparing: boolean
	/** The drafts editor names what is open; under a settings row the row does. */
	heading: boolean
	drafts: DraftEntry[]
	/** Back to the settings, from the drafts editor. */
	onClose?: () => void
	onFormat: () => void
	onReset: () => void
	onSave: (name: string) => void
	onLoad: (name: string) => void
	onDelete: (name: string) => void
	onFullscreen: (on: boolean) => void
	onCompare: () => void
	onCopy: (what: Copyable) => void
}) {
	const [draftName, setDraftName] = createSignal('')
	const dirty = () => isDirty(props.doc)

	function save() {
		props.onSave(draftName().trim() || draftNameFor(props.doc))
		setDraftName('')
	}

	return (
		<header class='tweak-bar'>
			<Show when={props.heading}>
				<div class='tweak-bar-row'>
					<Show when={props.onClose}>
						{(close) => (
							<button
								class='tweak-tool tweak-back'
								title='Back to the settings'
								onClick={() => close()()}
							>
								‹ Settings
							</button>
						)}
					</Show>
					<span class='doc-kind'>{props.doc.kind}</span>
					<h1>{props.doc.title}</h1>
					<Show when={props.doc.name}>
						{(name) => <span class='tweak-name'>{name()}</span>}
					</Show>
					<Show when={dirty()}>
						<span class='doc-tag dirty'>edited</span>
					</Show>
				</div>
			</Show>

			<div class='tweak-bar-row'>
				<button
					class='tweak-tool'
					onClick={props.onFormat}
					disabled={props.busy}
				>
					Format
				</button>
				<button
					class='tweak-tool'
					onClick={props.onReset}
					disabled={props.busy || !dirty()}
					title='Back to what the room holds, or what the draft file says'
				>
					Reset
				</button>
				{/* Drafts are Lua files; an arrangement is kept as a preset instead. */}
				<Show when={props.doc.kind !== 'boxes'}>
					<Menu label='Drafts' title='Keep this as a draft, or bring one in'>
						<div class='menu-save'>
							<input
								class='draft-name'
								placeholder={draftNameFor(props.doc)}
								aria-label='Draft name'
								value={draftName()}
								onInput={(event) => setDraftName(event.currentTarget.value)}
								onKeyDown={(event) => event.key === 'Enter' && save()}
							/>
							<button class='tweak-tool' onClick={save} disabled={props.busy}>
								Save draft
							</button>
						</div>
						<Show
							when={props.drafts.length > 0}
							fallback={<p class='menu-note'>No {props.doc.kind} drafts yet</p>}
						>
							<p class='menu-note'>Load into {props.doc.title}</p>
							<For each={props.drafts}>
								{(draft) => (
									<div class='menu-row'>
										<button
											class='menu-item'
											title={`Replace what ${props.doc.title} holds here with ${draft.title}`}
											onClick={() => props.onLoad(draft.title)}
										>
											<span>{draft.title}</span>
											<span class='menu-sub'>{draft.name ?? ''}</span>
										</button>
										<button
											class='menu-drop'
											title={`Delete the draft ${draft.title}`}
											aria-label={`Delete the draft ${draft.title}`}
											onClick={() => props.onDelete(draft.title)}
										>
											<Glyph id='act-trash' />
										</button>
									</div>
								)}
							</For>
						</Show>
					</Menu>
				</Show>
				<Menu label='Copy' title='Copy what is typed here, in any of its forms'>
					<button class='menu-item' onClick={() => props.onCopy('lua')}>
						{KINDS[props.doc.kind].text}
					</button>
					<button
						class='menu-item'
						disabled={!props.prepared}
						onClick={() => props.onCopy('minified')}
					>
						minified
					</button>
					<button
						class='menu-item'
						disabled={!props.prepared}
						onClick={() => props.onCopy('blob')}
					>
						{KINDS[props.doc.kind].blob}
					</button>
					<button
						class='menu-item'
						disabled={!props.prepared}
						onClick={() => props.onCopy('command')}
					>
						!bSet command
					</button>
				</Menu>
				<span class='spacer' />
				<button
					class='tweak-tool'
					classList={{ on: props.comparing }}
					title='Any two of: a slot, a draft, your edit, a change, the vote'
					onClick={props.onCompare}
				>
					{props.comparing ? 'Editor' : 'Compare'}
				</button>
				<button
					class='tweak-tool'
					title={
						props.fullscreen ? 'Back into the pane (Esc)' : 'Fill the window'
					}
					onClick={() => props.onFullscreen(!props.fullscreen)}
				>
					{props.fullscreen ? 'Exit fullscreen' : 'Fullscreen'}
				</button>
			</div>
		</header>
	)
}

/**
 * Where the text goes, and whether it fits on the way. The one place that
 * sends, under the editor where a form keeps its submit: the bar above only
 * works on the text.
 *
 * Where the room would refuse it from us, the buttons stay where they are,
 * greyed, with the reason beside them -- they are how this works when it
 * does -- and the command can still be copied for somebody who can.
 */
export function SendBar(props: {
	doc: Doc
	prepared: Prepared | null
	/** Why the buffer could not be measured -- a syntax error, most days. */
	problem: string | null
	busy: boolean
	/** Why the room would refuse a set from us, if it would; see `setRefusal`. */
	refusal: string | null
	/** A host is asked: there is a vote to call and a chat line to fit. */
	spads: boolean
	/** The slot a draft or the scratch goes to. */
	target: string
	/** Sent minified rather than as written; see `Workspace.minify`. */
	minify: boolean
	onMinify: (on: boolean) => void
	onTarget: (key: string) => void
	onSend: (direct: boolean) => void
	onClear: () => void
	onCopy: (what: Copyable) => void
}) {
	/**
	 * Something to send, and room for it: a slot once it is edited, anything
	 * else as it stands. A room with no host to ask puts it in a start script,
	 * which has no cap.
	 */
	const sendable = () =>
		props.refusal === null &&
		!props.busy &&
		(props.doc.origin !== 'slot' || isDirty(props.doc)) &&
		props.prepared !== null &&
		(!props.spads || props.prepared.gauge.fits)
	const slots = () =>
		props.doc.origin === 'scratch'
			? TWEAK_SLOTS
			: SLOT_KEYS.filter((key) => kindOf(key) === props.doc.kind)
	/** Where it goes: a slot is sent to itself. */
	const slot = () =>
		props.doc.origin === 'slot' ? props.doc.title : props.target

	return (
		<footer class='tweak-foot'>
			<Show when={props.prepared}>
				{(ready) => {
					const gauge = () => ready().gauge
					const names = () => KINDS[props.doc.kind]
					/**
					 * The longest the base64url may be: what the server keeps of a
					 * chat line, less the `!bSet <slot> ` in front of it. Where
					 * nothing is said in a chat line, nothing is too long.
					 */
					const max = () => gauge().cap - (gauge().command - gauge().blob)
					const over = (length: number) => props.spads && length > max()
					return (
						<span class='gauge'>
							<span classList={{ over: over(gauge().raw) }}>
								{names().text.toLowerCase()} {gauge().raw}
							</span>
							<span class='gauge-dot'>·</span>
							{/* The override is always sent compact; a Lua tweak only when asked. */}
							<Show
								when={props.doc.kind !== 'boxes'}
								fallback={
									<span classList={{ over: over(gauge().minified) }}>
										minified {gauge().minified}
									</span>
								}
							>
								<label
									class='gauge-minify'
									classList={{ over: over(gauge().minified) }}
									title='Send it minified: comments and layout go, names stay. Off unless you need it to fit, so the room gets what you wrote.'
								>
									<input
										type='checkbox'
										checked={props.minify}
										onChange={(event) =>
											props.onMinify(event.currentTarget.checked)
										}
									/>
									minified {gauge().minified}
								</label>
							</Show>
							<span class='gauge-dot'>·</span>
							<span classList={{ over: over(gauge().blob) }}>
								{names().blob} {gauge().blob}
							</span>
							<Show when={props.spads}>
								<span class='gauge-dot'>·</span>
								<span
									title={`The server keeps ${gauge().cap} characters of a chat line, the !bSet in front included`}
								>
									max {max()}
								</span>
							</Show>
						</span>
					)
				}}
			</Show>
			<Show when={props.problem}>
				{(text) => (
					<span class='gauge-badge over' title={text()}>
						will not load
					</span>
				)}
			</Show>
			{/* One group, so the buttons stay on a line and the numbers give way. */}
			<span class='tweak-send'>
				<Show when={props.doc.origin !== 'slot'}>
					<label class='tweak-target'>
						<span class='muted'>to</span>
						<Select
							value={props.target}
							onChange={(event) => props.onTarget(event.currentTarget.value)}
						>
							<For each={slots()}>
								{(key) => <option value={key}>{key}</option>}
							</For>
						</Select>
					</label>
				</Show>
				<Show when={props.refusal}>
					{(why) => (
						<>
							<span class='muted tweak-why'>{why()}</span>
							<button
								disabled={!props.prepared}
								title='Copy the command, for somebody who can set it'
								onClick={() => props.onCopy('command')}
							>
								Copy !bSet
							</button>
						</>
					)}
				</Show>
				<button
					disabled={props.busy || props.refusal !== null}
					title={props.refusal ?? `Set ${slot()} to nothing`}
					onClick={props.onClear}
				>
					Clear slot
				</button>
				<Show when={props.spads}>
					<button
						disabled={!sendable()}
						title={props.refusal ?? `Ask the room to set ${slot()}`}
						onClick={() => props.onSend(false)}
					>
						Call a vote
					</button>
				</Show>
				<button
					class='primary'
					disabled={!sendable()}
					title={props.refusal ?? `Set ${slot()} now (Ctrl+Enter)`}
					onClick={() => props.onSend(true)}
				>
					{props.spads ? 'Send !bSet' : 'Set slot'}
				</button>
			</span>
		</footer>
	)
}

/**
 * A button that drops a short list. It closes on a press outside, on Escape,
 * and once one of its buttons has done its thing.
 */
function Menu(props: { label: string; title: string; children: JSX.Element }) {
	const [open, setOpen] = createSignal(false)
	let root: HTMLSpanElement | undefined
	createEffect(() => {
		if (open())
			dismiss(
				() => root,
				() => setOpen(false),
			)
	})

	return (
		<span class='tweak-menu' ref={root}>
			<button
				class='tweak-tool'
				title={props.title}
				aria-haspopup='menu'
				aria-expanded={open()}
				onClick={() => setOpen(!open())}
			>
				{props.label}
				<Glyph id='act-expand' />
			</button>
			<Show when={open()}>
				<div
					class='popover tweak-menu-list'
					role='menu'
					onClick={(event) => {
						if ((event.target as Element).closest('button')) setOpen(false)
					}}
				>
					{props.children}
				</div>
			</Show>
		</span>
	)
}
