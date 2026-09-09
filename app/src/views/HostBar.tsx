import { For, Show, createMemo, createSignal } from 'solid-js'
import { describeError } from '../ipc/client'
import { pushNotice } from '../store/chat'
import { useRoom } from './room/model'

/**
 * Running a room you boss.
 *
 * These are SPADS commands, sent as chat exactly as anyone would type them —
 * `!balance`, `!forceStart`, `!lock`. There is no protocol behind them beyond
 * `SAYBATTLE`, which is why this needs nothing in the runtime: the throttle
 * policy already routes a `!` line through the command bucket.
 *
 * Shown only to the room's boss. Everyone else would just be collecting
 * refusals, and SPADS answers a refused command with a private message that
 * lands in the server room anyway.
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

  const boss = createMemo(() => {
    const who = room.my()?.boss
    return who !== null && who === room.me()
  })

  async function run(command: string) {
    setBusy(true)
    try {
      await room.io.sayBattle(command)
    } catch (error) {
      pushNotice('warning', `${command}: ${describeError(error)}`)
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
    <Show when={boss()}>
      <div class='host-bar'>
        <span class='filter-label'>Your room</span>

        <For each={actions()}>
          {([label, command, hint]) => (
            <button
              disabled={busy()}
              title={`${command} — ${hint}`}
              onClick={() => void run(command)}
            >
              {label}
            </button>
          )}
        </For>

        <label class='host-size'>
          Preset
          <select
            disabled={busy()}
            title='!preset — the settings the room starts from'
            value={room.my()?.preset ?? ''}
            onChange={(e) => void run(`!preset ${e.currentTarget.value}`)}
          >
            <For each={presets()}>
              {(name) => <option value={name}>{name || '—'}</option>}
            </For>
          </select>
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
          Anything else still works by typing it, like <code>!map</code>.
        </span>
      </div>
    </Show>
  )
}
