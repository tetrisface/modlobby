import { Show, createEffect, createResource } from 'solid-js'
import { ActionCell, CellButton } from '../../components/ActionCell'
import { api, describeError } from '../../ipc/client'
import { BOX_OVERRIDE } from '../../lib/boxes'
import { isCleared, label, type Row } from '../../lib/setup'
import { isDirty, kindOf, slotId, slotOf } from '../../lib/tweakspace'
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
 * reset, and the row says so meanwhile.
 */
export function TweakRow(props: { row: Row }) {
	const room = useRoom()
	const space = tweakspaceFor(room)
	const key = () => props.row.option.key
	const id = () => slotId(key())
	const doc = () => space.ws.docs[id()]
	const open = () => space.ws.expanded === id()
	/** Why the room would refuse a paste from us, if it would. */
	const refusal = () => setRefusal(room)
	/** What the room holds; SPADS's `0` is nothing. */
	const blob = () => {
		const now = props.row.current ?? ''
		return isCleared(now) ? '' : now
	}

	// The override's JSON carries no name; Rust says what it holds instead.
	const [words] = createResource(
		() => key() === BOX_OVERRIDE && blob(),
		(raw) => api.describeMapOption(BOX_OVERRIDE, raw).catch(() => null),
	)
	const name = () => doc()?.name ?? words() ?? ''

	// Opened, the row goes to the top of the column and the editor fills the
	// rest, send bar included -- nothing to scroll for to find it.
	let line: HTMLDivElement | undefined
	createEffect(() => {
		if (open()) line?.scrollIntoView?.({ block: 'start', behavior: 'smooth' })
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
				classList={{ changed: props.row.changed, open: open() }}
			>
				<span class='mark' />
				<span class='k' title={props.row.option.desc ?? ''}>
					{label(props.row.option)}
				</span>
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
						<span class='doc-tag dirty' title='Edited here and not sent yet'>
							unsent
						</span>
					</Show>
				</span>
				<input
					class='v-edit tweak-input'
					value={blob()}
					readOnly={refusal() !== null}
					placeholder={refusal() === null ? 'paste base64url' : '—'}
					aria-label={`${key()} as base64url`}
					title={refusal() ?? blob()}
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
				<div class='tweak-inline'>
					<Tweaks />
				</div>
			</Show>
		</>
	)
}
