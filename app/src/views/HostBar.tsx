import { Select } from '../components/Select'
import { For, Show, createMemo, createSignal } from 'solid-js'
import { describeError } from '../ipc/client'
import { isBoss } from '../lib/roster'
import { pushNotice } from '../store/chat'
import { useRoom } from './room/model'

/**
 * Running a room you boss, or putting its running to a vote.
 *
 * These are SPADS commands, sent as chat exactly as anyone would type them —
 * `!balance`, `!forceStart`, `!lock`. There is no protocol behind them beyond
 * `SAYBATTLE`, which is why this needs nothing in the runtime: the throttle
 * policy already routes a `!` line through the command bucket.
 *
 * The boss says them. A seated player proposes each as a vote, `!cv` ahead
 * of it, and the room reads what was asked rather than an auto-callvote's
 * rewording of it; which votes a room takes is its host's to say, and SPADS
 * answers a refused one in chat. A spectator is shown nothing: SPADS gives a
 * spectator no vote at all (`commands.conf` `[callVote]`).
 */
/** BAR's SPADS presets, as Chobby lists them (`gui_battle_room_window.lua:3490`). */
const PRESETS = ['team', 'ffa', 'coop', 'duel', 'tourney', 'custom']

export function HostBar() {
	const room = useRoom()
	const [busy, setBusy] = createSignal(false)
	const [size, setSize] = createSignal('')

	/**
	 * What the preset picker lists: the known six, plus whatever the room says
	 * it runs under when that is none of them, and a blank where it has not
	 * said -- a host without the BarManager plugin never reports one.
	 */
	const presets = createMemo(() => {
		const now = room.my()?.preset ?? null
		if (now === null) return ['', ...PRESETS]
		return PRESETS.includes(now) ? PRESETS : [now, ...PRESETS]
	})

	const boss = createMemo(() => isBoss(room.my()?.boss, room.me()))
	const seated = createMemo(() => {
		const me = room.me()
		return me !== null && (room.users()[me]?.battleStatus?.player ?? false)
	})

	/** The line as the boss says it, or as a player puts it to the room. */
	const command = (line: string) => (boss() ? line : `!cv ${line.slice(1)}`)

	async function run(line: string) {
		const said = command(line)
		setBusy(true)
		try {
			await room.io.sayBattle(said)
		} catch (error) {
			pushNotice('warning', `${said}: ${describeError(error)}`)
		} finally {
			setBusy(false)
		}
	}

	/** The commands worth a button; everything else is still typeable. */
	const actions = createMemo(() => {
		const locked = room.battle()?.locked ?? false
		// While the room balances itself, SPADS refuses to move anybody by hand --
		// which is what dragging a player asks it to do (`spads.pl:8886`).
		const auto = room.my()?.autoBalance
		const balancing = auto !== null && auto !== undefined && auto !== 'off'
		return [
			['Balance', '!balance', 'Even the teams by skill'],
			[
				balancing ? 'Auto balance off' : 'Auto balance on',
				balancing ? '!autoBalance off' : '!autoBalance advanced',
				balancing
					? 'Stop the room arranging its own teams, so players can be moved'
					: 'Let the room arrange its own teams again',
			],
			['Fix colours', '!fixColors', 'Give every team a distinct colour'],
			[
				locked ? 'Unlock' : 'Lock',
				locked ? '!unlock' : '!lock',
				locked ? 'Let people join again' : 'Stop anyone else joining',
			],
			// `!start` is the card's button, beside Leave room, for boss and
			// player alike. Only the impatient form lives here.
			['Force start', '!forceStart', 'Start without waiting for everyone'],
		] as const
	})

	return (
		<Show when={boss() || seated()}>
			<div class='host-bar'>
				<span class='filter-label'>{boss() ? 'Your room' : 'Call vote'}</span>

				<For each={actions()}>
					{([label, line, hint]) => (
						<button
							disabled={busy()}
							title={`${command(line)} — ${hint}`}
							onClick={() => void run(line)}
						>
							{label}
						</button>
					)}
				</For>

				<label class='host-size'>
					Preset
					<Select
						disabled={busy()}
						title={`${command('!preset')} — the settings the room starts from`}
						value={room.my()?.preset ?? ''}
						onChange={(e) => void run(`!preset ${e.currentTarget.value}`)}
					>
						<For each={presets()}>
							{(name) => <option value={name}>{name || '—'}</option>}
						</For>
					</Select>
				</label>

				<label class='host-size'>
					Team size
					<input
						type='number'
						min='1'
						max='16'
						placeholder={String(room.battle()?.layout?.teamSize ?? '')}
						value={size()}
						onInput={(e) => setSize(e.currentTarget.value)}
						onChange={(e) => {
							const wanted = Number(e.currentTarget.value)
							if (wanted >= 1 && wanted <= 16)
								void run(`!set teamSize ${wanted}`)
						}}
					/>
				</label>

				<span class='spacer' />
				<span class='muted'>
					Anything else still works by typing it, like{' '}
					<code>{command('!map')}</code>.
				</span>
			</div>
		</Show>
	)
}
