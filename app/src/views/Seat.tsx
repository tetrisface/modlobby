import { For, Show, createEffect, createMemo, createSignal } from 'solid-js'
import { SideIcon } from '../components/icons'
import type { AiChoice } from '../ipc/bindings/AiChoice'
import { api, describeError } from '../ipc/client'
import { DEFAULT_TEAMS, freeTeam, unusedBotName } from '../lib/roster'
import { pushNotice } from '../store/chat'
import { applySettings, settings } from '../store/settings'
import { useRoom, type RoomModel } from './room/model'

/** side 2 is Random; Legion needs its modoption, so it is offered last. */
const SIDES = [
  { id: 0, label: 'Armada' },
  { id: 1, label: 'Cortex' },
  { id: 2, label: 'Random' },
  { id: 3, label: 'Legion' },
]

/** The lowest team number nobody else in the room holds. */
function nextTeam(room: RoomModel): number {
  const battle = room.battle()
  return battle ? freeTeam(battle, room.users(), room.me()) : 0
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
export async function sitOn(room: RoomModel, ally: number): Promise<void> {
  await room.io.takeSeat(nextTeam(room), ally)
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

  const room = useRoom()
  const battleOf = createMemo(room.battle)
  const me = createMemo(() => {
    const name = room.me()
    return name === null ? undefined : room.users()[name]
  })
  const seat = () => me()?.battleStatus
  const seated = () => seat()?.player ?? false
  const running = () => room.running() !== null

  /**
   * Ally teams already in use, plus the next free one — you can join a side or
   * open a new one, and nothing else would mean anything.
   */
  /** Ally teams somebody is already sitting on. */
  const usedAllies = createMemo(() => {
    const battle = battleOf()
    const used = new Set<number>()
    if (!battle) return used
    for (const name of battle.members) {
      const status = room.users()[name]?.battleStatus
      if (status?.player) used.add(status.allyTeam)
    }
    for (const bot of battle.bots) used.add(bot.status.allyTeam)
    return used
  })

  const allyTeams = createMemo(() => {
    const battle = battleOf()
    if (!battle) return [0]
    const used = usedAllies()
    const highest = used.size === 0 ? -1 : Math.max(...used)
    // Every team the room draws, and then one more to open a new one. Gaps
    // are offered as well: an empty team between two full ones is a seat you
    // can take, not a hole in the list -- and the room is already drawing it.
    const drawn = Math.max(
      battle.layout?.teams ?? 0,
      highest + 1,
      DEFAULT_TEAMS,
    )
    return Array.from({ length: drawn + 1 }, (_, ally) => ally)
  })

  /** The ally team a seat would join: the emptiest one already in play. */
  function freeAlly(): number {
    const battle = battleOf()
    if (!battle) return 0
    const held = new Map<number, number>()
    for (const name of battle.members) {
      const status = room.users()[name]?.battleStatus
      if (status?.player)
        held.set(status.allyTeam, (held.get(status.allyTeam) ?? 0) + 1)
    }
    for (const bot of battle.bots)
      held.set(bot.status.allyTeam, (held.get(bot.status.allyTeam) ?? 0) + 1)
    const teams = allyTeams()
    // Prefer a side somebody is on; the empty ones are all equally new.
    const existing = teams.filter((ally) => held.has(ally))
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
    const battle = battleOf()
    const play = settings()?.play
    if (!battle || !play || seated()) return
    if (seatedIn === battle.id) return
    const wanted =
      play.joinAs === 'remember' ? play.lastWasPlayer : play.joinAs === 'player'
    if (!wanted) return
    seatedIn = battle.id
    void act('take a seat', () => room.io.takeSeat(nextTeam(room), freeAlly()))
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
            await room.io.releaseSeat()
            await remember(false)
          })
        : await act('take a seat', () => sitOn(room, Number(choice)))
    if (!done) picker.value = current()
  }

  return (
    <div class='seat'>
      <select
        value={current()}
        disabled={busy()}
        onChange={(e) => void pickSeat(e.currentTarget)}
      >
        <For each={allyTeams()}>
          {(ally) => (
            <option value={String(ally)}>
              {current() === String(ally)
                ? `Team ${ally + 1}`
                : usedAllies().has(ally)
                  ? `Join team ${ally + 1}`
                  : `New team ${ally + 1}`}
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

        {/* Ready is a thing you say to somebody. */}
        <Show when={room.caps.ready}>
          <button
            class={seat()?.ready ? 'primary' : ''}
            disabled={busy()}
            onClick={() =>
              act('ready', () => room.io.setReady(!(seat()?.ready ?? false)))
            }
          >
            {seat()?.ready ? 'Ready' : 'Not ready'}
          </button>
        </Show>

        <select
          value={String(seat()?.side ?? 0)}
          disabled={busy()}
          onChange={(e) =>
            act('faction', () => room.io.setSide(Number(e.currentTarget.value)))
          }
        >
          <For each={SIDES}>
            {(side) => <option value={String(side.id)}>{side.label}</option>}
          </For>
        </select>
        <SideIcon side={seat()?.side ?? 0} />
      </Show>

      <AddAi
        busy={busy()}
        act={act}
        freeTeam={() => nextTeam(room)}
        freeAlly={freeAlly}
        allyTeams={allyTeams}
      />

      {/* One click onto the emptiest side; the picker above is for choosing. */}
      <Show when={!seated()}>
        <button
          disabled={busy()}
          title='Take a seat on the emptiest team'
          onClick={() => act('take a seat', () => sitOn(room, freeAlly()))}
        >
          Join
        </button>
      </Show>

      <span class='spacer' />
      {/* Both halves of Chobby's Host button: an empty autohost is a listed
          room you boss, `!privatehost` is a passworded one made on request.
          Chobby asks for a region; the runtime measures instead, and says
          which room it chose and how far away it is. Both are rooms on the
          server, so neither is offered where there is no server. */}
      <Show when={room.caps.spads}>
        <button
          disabled={busy()}
          onClick={() =>
            act('host a room', async () => {
              // Already standing in a room: the view swaps to the new one as
              // soon as the host lets us in. Said out loud because until then
              // the old room is still on screen and nothing looks to happen.
              await api.hostPublic()
              pushNotice(
                'info',
                'took a room; opening it when the host answers',
              )
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
      </Show>
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
  /** The ally teams this room draws, plus the next one that could be opened. */
  allyTeams: () => number[]
}) {
  const room = useRoom()
  const [ais, setAis] = createSignal<AiChoice[]>([])
  const [ai, setAi] = createSignal('')
  const [open, setOpen] = createSignal(false)
  const [count, setCount] = createSignal(1)
  const [bonus, setBonus] = createSignal(0)
  const [ally, setAlly] = createSignal<number | null>(null)

  createEffect(() => {
    // The room's game, whose Lua AIs are part of the offer.
    const name = room.battle()?.gameName
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

  const chosen = () => ais().find((choice) => choice.name === ai())
  /** The team the sheet is set to, which starts wherever a seat would go. */
  const team = () => ally() ?? props.freeAlly()

  const colour = () =>
    BOT_COLOURS[Math.floor(Math.random() * BOT_COLOURS.length)] as number

  async function add() {
    const claimed = new Set<string>()
    const wanted = bonus()
    for (let n = 0; n < count(); n++) {
      const name = unusedBotName(room.battle(), ai(), claimed)
      claimed.add(name)
      const seat = props.freeTeam() + n
      await room.io.addBot(name, ai(), seat, team(), colour())
      // `ADDBOT` carries no bonus, so one is a second message -- which the
      // server takes from us because the AI we just added is ours.
      if (wanted > 0)
        await room.io.updateBot(name, seat, team(), wanted, colour())
    }
  }

  return (
    <Show when={ais().length > 0}>
      <button
        disabled={props.busy}
        title='The AI plays from this machine'
        onClick={() => setOpen(true)}
      >
        Add AI
      </button>

      <Show when={open()}>
        <div class='sheet' onMouseDown={() => setOpen(false)}>
          <form
            class='sheet-card'
            onMouseDown={(event) => event.stopPropagation()}
            onSubmit={(event) => {
              event.preventDefault()
              setOpen(false)
              void props.act('add an AI', add)
            }}
          >
            <h2>Add AI</h2>
            <label>
              Which
              <select
                value={ai()}
                onChange={(e) => setAi(e.currentTarget.value)}
              >
                <For each={ais()}>
                  {(choice) => (
                    <option value={choice.name}>{choice.name}</option>
                  )}
                </For>
              </select>
            </label>
            <Show when={chosen()?.desc}>
              <p class='muted'>{chosen()?.desc}</p>
            </Show>
            <label>
              Team
              <select
                value={String(team())}
                onChange={(e) => setAlly(Number(e.currentTarget.value))}
              >
                <For each={props.allyTeams()}>
                  {(one) => <option value={String(one)}>Team {one + 1}</option>}
                </For>
              </select>
            </label>
            <label>
              How many
              <input
                type='number'
                min={1}
                max={16}
                value={count()}
                onInput={(e) => {
                  const many = Number(e.currentTarget.value)
                  if (Number.isFinite(many))
                    setCount(Math.max(1, Math.min(16, Math.round(many))))
                }}
              />
            </label>
            <label>
              Bonus
              <input
                type='range'
                min={0}
                max={100}
                step={5}
                value={bonus()}
                onInput={(e) => setBonus(Number(e.currentTarget.value))}
              />
              <output>{bonus()}%</output>
            </label>
            <p class='muted'>
              A resource bonus, the same one a host gives with{' '}
              <code>!force … bonus</code>. Every AI added here runs on this
              machine when the game starts.
            </p>
            <div class='sheet-actions'>
              <button type='button' onClick={() => setOpen(false)}>
                Cancel
              </button>
              <button type='submit' class='primary' disabled={!ai()}>
                Add
              </button>
            </div>
          </form>
        </div>
      </Show>
    </Show>
  )
}
