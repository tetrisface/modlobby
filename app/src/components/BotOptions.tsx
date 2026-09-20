import {
	Show,
	createEffect,
	createMemo,
	createResource,
	createSignal,
} from 'solid-js'
import type { BotView } from '../ipc/bindings/BotView'
import { api, describeError } from '../ipc/client'
import { isGameMode } from '../lib/roster'
import { rowsOf } from '../lib/setup'
import { pushNotice } from '../store/chat'
import { Rows } from '../views/Setup'
import { useRoom } from '../views/room/model'
import { setBonus } from '../views/room/move'

/**
 * What an AI can be told about itself.
 *
 * An engine AI declares its own options in `AIOptions.lua`, which is the same
 * `local options = { … }` table BAR's `modoptions.lua` is — so the same parser
 * reads it and the same rows draw it, tabs and groups and all. What is set
 * here rides into the start script as that AI's `[options]` block, which is
 * where its difficulty lives.
 *
 * The resource bonus is not one of those: it is a `[team]` key the engine
 * reads for any team, AI or not. It is here anyway, above them, because from
 * where you are standing it is the same question — how hard this one is.
 *
 * A game's Lua AIs — Scavengers, Raptors — have no such file. They are
 * configured by the game's own modoptions, in the pane on the right, and this
 * says so rather than showing an empty list.
 */
export function BotOptions(props: { bot: BotView; onClose: () => void }) {
	const room = useRoom()

	const [catalogue] = createResource(
		() => [room.battle()?.engineVersion ?? '', props.bot.ai] as const,
		([engine, ai]) =>
			engine === '' ? [] : api.aiOptions(engine, ai).catch(() => []),
	)

	/** The room's own record of this AI: what it has been told, and given. */
	const held = createMemo(() =>
		room.battle()?.bots.find((bot) => bot.name === props.bot.name),
	)

	const rows = createMemo(() =>
		rowsOf({ name: '', options: catalogue() ?? [] }, held()?.options ?? {}),
	)

	/**
	 * The bonus while the slider is being dragged; the room's own value the
	 * moment it lands. Committed on `change` rather than `input`, so one drag
	 * is one message and not thirty.
	 */
	const [wanted, setWanted] = createSignal(props.bot.status.handicap)
	createEffect(() => setWanted(held()?.status.handicap ?? 0))

	/** Runs one change, saying why it did not happen. */
	async function tell(run: () => Promise<void>) {
		try {
			await run()
		} catch (error) {
			pushNotice('warning', `${props.bot.name}: ${describeError(error)}`)
		}
	}

	const set = (key: string, value: string) =>
		tell(() => room.io.setBotOption(props.bot.name, key, value))

	function give(percent: number) {
		const bot = held() ?? props.bot
		return tell(() =>
			setBonus(
				room,
				{
					kind: 'bot',
					name: bot.name,
					mine: true,
					team: bot.status.team,
					handicap: bot.status.handicap,
					colour: bot.teamColour,
				},
				percent,
				bot.status.allyTeam,
			),
		)
	}

	return (
		<div class='sheet' onMouseDown={props.onClose}>
			<div
				class='sheet-card bot-options'
				onMouseDown={(event) => event.stopPropagation()}
			>
				<header class='ed-head'>
					<h2>
						{props.bot.name} <span class='muted'>{props.bot.ai}</span>
					</h2>
					<span class='spacer' />
					<button type='button' onClick={props.onClose}>
						Close
					</button>
				</header>

				{/* A side the game runs has no economy of its own, so there is
            nothing for a bonus to multiply.

            Called what the menu and the Add AI sheet call it, and not
            "resource bonus": an AI may declare an option of that name below
            -- the engine's sample AI does -- and the two are unrelated. This
            one the engine applies to the team; that one is passed to the AI
            and means whatever the AI makes of it. */}
				<Show when={!isGameMode(props.bot.ai)}>
					<label
						class='bot-bonus-set'
						title='Extra metal and energy income, as a percentage. The engine applies it to this AI’s team, whatever the AI itself does.'
					>
						Bonus
						<input
							type='range'
							min={0}
							max={100}
							step={5}
							value={wanted()}
							onInput={(event) => setWanted(Number(event.currentTarget.value))}
							onChange={(event) => void give(Number(event.currentTarget.value))}
						/>
						<input
							type='number'
							min={0}
							max={100}
							value={wanted()}
							onChange={(event) => void give(Number(event.currentTarget.value))}
						/>
						%
					</label>
				</Show>

				<Show
					when={!catalogue.loading}
					fallback={<p class='muted setup-empty'>Reading what it takes…</p>}
				>
					<Show
						when={rows().length > 0}
						fallback={
							<p class='muted setup-note'>
								{props.bot.ai} declares no options of its own. A game's own AIs
								— Scavengers, Raptors — are set up in the room's settings
								instead.
							</p>
						}
					>
						<div class='setup-detail'>
							<Rows rows={rows()} editable onEdit={() => {}} set={set} />
						</div>
					</Show>
				</Show>
			</div>
		</div>
	)
}
