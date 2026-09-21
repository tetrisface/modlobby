import { For, Show, onMount } from 'solid-js'
import type { DocId, Found, Hit } from '../../lib/tweakspace'

/** How much of a line is kept ahead of a match, so the match stays in view. */
const LEAD = 24

/**
 * Every tweak in the room, searched at once: beside the editor, left of its
 * line numbers, the way an editor's own search sits. Each match is its line
 * with the match marked; pressing one opens its tweak there.
 */
export function SearchPanel(props: {
	query: string
	found: Found[]
	/** The tweak open now, whose matches are marked as its own. */
	active: DocId
	/** The input, so Ctrl+Shift+F can come back to it. */
	ref?: (input: HTMLInputElement) => void
	onQuery: (query: string) => void
	onPick: (id: DocId, hit: Hit) => void
	onClose: () => void
}) {
	let input: HTMLInputElement | undefined
	onMount(() => input?.select())

	const total = () =>
		props.found.reduce((sum, found) => sum + found.hits.length + found.more, 0)
	const first = () => {
		const found = props.found[0]
		const hit = found?.hits[0]
		if (found && hit) props.onPick(found.id, hit)
	}

	return (
		<aside class='tweak-search' role='search'>
			<div class='tweak-search-head'>
				<input
					ref={(element) => {
						input = element
						props.ref?.(element)
					}}
					placeholder='Search all tweaks'
					aria-label='Search all tweaks'
					spellcheck={false}
					autocomplete='off'
					value={props.query}
					onInput={(event) => props.onQuery(event.currentTarget.value)}
					onKeyDown={(event) => {
						if (event.key === 'Enter') first()
						if (event.key !== 'Escape') return
						event.preventDefault()
						props.onClose()
					}}
				/>
				<button
					class='tweak-search-close'
					title='Close the search (Esc)'
					aria-label='Close the search'
					onClick={props.onClose}
				>
					×
				</button>
			</div>
			<Show when={props.query !== ''}>
				<p class='tweak-search-count'>
					{total() === 0
						? 'Nothing in any tweak'
						: `${total()} in ${props.found.length} ${props.found.length === 1 ? 'tweak' : 'tweaks'}`}
				</p>
			</Show>
			<div class='tweak-search-list'>
				<For each={props.found}>
					{(found) => (
						<section
							class='tweak-search-doc'
							classList={{ on: found.id === props.active }}
						>
							<header title={found.name ?? found.title}>
								<span class='tweak-search-key'>{found.title}</span>
								<span class='tweak-search-name'>{found.name ?? ''}</span>
								<span class='tweak-search-c'>
									{found.hits.length + found.more}
								</span>
							</header>
							<For each={found.hits}>
								{(hit) => (
									<button
										class='tweak-search-hit'
										title={`Line ${hit.line}`}
										onClick={() => props.onPick(found.id, hit)}
									>
										<span class='tweak-search-line'>{hit.line}</span>
										<Snippet hit={hit} />
									</button>
								)}
							</For>
							<Show when={found.more > 0}>
								<p class='tweak-search-more'>and {found.more} more</p>
							</Show>
						</section>
					)}
				</For>
			</div>
		</aside>
	)
}

/** A match's line, trimmed ahead of it so the match is what shows. */
function Snippet(props: { hit: Hit }) {
	const start = () => props.hit.column - 1
	const before = () => {
		const text = props.hit.text.slice(0, start()).trimStart()
		return text.length > LEAD ? `…${text.slice(-LEAD)}` : text
	}
	return (
		<span class='tweak-search-text'>
			{before()}
			<mark>{props.hit.text.slice(start(), start() + props.hit.length)}</mark>
			{props.hit.text.slice(start() + props.hit.length)}
		</span>
	)
}
