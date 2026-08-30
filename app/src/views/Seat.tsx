import { For, Show, createMemo, createSignal } from 'solid-js'
import { SideIcon } from '../components/icons'
import { api, describeError } from '../ipc/client'
import { pushNotice } from '../store/chat'
import { lobby } from '../store/lobby'
import { settings } from '../store/settings'

const REGIONS = ['EU', 'US', 'AU', 'EA']

/** side 2 is Random; Legion needs its modoption, so it is offered last. */
const SIDES = [
  { id: 0, label: 'Armada' },
  { id: 1, label: 'Cortex' },
  { id: 2, label: 'Random' },
  { id: 3, label: 'Legion' },
]

/**
 * Playing rather than watching.
 *
 * A seat in a public room is refused unless the owner has said otherwise —
 * that slot belongs to a real player waiting for a game. A room a cluster
 * manager made on request is passworded and ours, and needs no such licence.
 */
export function Seat() {
  const [region, setRegion] = createSignal(REGIONS[0] as string)
  const [busy, setBusy] = createSignal(false)

  const room = createMemo(() => {
    const id = lobby.myBattle?.id
    return id === undefined ? undefined : lobby.battles[id]
  })
  const me = createMemo(() =>
    lobby.me === null ? undefined : lobby.users[lobby.me],
  )
  const seat = () => me()?.battleStatus
  const seated = () => seat()?.player ?? false
  /** Ours if it was given to us, or if SPADS says we are bossing it. */
  const ours = () =>
    (room()?.passworded ?? false) ||
    (lobby.myBattle?.boss !== null && lobby.myBattle?.boss === lobby.me)
  /**
   * A room whose game has started has no seat to take: the engine will not
   * admit a latecomer as a player, and claiming a slot disturbs a game already
   * in progress. The runtime refuses it too; this is so the button never
   * appears in the first place.
   */
  const running = () => lobby.gameRunning !== null
  const allowed = () =>
    !running() && (ours() || (settings()?.play.inPublicRooms ?? false))

  /**
   * Ally teams already in use, plus the next free one — you can join a side or
   * open a new one, and nothing else would mean anything.
   */
  const allyTeams = createMemo(() => {
    const battle = room()
    if (!battle) return [0]
    const used = new Set<number>()
    for (const name of battle.members) {
      const status = lobby.users[name]?.battleStatus
      if (status?.player) used.add(status.allyTeam)
    }
    for (const bot of battle.bots) used.add(bot.status.allyTeam)
    const sorted = [...used].sort((a, b) => a - b)
    const next = sorted.length === 0 ? 0 : (sorted[sorted.length - 1] ?? 0) + 1
    return [...sorted, next]
  })

  /** The lowest team number nobody holds, so two players never collide. */
  function freeTeam(): number {
    const battle = room()
    const taken = new Set<number>()
    if (battle) {
      for (const name of battle.members) {
        const status = lobby.users[name]?.battleStatus
        if (status?.player && name !== lobby.me) taken.add(status.team)
      }
      for (const bot of battle.bots) taken.add(bot.status.team)
    }
    let team = 0
    while (taken.has(team)) team += 1
    return team
  }

  async function act(what: string, run: () => Promise<void>) {
    setBusy(true)
    try {
      await run()
    } catch (error) {
      pushNotice('warning', `${what}: ${describeError(error)}`)
    } finally {
      setBusy(false)
    }
  }

  return (
    <div class='seat'>
      <Show
        when={allowed()}
        fallback={
          <span class='muted'>
            <Show
              when={running()}
              fallback="Spectating. A seat here would take a real player's slot — host a room to play, or allow public seats in Settings."
            >
              This game has already started — you can watch it.
            </Show>
          </span>
        }
      >
        <Show
          when={seated()}
          fallback={
            <>
              <span class='muted'>Spectating.</span>
              <For each={allyTeams()}>
                {(ally, index) => (
                  <button
                    disabled={busy()}
                    onClick={() =>
                      act('take a seat', () => api.takeSeat(freeTeam(), ally))
                    }
                  >
                    {index() === allyTeams().length - 1
                      ? `New team ${ally + 1}`
                      : `Join team ${ally + 1}`}
                  </button>
                )}
              </For>
            </>
          }
        >
          <span>Team {(seat()?.allyTeam ?? 0) + 1}</span>

          <button
            class={seat()?.ready ? 'primary' : ''}
            disabled={busy()}
            onClick={() =>
              act('ready', () => api.setReady(!(seat()?.ready ?? false)))
            }
          >
            {seat()?.ready ? 'Ready' : 'Not ready'}
          </button>

          <select
            value={String(seat()?.side ?? 0)}
            disabled={busy()}
            onChange={(e) =>
              act('faction', () => api.setSide(Number(e.currentTarget.value)))
            }
          >
            <For each={SIDES}>
              {(side) => <option value={String(side.id)}>{side.label}</option>}
            </For>
          </select>
          <SideIcon side={seat()?.side ?? 0} />

          <button
            disabled={busy()}
            onClick={() => act('spectate', () => api.releaseSeat())}
          >
            Spectate
          </button>
        </Show>
      </Show>

      <Show when={lobby.content}>
        {(content) => {
          const missing = () =>
            (['engine', 'game', 'map'] as const).filter(
              (what) => !content()[what],
            )
          return (
            <Show
              when={missing().length}
              fallback={<span class='synced'>content ready</span>}
            >
              <span class='error'>missing {missing().join(', ')}</span>
            </Show>
          )
        }}
      </Show>

      <span class='spacer' />
      <select
        value={region()}
        onChange={(e) => setRegion(e.currentTarget.value)}
      >
        {REGIONS.map((r) => (
          <option value={r}>{r}</option>
        ))}
      </select>
      {/* Both halves of Chobby's Host button: an empty autohost is a listed
          room you boss, `!privatehost` is a passworded one made on request. */}
      <button
        disabled={busy()}
        onClick={() =>
          act('host a room', async () => {
            await api.hostPublic(region())
            pushNotice('info', 'joined an empty room; you are its boss')
          })
        }
      >
        Host a public room
      </button>
      <button
        disabled={busy()}
        onClick={() =>
          act('host a room', async () => {
            const manager = await api.requestPrivateHost(region())
            pushNotice(
              'info',
              `asked ${manager} for a private room; joining when it opens`,
            )
          })
        }
      >
        Private room
      </button>
    </div>
  )
}
