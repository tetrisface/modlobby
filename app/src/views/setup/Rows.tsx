import { For, Match, Show, Switch, createMemo } from 'solid-js'
import { ActionCell, CellButton } from '../../components/ActionCell'
import { openExternal } from '../../components/Linkify'
import { OptionName } from '../../components/OptionName'
import { Select } from '../../components/Select'
import { describeError } from '../../ipc/client'
import {
	NO_NOTE,
	defaultText,
	defaultWords,
	displayText,
	isOn,
	notInForce,
	resetValue,
	tipFacts,
	type Row,
	type RowNote,
} from '../../lib/setup'
import { isDocumentKey } from '../../lib/tweakspace'
import { pushNotice } from '../../store/chat'
import { useRoom } from '../room/model'
import { setRefusal } from '../room/move'
import { TweakRow } from '../tweaks/TweakRow'

type Put = (key: string, value: string) => Promise<void>

/**
 * Where every change to a setting goes, whichever control made it: never when
 * it would change nothing, since each send is a chat command the whole lobby
 * sees. `set` is where a value goes if not to the lobby's own settings.
 */
export function useCommit(set?: () => Put | undefined) {
	const room = useRoom()
	return async (row: Row, next: string) => {
		if (next === (row.current ?? defaultText(row.option))) return
		try {
			const put = set?.() ?? room.io.setOption
			await put(row.option.key, next)
		} catch (error) {
			pushNotice('warning', `${row.option.key}: ${describeError(error)}`)
		}
	}
}

/**
 * Settings as rows. A base64url document -- a tweak, the start-box override,
 * or one of the map's own tables -- is a `TweakRow` wherever it appears: its
 * blob to read or paste over, and the editor one press away.
 *
 * An AI's own options are the same kind of table -- BAR's `modoptions.lua` and
 * an engine AI's `AIOptions.lua` are one format -- and drawing them the same
 * way is the whole reason they can be. `set` is where a changed value goes;
 * the lobby's own settings are the default. `note` is what the pane knows
 * about a row beyond its value.
 *
 * Kept by option key, never by place or by identity. The rows are made again
 * whenever the lobby says anything, so one kept by identity would be a new row
 * each time -- under an open tweak, a new editor. And sorting moves them, so
 * one kept by place would hand its input to whichever setting lands there,
 * with what was being typed sent as that setting's value.
 */
export function Rows(props: {
	rows: Row[]
	editable: boolean
	set?: Put
	note?: (row: Row) => RowNote
}) {
	const byKey = createMemo(
		() => new Map(props.rows.map((row) => [row.option.key, row])),
	)
	const noteOf = (row: Row) => props.note?.(row) ?? NO_NOTE
	return (
		<div class='setup-rows'>
			<Show
				when={props.rows.length > 0}
				fallback={<p class='muted setup-empty'>Nothing here.</p>}
			>
				<For each={props.rows.map((row) => row.option.key)}>
					{(key) => (
						<Show when={byKey().get(key)}>
							{(row) => (
								<Switch
									fallback={
										<PlainRow
											row={row()}
											editable={props.editable}
											set={props.set}
											note={noteOf(row())}
										/>
									}
								>
									<Match when={isDocumentKey(key)}>
										<TweakRow row={row()} note={noteOf(row())} />
									</Match>
									<Match when={row().option.type === 'link'}>
										<LinkRow row={row()} note={noteOf(row())} />
									</Match>
								</Switch>
							)}
						</Show>
					)}
				</For>
			</Show>
		</div>
	)
}

