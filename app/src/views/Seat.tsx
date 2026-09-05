import { For, Show, createEffect, createMemo, createSignal } from 'solid-js'
import { SideIcon } from '../components/icons'
import type { AiChoice } from '../ipc/bindings/AiChoice'
import { api, describeError } from '../ipc/client'
import { freeTeam } from '../lib/roster'
import { pushNotice } from '../store/chat'
import { lobby, myRoom } from '../store/lobby'
import { applySettings, settings } from '../store/settings'

/** side 2 is Random; Legion needs its modoption, so it is offered last. */
const SIDES = [
  { id: 0, label: 'Armada' },
  { id: 1, label: 'Cortex' },
  { id: 2, label: 'Random' },
  { id: 3, label: 'Legion' },
]

/** Ours if it was given to us, or if SPADS says we are bossing it. */
function ours(): boolean {
  return (
    (myRoom()?.passworded ?? false) ||
    (lobby.myBattle?.boss !== null && lobby.myBattle?.boss === lobby.me)
  )
}

/** Whether a seat may be taken here: our own room, or the setting says so. */
export function seatsAllowed(): boolean {
  return ours() || (settings()?.play.inPublicRooms ?? false)
}

/** The lowest team number nobody else in the room holds. */
function nextTeam(): number {
  const room = myRoom()
  return room ? freeTeam(room, lobby.users, lobby.me) : 0
}

/** What `remember` remembers, kept current by what you actually do. */
async function remember(played: boolean) {
  try {
    applySettings(await api.rememberPlayed(played))
  } catch {
    // A preference we could not write is not worth interrupting a game for.
  }
}

/**
 * Sits on ally team `ally` — joining it, or moving there from another — and
 * makes playing what `remember` remembers. Taking a seat resets ready, in the
 * runtime and by SPADS alike, so moving sides is one action and not two.
 */
export async function sitOn(ally: number): Promise<void> {
  await api.takeSeat(nextTeam(), ally)
  await remember(true)
}

/**
 * Playing rather than watching.
 *
 * Sitting down is what a lobby is for, so the seats are simply here. The
 * setting behind them exists for a client with nobody at the keyboard, and a
 * room of your own — passworded, or one SPADS says you boss — never consults
 * it at all.
 */
