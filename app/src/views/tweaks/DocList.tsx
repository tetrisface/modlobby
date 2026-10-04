import { Select } from '../../components/Select'
import { For, Show } from 'solid-js'
import { shortcut } from '../../lib/platform'
import {
	KINDS,
	SCRATCH,
	isSlotId,
	type DocId,
	type Filter,
	type Item,
	type Sort,
} from '../../lib/tweakspace'

const SORTS: { key: Sort; label: string }[] = [
	{ key: 'name', label: 'Name' },
	{ key: 'kind', label: 'Kind' },
]

/**
 * The drafts editor's list: the room's slots on top, as the room holds them,
 * then the unslotted tweak and the drafts on disk. One search finds in both;
 * the sort is the drafts'. Each says what it is called, how long it is and
 * whether it holds something its slot or file does not.
 */
export function DocList(props: {
	slots: Item[]
	items: Item[]
	active: DocId
	filter: Filter
	onSelect: (id: DocId) => void
	onFilter: (patch: Partial<Filter>) => void
}) {
	const searching = () => props.filter.query.trim() !== ''
	const row = (item: Item) => (
		<DocRow
			item={item}
			on={item.id === props.active}
			onSelect={() => props.onSelect(item.id)}
		/>
	)
	return (
		<aside class='doc-list'>
			<div class='doc-find'>
				<input
					placeholder='Find a slot or draft'
					aria-label='Find'
					value={props.filter.query}
					onInput={(event) =>
						props.onFilter({ query: event.currentTarget.value })
					}
				/>
				<Select
					aria-label='Sort'
					title='How the drafts are sorted; the slots keep the order BAR runs them in'
					value={props.filter.sort}
					onChange={(event) =>
						props.onFilter({ sort: event.currentTarget.value as Sort })
					}
				>
					<For each={SORTS}>
						{(sort) => <option value={sort.key}>{sort.label}</option>}
					</For>
				</Select>
			</div>

			<div class='doc-rows'>
				<p class='doc-head'>In this room</p>
				<For
					each={props.slots}
					fallback={
						<p class='muted doc-none'>
							{searching() ? 'No slot matches' : 'Nothing set yet'}
						</p>
					}
				>
					{row}
				</For>

				<p class='doc-head'>Drafts</p>
				<For each={props.items}>{row}</For>
				<Show when={props.items.length === 1 && !searching()}>
					<p class='muted setup-empty'>
						No drafts yet. Keep one with Save, or {shortcut('S')}.
					</p>
				</Show>
			</div>
		</aside>
	)
}

/** What a document holds that its slot or file does not, by what it is. */
function editedTag(item: Item): string {
	if (item.id === SCRATCH) return 'unsaved'
	return isSlotId(item.id) ? 'unsent' : 'edited'
}

function DocRow(props: { item: Item; on: boolean; onSelect: () => void }) {
	return (
		<button
			class='doc'
			classList={{
				on: props.on,
				dirty: props.item.dirty,
				scratch: props.item.id === SCRATCH,
				empty: props.item.empty,
			}}
			title={props.item.name ?? props.item.title}
			onClick={props.onSelect}
		>
			<span class='doc-line'>
				<span class='doc-kind'>{props.item.kind}</span>
				<span class='doc-title'>{props.item.title}</span>
				<span class='doc-size'>
					{props.item.empty
						? '—'
						: `${props.item.size} ${KINDS[props.item.kind].text.toLowerCase()}`}
				</span>
			</span>
			<span class='doc-line'>
				<span class='doc-name'>{props.item.name ?? ''}</span>
				<Show when={props.item.dirty}>
					<span class='doc-tag dirty'>{editedTag(props.item)}</span>
				</Show>
			</span>
		</button>
	)
}