/** A setting: its name, its value or a control for it, and the way back to its default. */
function PlainRow(props: {
	row: Row
	editable: boolean
	set?: Put
	note: RowNote
}) {
	const room = useRoom()
	const commit = useCommit(() => props.set)
	const option = () => props.row.option
	/** Why it cannot go back, or what going back would set. */
	const resetTitle = () => {
		if (!props.row.changed) return 'Already on its default'
		if (!props.editable) return setRefusal(room) ?? 'Not yours to change'
		return `Back to its default, ${defaultWords(option())}`
	}
	return (
		<div
			class='opt'
			classList={{
				changed: props.row.changed,
				recent: props.note.recent,
				off: props.note.lock !== null,
				on: props.note.on,
			}}
			data-key={option().key}
		>
			<span class='mark' />
			<OptionName option={option()}>
				<Show when={option().hidden}>
					<span
						class='doc-tag opt-hidden'
						title="BAR keeps this out of its own lobby's settings"
					>
						hidden
					</span>
				</Show>
				<Show when={props.note.lock}>
					{(lock) => (
						<span class='doc-tag opt-off' title={notInForce(lock())}>
							off
						</span>
					)}
				</Show>
			</OptionName>
			<span class='opt-value'>
				<Switch fallback={<span class='v'>{displayText(props.row)}</span>}>
					<Match when={props.editable}>
						<Control row={props.row} set={props.set} />
					</Match>
				</Switch>
				<Show when={props.row.changed && defaultWords(option()) !== ''}>
					<span class='opt-default'>default {defaultWords(option())}</span>
				</Show>
				<Show when={props.note.vote !== null}>
					<span
						class='doc-tag opt-vote'
						title='A vote under way would set this'
					>
						vote {props.note.vote}
					</span>
				</Show>
			</span>
			<ActionCell filled={props.row.changed && props.editable}>
				<CellButton
					icon='act-undo'
					title={resetTitle()}
					disabled={!props.row.changed || !props.editable}
					onClick={() =>
						void commit(
							props.row,
							resetValue(option(), !props.set && room.caps.spads),
						)
					}
				/>
			</ActionCell>
		</div>
	)
}

/** One of BAR's links: somewhere to go, with no value to set. */
function LinkRow(props: { row: Row; note: RowNote }) {
	const url = () => props.row.option.link ?? ''
	return (
		<div
			class='opt link'
			classList={{ on: props.note.on }}
			data-key={props.row.option.key}
		>
			<span class='mark' />
			<OptionName option={props.row.option} />
			<span class='opt-value'>
				<span class='v' title={url()}>
					{url()}
				</span>
			</span>
			<ActionCell filled={url() !== ''}>
				<CellButton
					icon='act-external'
					title={`Open ${url()}`}
					disabled={url() === ''}
					onClick={() => void openExternal(url())}
				/>
			</ActionCell>
		</div>
	)
}

/**
 * Whether the webview draws a list's choices itself, markup and all. A native
 * list would run a choice's name and description together into one line.
 */
const richLists = () =>
	globalThis.CSS?.supports?.('appearance', 'base-select') === true

/**
 * What a list shows while shut: by default a copy of the whole chosen choice,
 * description and all, which this button lets the stylesheet leave out. Made
 * by hand because the JSX types do not know `selectedcontent` yet.
 */
function closedBox(): HTMLButtonElement | null {
	if (!richLists()) return null
	const button = document.createElement('button')
	button.type = 'button'
	button.append(document.createElement('selectedcontent'))
	return button
}

/**
 * One editable setting. The value is sent when it is committed, never on
 * every keystroke: each send is a chat command the whole lobby sees.
 */
function Control(props: { row: Row; set?: Put }) {
	const room = useRoom()
	const commit = useCommit(() => props.set)
	const option = () => props.row.option
	const value = () => props.row.current ?? defaultText(option())
	const set = (next: string) => void commit(props.row, next)

	return (
		<Switch fallback={<span class='v'>{displayText(props.row)}</span>}>
			<Match when={option().type === 'bool'}>
				<input
					class='v-edit'
					type='checkbox'
					checked={isOn(value())}
					onChange={(event) => set(event.currentTarget.checked ? '1' : '0')}
				/>
			</Match>
			<Match when={option().type === 'number'}>
				{/* Emptied, it reads as its default, and that is what it goes back to. */}
				<input
					class='v-edit'
					type='number'
					value={value()}
					placeholder={defaultText(option())}
					title={tipFacts(option())}
					min={option().min ?? undefined}
					max={option().max ?? undefined}
					step={option().step ?? undefined}
					onChange={(event) =>
						set(
							event.currentTarget.value ||
								resetValue(option(), !props.set && room.caps.spads),
						)
					}
				/>
			</Match>
			<Match when={option().type === 'list'}>
				<Select
					class='v-edit'
					value={value()}
					onChange={(event) => set(event.currentTarget.value)}
				>
					{closedBox()}
					<For each={option().items ?? []}>
						{(item) => (
							<option value={item.key}>
								{item.name}
								{richLists() && item.desc ? (
									<small class='select-note'>{item.desc}</small>
								) : null}
							</option>
						)}
					</For>
				</Select>
			</Match>
			<Match when={option().type === 'string'}>
				<input
					class='v-edit'
					type='text'
					spellcheck={false}
					placeholder='empty'
					value={value()}
					onChange={(event) => set(event.currentTarget.value.trim())}
				/>
			</Match>
		</Switch>
	)
}