export function Seat() {
  const [busy, setBusy] = createSignal(false)

  const room = createMemo(myRoom)
  const me = createMemo(() =>
    lobby.me === null ? undefined : lobby.users[lobby.me],
  )
  const seat = () => me()?.battleStatus
  const seated = () => seat()?.player ?? false
  const running = () => lobby.gameRunning !== null
  const allowed = seatsAllowed

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

  /** The ally team a seat would join: the emptiest one already in play. */
  function freeAlly(): number {
    const battle = room()
    if (!battle) return 0
    const held = new Map<number, number>()
    for (const name of battle.members) {
      const status = lobby.users[name]?.battleStatus
      if (status?.player)
        held.set(status.allyTeam, (held.get(status.allyTeam) ?? 0) + 1)
    }
    for (const bot of battle.bots)
      held.set(bot.status.allyTeam, (held.get(bot.status.allyTeam) ?? 0) + 1)
    const teams = allyTeams()
    // The last entry is always the new, empty one; prefer a side that exists.
    const existing = teams.slice(0, -1)
    if (existing.length === 0) return teams[0] ?? 0
    return existing.reduce((best, ally) =>
      (held.get(ally) ?? 0) < (held.get(best) ?? 0) ? ally : best,
    )
  }

  /**
   * Sits down on arrival when that is the posture, once per room.
   *
   * Once, so that leaving your seat is not immediately undone — and leaving it
   * also changes what `remember` remembers, so the next room agrees with what
   * you just did.
   *
   * The team is picked from the members known at that moment, which on a busy
   * room may be a moment before the last of them has arrived. Two people can
   * therefore land on one team number, exactly as they can when a person
   * clicks the button the instant they walk in — and it matters as little,
   * because SPADS assigns teams itself when the game starts.
   */
  let seatedIn: number | undefined
  createEffect(() => {
    const battle = room()
    const play = settings()?.play
    if (!battle || !play || seated() || !allowed()) return
    if (seatedIn === battle.id) return
    const wanted =
      play.joinAs === 'remember' ? play.lastWasPlayer : play.joinAs === 'player'
    if (!wanted) return
    seatedIn = battle.id
    void act('take a seat', () => api.takeSeat(nextTeam(), freeAlly()))
  })

  /** Runs one action, telling the user why it did not happen; true if it did. */
  async function act(what: string, run: () => Promise<void>): Promise<boolean> {
    setBusy(true)
    try {
      await run()
      return true
    } catch (error) {
      pushNotice('warning', `${what}: ${describeError(error)}`)
      return false
    } finally {
      setBusy(false)
    }
  }

  const SPECTATOR = 'spectator'
  /** What the seat picker shows: the side we hold, or the spectator row. */
  const current = () => (seated() ? String(seat()?.allyTeam ?? 0) : SPECTATOR)

  /**
   * Sits, moves, or stands up as picked. A refused pick snaps the picker
   * back, since the row it landed on never came true.
   */
  async function pickSeat(picker: HTMLSelectElement) {
    const choice = picker.value
    const done =
      choice === SPECTATOR
        ? await act('spectate', async () => {
            await api.releaseSeat()
            await remember(false)
          })
        : await act('take a seat', () => sitOn(Number(choice)))
    if (!done) picker.value = current()
  }

  return (
    <div class='seat'>
      <Show
        when={allowed()}
        fallback={
          <span class='muted'>Spectating; seats are off in Settings.</span>
        }
      >
        <select
          value={current()}
          disabled={busy()}
          onChange={(e) => void pickSeat(e.currentTarget)}
        >
          <For each={allyTeams()}>
            {(ally, index) => (
              <option value={String(ally)}>
                {index() === allyTeams().length - 1
                  ? 'New team'
                  : current() === String(ally)
                    ? `Team ${ally + 1}`
                    : `Join team ${ally + 1}`}
              </option>
            )}
          </For>
          <option value={SPECTATOR}>Spectator</option>
        </select>

        <Show when={seated()}>
          {/* Sitting down mid-game puts you in the lineup for the next one,
              which is worth saying so nobody waits for this one to let them in. */}
          <Show when={running()}>
            <span class='muted'>next game</span>
          </Show>

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
        </Show>
      </Show>

      <AddAi busy={busy()} act={act} freeTeam={nextTeam} freeAlly={freeAlly} />

      <Show when={lobby.content}>
        {(content) => {
          const missing = () =>
            (['engine', 'game', 'map'] as const).filter(
              (what) => !content()[what],
            )
          return (
            <Show when={missing().length}>
              <span class='error'>missing {missing().join(', ')}</span>
            </Show>
          )
        }}
      </Show>

      <span class='spacer' />
      {/* Both halves of Chobby's Host button: an empty autohost is a listed
          room you boss, `!privatehost` is a passworded one made on request.
          Chobby asks for a region; the runtime measures instead, and says
          which room it chose and how far away it is. */}
      <button
        disabled={busy()}
        onClick={() =>
          act('host a room', async () => {
            await api.hostPublic()
          })
        }
      >
        Host a public room
      </button>
      <button
        disabled={busy()}
        onClick={() =>
          act('host a room', async () => {
            const manager = await api.requestPrivateHost()
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

/** Colours the engine can tell apart at a glance, as 0xBBGGRR. */
const BOT_COLOURS = [0x4b73f2, 0x3fd07f, 0x2fb8f0, 0x9e5ce8, 0x50a0ff, 0x8fd04b]

/**
 * An AI for the room.
 *
 * The AI runs on this machine when the game starts, which is why the choices
 * are what is installed here: the AIs the engine ships, then the ones the
 * room's game implements in Lua — Scavengers and Raptors, for BAR. There is
 * nothing to offer until that list has been read. Whether the room takes it
 * is the host's call — SPADS answers a refusal in chat, where it can be seen.
 */
function AddAi(props: {
  busy: boolean
  act: (what: string, run: () => Promise<void>) => Promise<boolean>
  freeTeam: () => number
  freeAlly: () => number
}) {
  const [ais, setAis] = createSignal<AiChoice[]>([])
  const [ai, setAi] = createSignal('')

  createEffect(() => {
    // The room's game, whose Lua AIs are part of the offer.
    const name = myRoom()?.gameName
    if (name === undefined) return
    api
      .gameAis(name)
      .then((choices) => {
        setAis(choices)
        const first = choices[0]
        if (!ai() && first) setAi(first.name)
      })
      // No data directory means no AIs to run; the control just stays away.
      .catch(() => setAis([]))
  })

  /** `BARb`, then `BARb2` — never a name the room already holds. */
  function unusedName(base: string): string {
    const battle = lobby.myBattle && lobby.battles[lobby.myBattle.id]
    const taken = new Set((battle?.bots ?? []).map((bot) => bot.name))
    if (!taken.has(base)) return base
    let n = 2
    while (taken.has(`${base}${n}`)) n += 1
    return `${base}${n}`
  }

  return (
    <Show when={ais().length > 0}>
      <span class='add-ai'>
        <select
          value={ai()}
          disabled={props.busy}
          onChange={(e) => setAi(e.currentTarget.value)}
        >
          <For each={ais()}>
            {(choice) => (
              <option value={choice.name} title={choice.desc}>
                {choice.name}
              </option>
            )}
          </For>
        </select>
        <button
          disabled={props.busy || !ai()}
          title='The AI plays from this machine'
          onClick={() =>
            props.act('add an AI', () =>
              api.addBot(
                unusedName(ai()),
                ai(),
                props.freeTeam(),
                props.freeAlly(),
                BOT_COLOURS[
                  Math.floor(Math.random() * BOT_COLOURS.length)
                ] as number,
              ),
            )
          }
        >
          Add AI
        </button>
      </span>
    </Show>
  )
}
