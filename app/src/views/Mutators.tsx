import { For, Show, createMemo } from 'solid-js'
import { describeError } from '../ipc/client'
import { age, exactly } from '../lib/age'
import { offers, sameMutator, sourceWords } from '../lib/mutators'
import { pushNotice } from '../store/chat'
import { useRoom } from './room/model'
import { bossing, mutatorRefusal } from './room/move'

/**
 * What the room loads on top of its game, and what its host offers to add:
 * the pane's third face, in a room whose host runs mutators.
 *
 * Every change is a command to the host -- `!mutator add`, `remove`,
 * `update` -- so a boss's is taken and a player's is put to the room as a
 * vote. The buttons stay where they are for anyone else, greyed, saying why.
 */
export function Mutators() {
	const room = useRoom()
	const loaded = () => room.check().mutators
	const offered = createMemo(() => offers(room.my()?.scriptTags))
	const addable = () =>
		offered().filter(
			(offer) => !loaded().some((mutator) => sameMutator(mutator, offer)),
		)
	const refusal = () => mutatorRefusal(room)
	const verb = (word: string) =>
		bossing(room) ? word : `Vote to ${word.toLowerCase()}`
	const say = (command: string) =>
		void room.io
			.sayBattle(command)
			.catch((error) => pushNotice('warning', describeError(error)))

	return (
		<div class='mutators'>
			<div class='toolbar'>
				<span class='note'>
					{refusal() ?? 'Loaded in this order, each on top of the last'}
				</span>
				<span class='spacer' />
				<Show when={loaded().some((mutator) => mutator.source)}>
					<button
						class='chip-choice'
						disabled={refusal() !== null}
						title='Move each one from GitHub to the newest commit of its branch, in this room'
						onClick={() => say('!mutator update')}
					>
						{verb('Update')}
					</button>
				</Show>
			</div>
			<div class='mutator-rows'>
				<div class='mutator-head'>Loaded</div>
				<For
					each={loaded()}
					fallback={<div class='setup-empty'>Nothing is loaded</div>}
				>
					{(mutator) => {
						// The host knows a loaded mutator by the name it was added by.
						const name = () =>
							offered().find((offer) => sameMutator(mutator, offer))?.name ??
							mutator.title
						return (
							<Row
								name={mutator.title}
								source={mutator.source}
								date={mutator.date}
								state={mutator.here ? null : 'not here yet'}
								action={verb('Remove')}
								refusal={refusal()}
								onAct={() => say(`!mutator remove ${name()}`)}
							/>
						)
					}}
				</For>
				<div class='mutator-head'>Offered by the host</div>
				<For
					each={addable()}
					fallback={
						<div class='setup-empty'>Everything it offers is loaded</div>
					}
				>
					{(offer) => (
						<Row
							name={offer.name}
							source={offer.source}
							date={offer.date}
							state={null}
							action={verb('Add')}
							refusal={refusal()}
							onAct={() => say(`!mutator add ${offer.name}`)}
						/>
					)}
				</For>
			</div>
		</div>
	)
}

/** One mutator: its name, how recent it is, and the one thing to do with it. */
function Row(props: {
	name: string
	source: string | null
	date: string | null
	state: string | null
	action: string
	refusal: string | null
	onAct: () => void
}) {
	const tip = () =>
		props.source &&
		`${sourceWords(props.source)}${props.date ? `, committed ${exactly(props.date)}` : ''}`
	return (
		<div class='mutator-row' title={tip() || undefined}>
			<span class='mutator-name'>{props.name}</span>
			<span class='mutator-age'>
				{props.state ?? (props.date ? age(props.date) : '')}
			</span>
			<button
				class='chip-choice'
				disabled={props.refusal !== null}
				title={props.refusal ?? undefined}
				onClick={() => props.onAct()}
			>
				{props.action}
			</button>
		</div>
	)
}
