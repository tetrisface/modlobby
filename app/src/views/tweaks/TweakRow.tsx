import { Show, createEffect, createResource, createSignal } from 'solid-js'
import { ActionCell, CellButton } from '../../components/ActionCell'
import { OptionName } from '../../components/OptionName'
import { api, describeError } from '../../ipc/client'
import { NO_NOTE, isCleared, type Row, type RowNote } from '../../lib/setup'
import {
	KINDS,
	MAP_TABLE_REFUSAL,
	isDirty,
	isMapTable,
	kindOf,
	slotId,
	slotOf,
} from '../../lib/tweakspace'
import { pushNotice } from '../../store/chat'
import { tweakspaceFor } from '../../store/tweakspaceInstance'
import { useRoom } from '../room/model'
import { setRefusal } from '../room/move'
import { Tweaks } from './Tweaks'

/**
 * A base64url slot as a settings row: its key, the name its tweak goes by,
 * the blob itself -- to read, or to paste another over -- and the two things
 * to do with it. The pen opens the editor under the row and turns into the
 * chevron that folds it away again; what was typed stays until it is sent or
 * reset, and the row says so meanwhile. One of the map's own tables is the
 * same row with nothing to send: read, opened, refused.
 */
export function TweakRow(props: { row: Row; note?: RowNote }) {
	const room = useRoom()
	const note = () => props.note ?? NO_NOTE
	const space = tweakspaceFor(room)
	const key = () => props.row.option.key
	const id = () => slotId(key())
	const doc = () => space.ws.docs[id()]
	const open = () => space.ws.expanded === id()
	/** Why the room would refuse a paste from us, if it would. */
	const refusal = () =>
		isMapTable(key()) ? MAP_TABLE_REFUSAL : setRefusal(room)
	/** What the room holds; SPADS's `0` is nothing. */
	const blob = () => {
		const now = props.row.current ?? ''
		return isCleared(now) ? '' : now
	}
	/**
	 * The field's tip: why it is read-only, else how long the blob is. Never
	 * the blob -- a tip is a native window on Linux, and a full slot on one
	 * unbroken line is wider than GTK can draw.
	 */
	const tip = () => {
		const refused = refusal()
		if (refused !== null) return refused
		const length = blob().length
		if (length === 0) return undefined
		return `${length} characters of ${KINDS[kindOf(key())].blob}`
	}

	// The JSON documents carry no name; Rust says what each holds instead.
	const [words] = createResource(
		() => kindOf(key()) === 'boxes' && blob(),
		(raw) => api.describeMapOption(key(), raw).catch(() => null),
	)
	const name = () => doc()?.name ?? words() ?? ''

	// Opened, the row goes to the top of the column under the row above it, and
	// the editor fills the rest but for the row below: a slot is found among
	// its neighbours, and the send bar is on screen without scrolling. The
	// neighbours are measured, since any row can stand beside a tweak's.
	let line: HTMLDivElement | undefined
	let card: HTMLDivElement | undefined
	const [around, setAround] = createSignal(0)
	createEffect(() => {
		if (!open() || !line) return
		const above = line.previousElementSibling
		const below = card?.nextElementSibling
		const top = line.getBoundingClientRect().top
		setAround(
			(above ? top - above.getBoundingClientRect().top : 0) +
				(below?.getBoundingClientRect().height ?? 0),
		)
		;(above ?? line).scrollIntoView?.({ block: 'start', behavior: 'smooth' })
	})

	async function copy() {
		try {
			await navigator.clipboard.writeText(`!bSet ${key()} ${blob()}`)
			pushNotice('info', `!bSet ${key()} copied`)
		} catch (error) {
			pushNotice('warning', `copy: ${describeError(error)}`)
		}
	}

	/**
	 * A pasted blob, set as it is. Decoded first, so what cannot be read never
	 * reaches the room, then sent the way the editor sends: Rust prepares it
	 * again and refuses what the server would cut short. Emptying the field is
	 * not clearing the slot -- that is the editor's Clear, on purpose.
	 */
	async function commit(input: HTMLInputElement) {
		const next = input.value.trim()
		const slot = slotOf(key())
		if (next === '' || next === blob() || slot === null) {
			input.value = blob()
			return
		}
		try {
			const view = await space.decode(next, kindOf(key()))
			// As it came: a pasted blob goes back out the same.
			await room.io.tweakSend(view.text, slot, true, false)
			pushNotice('info', `${key()} set`)
		} catch (error) {
			input.value = blob()
			pushNotice('warning', `${key()}: ${describeError(error)}`)
		}
	}

	return (
		<>
			<div
				ref={line}
				class='opt tweak'
				classList={{
					changed: props.row.changed,
					open: open(),
					recent: note().recent,
					on: note().on,
				}}
				data-key={key()}
			>
				<span class='mark' />
				<OptionName
					option={props.row.option}
					after={
						<span class='tweak-note'>
							<span class='tweak-name' title={name()}>
								{name()}
							</span>
							<Show when={doc()?.stale}>
								<span
									class='doc-tag'
									title='Somebody set this slot while you were editing it'
								>
									room moved
								</span>
							</Show>
							<Show when={doc() && isDirty(doc()!)}>
								<span
									class='doc-tag dirty'
									title='Edited here and not sent yet'
								>
									unsent
								</span>
							</Show>
							<Show when={note().vote !== null}>
								<span
									class='doc-tag opt-vote'
									title='A vote under way would change this'
								>
									vote
								</span>
							</Show>
						</span>
					}
				/>
				<input
					class='v-edit tweak-input'
					value={blob()}
					readOnly={refusal() !== null}
					placeholder={refusal() === null ? 'paste base64url' : '—'}
					aria-label={`${key()} as base64url`}
					title={tip()}
					spellcheck={false}
					autocomplete='off'
					onFocus={(event) => event.currentTarget.select()}
					onChange={(event) => void commit(event.currentTarget)}
				/>
				<ActionCell filled={blob() !== ''}>
					<CellButton
						icon='act-copy'
						title={`Copy the !bSet command for ${key()}`}
						disabled={blob() === ''}
						onClick={() => void copy()}
					/>
					<CellButton
						class='slot-open'
						icon={open() ? 'act-expand' : 'act-pen'}
						title={
							open()
								? 'Fold the editor away; what you typed is kept'
								: blob() === ''
									? `Write into ${key()}`
									: `Open ${key()} in the editor`
						}
						onClick={() => space.expand(open() ? null : id())}
					/>
				</ActionCell>
			</div>
			<Show when={open()}>
				{/* The row's own for the list's purposes: a hand on the editor
				    is a hand on its row. */}
				<div
					ref={card}
					class='tweak-inline'
					data-key={key()}
					style={{ '--around': `${around()}px` }}
				>
					<Tweaks />
				</div>
			</Show>
		</>
	)
}
