import { For, Show, createMemo, createSignal } from 'solid-js'
import { api, describeError } from '../ipc/client'
import { pushNotice } from '../store/chat'
import { lobby } from '../store/lobby'

/**
 * Running a room you boss.
 *
 * These are SPADS commands, sent as chat exactly as anyone would type them —
 * `!balance`, `!start`, `!lock`. There is no protocol behind them beyond
 * `SAYBATTLE`, which is why this needs nothing in the runtime: the throttle
 * policy already routes a `!` line through the command bucket.
 *
 * Shown only to the room's boss. Everyone else would just be collecting
 * refusals, and SPADS answers a refused command with a private message that
 * lands in the server room anyway.
 */
export function HostBar() {
  const [busy, setBusy] = createSignal(false)
  const [size, setSize] = createSignal('')

  const boss = createMemo(
    () => lobby.myBattle?.boss !== null && lobby.myBattle?.boss === lobby.me,
  )
  const room = createMemo(() => {
    const id = lobby.myBattle?.id
    return id === undefined ? undefined : lobby.battles[id]
  })

  async function run(command: string) {
    setBusy(true)
    try {
      await api.sayBattle(command)
    } catch (error) {
      pushNotice('warning', `${command}: ${describeError(error)}`)
    } finally {
      setBusy(false)
    }
  }

  /** The commands worth a button; everything else is still typeable. */
  const actions = createMemo(() => {
    const locked = room()?.locked ?? false
    return [
      ['Balance', '!balance', 'Even the teams by skill'],
      ['Fix colours', '!fixColors', 'Give every team a distinct colour'],
      [
        locked ? 'Unlock' : 'Lock',
        locked ? '!unlock' : '!lock',
        locked ? 'Let people join again' : 'Stop anyone else joining',
      ],
      ['Start', '!start', 'Start the game once everyone is ready'],
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
          Team size
          <input
            type='number'
            min='1'
            max='16'
            placeholder={String(room()?.layout?.teamSize ?? '')}
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
