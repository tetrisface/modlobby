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
  spectators: UserView[]
  /** Unplaced members with no seat to guess for them; listed as spectators. */
  pending: UserView[]
  /**
   * The spectator count to show: the list's own while members are still
   * being placed, since most of the unplaced are players on their way.
   */
  spectatorCount: number
}

/** Teams to draw while the room is still arriving, when the server said none. */
const DEFAULT_TEAMS = 2

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

  const spectators: UserView[] = []
  const pending: UserView[] = []
  // `members` arrives sorted by name; keeping that order means a skill tag
  // landing late never reshuffles the list under the reader's eyes.
  for (const name of room.members) {
    const user = users[name]
    if (!user) continue
    if (user.battleStatus?.player)
      team(user.battleStatus.allyTeam).users.push(user)
    else if (user.battleStatus) spectators.push(user)
    // An autohost only ever spectates its own room, status or no status.
    else if (user.status.bot) spectators.push(user)
    else pending.push(user)
  }
  for (const bot of room.bots) team(bot.status.allyTeam).bots.push(bot)

  if (pending.length === 0) {
    for (const t of teams.values()) t.expected = t.users.length + t.bots.length
    return {
      teams: sorted(teams),
      spectators,
      pending,
      spectatorCount: spectators.length,
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
  const queue = pending.filter((user) => user.name !== me)
  const round = sorted(teams)
  for (let i = 0; toSeat > 0 && i < queue.length;) {
    let seated = false
    for (const t of round) {
      if (toSeat === 0 || i >= queue.length) break
      if (emptySeats(t) === 0) continue
      const user = queue[i]!
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
    spectators,
    pending: pending.filter((user) => !dealt.has(user.name)),
    spectatorCount: Math.max(room.spectatorCount, spectators.length),
  }
}

function sorted(teams: Map<number, Team>): Team[] {
  return [...teams.values()].sort((a, b) => a.allyTeam - b.allyTeam)
}

/** Seats a team shows empty, while its players are still on their way. */
export function emptySeats(team: Team): number {
  return Math.max(
    0,
    team.expected - team.users.length - team.bots.length - team.guessed.length,
  )
}
