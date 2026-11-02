import { For, Show, createSignal, onCleanup } from 'solid-js'
import { Marked } from '../../components/Marked'
import { Segmented } from '../../components/Segmented'
import type { OptionChangeView } from '../../ipc/bindings/OptionChangeView'
import type { VoteView } from '../../ipc/bindings/VoteView'
import { when } from '../../lib/presets'
import {
	choices,
	defaultText,
	displayText,
	facts,
	isCleared,
	label,
	litChoice,
	notInForce,
	voteOn,
	wordsFor,
	type Lock,
	type OptionLink,
	type Row,
} from '../../lib/setup'
import { KINDS, isDocumentKey, kindOf, slotId } from '../../lib/tweakspace'
import { createCountdown, tally } from '../../lib/vote'
import { tweakspaceFor } from '../../store/tweakspaceInstance'
import { useRoom } from '../room/model'
import { LinkButton, useCommit } from './Rows'

/**
 * The foot of the list: everything known about one row, in a place that does
 * not move. What it does, what it can be and what it starts as, who set it and
 * from what, and any vote on it, none of which a row has room to say.
 *
 * Its height is fixed, so a row picked never shifts the list above it. The row
 * it describes is the list's to say; see `lib/inspecting`.
 */
export function Inspector(props: {
	row: Row | null
	/** What it says until a row has been picked. */
	fallback: string
	/** Why this reader may not set anything, if so. */
	refusal: string | null
	change: OptionChangeView | undefined
	vote: VoteView | null
	lock: Lock | null
	links: OptionLink[]
}) {
	const space = tweakspaceFor(useRoom())
	/** The name a document's tweak goes by, which only the workspace knows. */
	const nameOf = (key: string) => space.ws.docs[slotId(key)]?.name ?? ''
	return (
		<section class='inspector' aria-label='About this setting'>
			<Show
				when={props.row}
				fallback={<p class='inspector-empty'>{props.fallback}</p>}
			>
				{(row) => (
					<>
						<div class='inspector-head'>
							<span class='inspector-name'>
								<Marked text={label(row().option)} />
							</span>
							<code class='inspector-key'>{row().option.key}</code>
							<Show when={row().option.hidden}>
								<span class='doc-tag opt-hidden'>hidden</span>
							</Show>
							<Show
								when={
									choices(row().option).length === 0 &&
									!isDocumentKey(row().option.key)
								}
							>
								<span
									class='inspector-value'
									classList={{ changed: row().changed }}
								>
									{displayText(row())}
								</span>
							</Show>
						</div>
						<Show when={row().option.desc}>
							{(desc) => (
								<p class='inspector-desc'>
									<Marked text={desc()} />
								</p>
							)}
						</Show>
						<Pills row={row()} refusal={props.refusal} />
						<p class='inspector-facts'>
							{factsOf(row(), nameOf(row().option.key)).join(' · ')}
						</p>
						<For each={props.links}>
							{(link) => (
								<p class='inspector-link'>
									<LinkButton link={link} />
									{link.host === '' ? link.desc : `${link.desc} · ${link.host}`}
								</p>
							)}
						</For>
						<Show when={props.lock}>
							{(lock) => <p class='inspector-off'>{notInForce(lock())}</p>}
						</Show>
						<Show when={props.change}>
							{(change) => <Trace row={row()} change={change()} />}
						</Show>
						<Show when={voteOn(props.vote, row().option.key)}>
							{(proposal) => (
								<VoteLine
									row={row()}
									vote={props.vote!}
									value={proposal().value}
								/>
							)}
						</Show>
					</>
				)}
			</Show>
		</section>
	)
}

/** What the table tells about a row; for a document, how long it is rather than what it holds. */
function factsOf(row: Row, name: string): string[] {
	const { key } = row.option
	if (!isDocumentKey(key)) return facts(row.option)
	const blob = row.current ?? ''
	if (isCleared(blob)) return ['empty']
	const size = `${blob.length} characters of ${KINDS[kindOf(key)].blob}`
	return name === '' ? [size] : [name, size]
}

/** What it can be, the value lit. Greyed for somebody the host would refuse, still readable. */
function Pills(props: { row: Row; refusal: string | null }) {
	const commit = useCommit()
	const option = () => props.row.option
	const lit = () =>
		litChoice(option(), props.row.current ?? defaultText(option()))
	const segments = () =>
		choices(option()).map((choice) => ({
			value: choice.value,
			label: choice.label,
			title:
				[choice.desc, props.refusal].filter(Boolean).join('\n') || undefined,
		}))
	const chosen = () =>
		choices(option()).find((choice) => choice.value === lit())?.desc ?? ''
	return (
		<Show when={segments().length > 0}>
			<Segmented
				label={label(option())}
				value={lit()}
				options={segments()}
				disabled={props.refusal !== null}
				onChange={(next) => void commit(props.row, next)}
			/>
			<Show when={chosen() !== ''}>
				<p class='inspector-choice'>{chosen()}</p>
			</Show>
		</Show>
	)
}

/** Who set it, when and from what. A document's former self is a blob, so it goes unsaid. */
function Trace(props: { row: Row; change: OptionChangeView }) {
	// A clock, so "3m ago" does not sit there being wrong.
	const [now, setNow] = createSignal(Date.now())
	const tick = setInterval(() => setNow(Date.now()), 30_000)
	onCleanup(() => clearInterval(tick))

	const option = () => props.row.option
	/** A value not set before was on its default, which is what the row showed. */
	const was = () =>
		isDocumentKey(option().key)
			? ''
			: wordsFor(option(), props.change.from || defaultText(option()))
	const by = () => (props.change.by === null ? '' : ` by ${props.change.by}`)
	return (
		<p class='inspector-trace'>
			Set{by()} {when(props.change.at, now())}
			{was() === '' ? '' : `, was ${was()}`}
		</p>
	)
}

function VoteLine(props: { row: Row; vote: VoteView; value: string }) {
	const left = createCountdown(() => props.vote.remainingSecs)
	const what = () =>
		isDocumentKey(props.row.option.key)
			? 'change this'
			: `set ${wordsFor(props.row.option, props.value)}`
	return (
		<p class='inspector-vote'>
			{props.vote.by ?? 'Somebody'} called a vote to {what()}: yes{' '}
			{tally(props.vote.yes, props.vote.yesNeeded)}, no{' '}
			{tally(props.vote.no, props.vote.noNeeded)}
			{left() > 0 ? `, ${left()}s left` : ''}
		</p>
	)
}
