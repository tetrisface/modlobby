import { freeTeam } from '../../lib/roster'
import type { RoomModel } from './model'

/**
 * Moving somebody to a team, and giving an AI a bonus.
 *
 * One gesture, three mechanisms -- which is Chobby's arrangement too
 * (`api_user_handler.lua:1573`): your own seat is a battle status of your own,
 * an AI you added is an `UPDATEBOT`, and everybody else is a request to the
 * host in battle chat. Only the last of the three can be refused, and it is
 * refused often; see [`FORCE_REFUSED`].
 */

/** Who is being moved. */
export type Target =
  | { kind: 'me' }
  | { kind: 'player'; name: string }
  | {
      kind: 'bot'
      name: string
      /** Whether we added it. Anyone else's, the server ignores us for. */
      mine: boolean
      /** Kept as it is when only the team changes, and vice versa. */
      team: number
      handicap: number
      colour: number
    }

/**
 * Whether the room arranges its own teams.
 *
 * While it does, SPADS declines to move anybody by hand -- and BAR's default
 * team preset ships `autoBalance` on, so this is the ordinary case rather than
 * an edge one (`spads.pl:8886`).
 *
 * A room that has not said counts as not balancing, and the move is attempted.
 * A BAR host broadcasts this to the whole room on every change and again as
 * each person joins, so silence means a host that is not BAR's -- rare, and not
 * worth refusing over. Letting SPADS answer for itself costs one line of battle
 * chat and is right whenever it is on.
 */
export function balancing(room: RoomModel): boolean {
  const mode = room.my()?.autoBalance
  return mode !== null && mode !== undefined && mode !== 'off'
}

/** A target with a name to put in a command: everybody except yourself. */
type Named = Exclude<Target, { kind: 'me' }>

/** Whether SPADS would take our word for it in this room. */
export function bossing(room: RoomModel): boolean {
  // A room with no host to ask is one we already run.
  if (!room.caps.spads) return true
  const boss = room.my()?.boss
  return boss !== undefined && boss !== null && boss === room.me()
}

/** Whether this row can be moved at all, and so whether it can be dragged. */
export function movable(room: RoomModel, target: Target): boolean {
  if (target.kind === 'me') return true
  if (target.kind === 'bot' && target.mine) return true
  return bossing(room)
}

/**
 * `!force <name> team <n>`.
 *
 * SPADS counts teams from one and takes the one back off before it sends
 * `FORCEALLYNO` (`spads.pl:8996`), so the number here is one more than the
 * index everything else in this app uses. A bot's name is prefixed with `%`
 * so it is not searched for among the players (`spads.pl:8917`).
 */
export function forceTeam(target: Named, allyTeam: number): string {
  const who = target.kind === 'bot' ? `%${target.name}` : target.name
  return `!force ${who} team ${allyTeam + 1}`
}

/** `!force <name> bonus <percent>`; SPADS refuses anything over 100. */
export function forceBonus(target: Named, percent: number): string {
  const who = target.kind === 'bot' ? `%${target.name}` : target.name
  return `!force ${who} bonus ${Math.max(0, Math.min(100, Math.round(percent)))}`
}

/** The lowest engine team nobody holds, which is what a new seat needs. */
function ourTeam(room: RoomModel): number {
  const battle = room.battle()
  return battle ? freeTeam(battle, room.users(), room.me()) : 0
}

export async function moveTo(
  room: RoomModel,
  target: Target,
  allyTeam: number,
): Promise<void> {
  if (target.kind === 'me') {
    await room.io.takeSeat(ourTeam(room), allyTeam)
    return
  }
  if (target.kind === 'bot' && target.mine) {
    // The message replaces the whole status, so everything not being changed
    // has to be sent again -- the bonus included, which is the bit Chobby
    // loses and patches up with a second command.
    await room.io.updateBot(
      target.name,
      target.team,
      allyTeam,
      target.handicap,
      target.colour,
    )
    return
  }
  if (balancing(room)) {
    // Sending it anyway means the answer arrives in battle chat, where a
    // person mid-drag is not looking. Better to say it here, in the words
    // that name the fix.
    throw new Error(
      'the room is balancing its own teams; turn Auto balance off first',
    )
  }
  await room.io.sayBattle(forceTeam(target, allyTeam))
}

/**
 * A resource bonus, as a percentage.
 *
 * Not for your own seat: a bonus is something a host gives, and asking for one
 * for yourself is a request like any other -- by name, through the host.
 */
export async function setBonus(
  room: RoomModel,
  target: Named,
  percent: number,
  allyTeam: number,
): Promise<void> {
  const wanted = Math.max(0, Math.min(100, Math.round(percent)))
  if (target.kind === 'bot' && target.mine) {
    await room.io.updateBot(
      target.name,
      target.team,
      allyTeam,
      wanted,
      target.colour,
    )
    return
  }
  await room.io.sayBattle(forceBonus(target, wanted))
}
