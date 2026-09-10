import type { BattleView } from '../ipc/bindings/BattleView'
import type { BotView } from '../ipc/bindings/BotView'
import type { UserView } from '../ipc/bindings/UserView'

/**
 * The room's roster, drawn before the server has said who sits where.
 *
 * Joining a room is a handshake through the host, then a burst of one
 * `CLIENTBATTLESTATUS` per member that can take a second to trickle in.
 * The battle list already knows the members, the room's shape
 * (`s.battle.teams`) and how many of them play, so the teams are laid out
 * from that and the names are seated provisionally, to be corrected as the
 * statuses arrive. The page keeps its shape; only names move.
 */

export type Team = {
  allyTeam: number
  /** Placed by the server. */
  users: UserView[]
  bots: BotView[]
  /** Seated here as a guess, until the server says where they really sit. */
  guessed: UserView[]
  /** Seats to draw, filled or not: at least the ones taken. */
  expected: number
}

export type Roster = {
  teams: Team[]
  /** Spectators waiting for a seat, first in line first. */
  queue: UserView[]
  /** Everyone else watching: the host first, then by name. */
  spectators: UserView[]
  /** Unplaced members with no seat to guess for them; listed as spectators. */
  pending: UserView[]
  /**
   * The spectator count to show: the list's own while members are still
   * being placed, since most of the unplaced are players on their way.
   */
  spectatorCount: number
}

/**
 * The fewest teams ever drawn. Two, so that a room of one side still has
 * somewhere to drag a player or an AI to; and what an arriving room shows
 * while the server has said nothing about its shape.
 */
export const DEFAULT_TEAMS = 2

export function arrange(
  room: BattleView,
  users: Record<string, UserView>,
  me: string | null = null,
): Roster {
  const teams = new Map<number, Team>()
  const team = (allyTeam: number) => {
    const found = teams.get(allyTeam) ?? {
      allyTeam,
      users: [],
      bots: [],
      guessed: [],
      expected: 0,
    }
    teams.set(allyTeam, found)
    return found
  }

  const place = new Map(room.queue.map((name, index) => [name, index]))
  const queue: UserView[] = []
  const spectators: UserView[] = []
  const pending: UserView[] = []
  for (const name of room.members) {
    const user = users[name]
    if (!user) continue
    if (user.battleStatus?.player)
      team(user.battleStatus.allyTeam).users.push(user)
    // The server's word on who waits, whether or not a status has arrived.
    else if (place.has(name)) queue.push(user)
    else if (user.battleStatus) spectators.push(user)
    // An autohost only ever spectates its own room, status or no status.
    else if (user.status.bot) spectators.push(user)
    else pending.push(user)
  }
  for (const bot of room.bots) team(bot.status.allyTeam).bots.push(bot)
  for (let allyTeam = 0; allyTeam < DEFAULT_TEAMS; allyTeam++) team(allyTeam)

  // Ordered by nothing a late status could change, so a skill tag landing
  // late never reshuffles the list under the reader's eyes.
  queue.sort((a, b) => place.get(a.name)! - place.get(b.name)!)
  spectators.sort(watching(room.founder))
  pending.sort(watching(room.founder))
  const watchers = queue.length + spectators.length

  if (pending.length === 0) {
    for (const t of teams.values()) t.expected = t.users.length + t.bots.length
    return {
      teams: sorted(teams),
      queue,
      spectators,
      pending,
      spectatorCount: watchers,
    }
  }

  const shape = room.layout
  const count = shape?.teams ?? Math.max(teams.size, DEFAULT_TEAMS)
  for (let allyTeam = 0; allyTeam < count; allyTeam++) team(allyTeam)
  const perTeam = shape?.teamSize ?? Math.ceil(room.playerCount / count)
  for (const t of teams.values())
    t.expected = Math.max(t.users.length + t.bots.length, perTeam)

  // The list said how many play. Those seats get the unplaced names, dealt
  // round the teams in turn; whoever is left over is most likely watching.
  // We arrive as a spectator ourselves, so our own name is never dealt.
  const placed = [...teams.values()].reduce(
    (n, t) => n + t.users.length + t.bots.length,
    0,
  )
  let toSeat = Math.max(0, room.playerCount - placed)
  const dealt = new Set<string>()
  const unplaced = pending.filter((user) => user.name !== me)
  const round = sorted(teams)
  for (let i = 0; toSeat > 0 && i < unplaced.length;) {
    let seated = false
    for (const t of round) {
      if (toSeat === 0 || i >= unplaced.length) break
      if (emptySeats(t) === 0) continue
      const user = unplaced[i]!
      t.guessed.push(user)
      dealt.add(user.name)
      i += 1
      toSeat -= 1
      seated = true
    }
    if (!seated) break
  }

  return {
    teams: round,
    queue,
    spectators,
    pending: pending.filter((user) => !dealt.has(user.name)),
    spectatorCount: Math.max(room.spectatorCount, watchers),
  }
}

function sorted(teams: Map<number, Team>): Team[] {
  return [...teams.values()].sort((a, b) => a.allyTeam - b.allyTeam)
}

/**
 * Chobby's spectator order: the host on top, then by name. `localeCompare`
 * folds case, as the chat roster's sort does, so `Zed` does not lead `alice`
 * the way the wire's byte order has it.
 */
function watching(founder: string) {
  return (a: UserView, b: UserView): number =>
    Number(b.name === founder) - Number(a.name === founder) ||
    a.name.localeCompare(b.name)
}

/**
 * The lowest team number nobody else holds, so two players never collide.
 * Our own current team is not counted: moving seats is not a collision.
 */
export function freeTeam(
  room: BattleView,
  users: Record<string, UserView>,
  me: string | null,
): number {
  const taken = new Set<number>()
  for (const name of room.members) {
    const status = users[name]?.battleStatus
    if (status?.player && name !== me) taken.add(status.team)
  }
  for (const bot of room.bots) taken.add(bot.status.team)
  let team = 0
  while (taken.has(team)) team += 1
  return team
}

/** Seats a team shows empty, while its players are still on their way. */
/**
 * `BARb`, then `BARb2` -- a name for an AI that the room does not already hold.
 *
 * `claimed` carries names taken earlier in the same batch: the room's own list
 * does not catch up between the messages of a single click.
 */
export function unusedBotName(
  battle: BattleView | undefined,
  base: string,
  claimed: ReadonlySet<string> = new Set(),
): string {
  const taken = new Set((battle?.bots ?? []).map((bot) => bot.name))
  for (const name of claimed) taken.add(name)
  if (!taken.has(base)) return base
  let n = 2
  while (taken.has(`${base}${n}`)) n += 1
  return `${base}${n}`
}

export function emptySeats(team: Team): number {
  return Math.max(
    0,
    team.expected - team.users.length - team.bots.length - team.guessed.length,
  )
}
