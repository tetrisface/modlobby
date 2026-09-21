import { Select } from '../../components/Select'
import { For, Show } from 'solid-js'
import { shortcut } from '../../lib/platform'
import {
	SCRATCH,
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
 * The drafts editor's list: the unslotted tweak on top, then the drafts on
 * disk, searched and sorted, each saying what it is called, how long it is
 * and whether it holds something its file does not.
 */
export function DocList(props: {
	items: Item[]
	active: DocId
	filter: Filter
	onSelect: (id: DocId) => void
	onFilter: (patch: Partial<Filter>) => void
}) {
	return (
		<aside class='doc-list'>
			<div class='doc-find'>
				<input
					placeholder='Find a draft'
					aria-label='Find'
					value={props.filter.query}
					onInput={(event) =>
						props.onFilter({ query: event.currentTarget.value })
					}
				/>
				<Select
					aria-label='Sort'
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
				<For each={props.items}>
					{(item) => (
						<button
							class='doc'
							classList={{
								on: item.id === props.active,
								dirty: item.dirty,
								scratch: item.id === SCRATCH,
								empty: item.empty,
							}}
							title={item.name ?? item.title}
							onClick={() => props.onSelect(item.id)}
						>
							<span class='doc-line'>
								<span class='doc-kind'>{item.kind}</span>
								<span class='doc-title'>{item.title}</span>
								<span class='doc-size'>
									{item.empty ? '—' : `${item.size} lua`}
								</span>
							</span>
							<span class='doc-line'>
								<span class='doc-name'>{item.name ?? ''}</span>
								<Show when={item.dirty}>
									<span class='doc-tag dirty'>
										{item.id === SCRATCH ? 'unsaved' : 'edited'}
									</span>
								</Show>
							</span>
						</button>
					)}
				</For>
				<Show when={props.items.length === 1 && props.filter.query === ''}>
					<p class='muted setup-empty'>
						No drafts yet. Keep one with Save, or {shortcut('S')}.
					</p>
				</Show>
			</div>
		</aside>
	)
}
