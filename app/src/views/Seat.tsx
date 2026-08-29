import { Show, createMemo, createSignal } from 'solid-js'
import { api, describeError } from '../ipc/client'
import { pushNotice } from '../store/chat'
import { lobby } from '../store/lobby'

const REGIONS = ['EU', 'US', 'AU', 'EA']

/**
 * Taking a seat and getting a room to take it in.
 *
 * modlobby spectates public rooms and never takes a slot in one — that slot is
 * someone else's game. A room a cluster manager made on request is passworded
 * and ours, which is the condition the runtime enforces.
 */
export function Seat() {
  const [region, setRegion] = createSignal(REGIONS[0] as string)
  const [busy, setBusy] = createSignal(false)

  const room = createMemo(() => {
    const id = lobby.myBattle?.id
    return id === undefined ? undefined : lobby.battles[id]
  })
  const seated = createMemo(
    () =>
      lobby.me !== null &&
      (lobby.users[lobby.me]?.battleStatus?.player ?? false),
  )

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
        when={room()?.passworded}
        fallback={
          <span class='muted'>
            Spectating. A seat here would take a real player's slot — host a
            room to play.
          </span>
        }
      >
        <Show
          when={seated()}
          fallback={
            <button
              class='primary'
              disabled={busy()}
              onClick={() => act('take a seat', () => api.takeSeat(0, 0))}
            >
              Take a seat
            </button>
          }
        >
          <span>Playing on team 0</span>
          <button
            disabled={busy()}
            onClick={() => act('spectate', () => api.releaseSeat())}
          >
            Back to spectating
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
        Host a private room
      </button>
    </div>
  )
}
