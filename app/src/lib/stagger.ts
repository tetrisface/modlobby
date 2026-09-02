import type { UserView } from '../ipc/bindings/UserView'

/**
 * Who in a room asks the PvE Stats service first, and how long the rest wait.
 *
 * Every client in a room sees the same change at the same moment, and the
 * service answers one request at a time: asked all at once, one client gets
 * a number and the rest get refusals to retry. Asked in a fixed order a few
 * hundred milliseconds apart, the first takes any cold start and the rest
 * find the service warm and free. The order is one everyone can compute
 * from what the lobby already tells them, so nobody has to agree on it.
 */

/** How far apart consecutive clients ask. */
export const STAGGER_STEP = 400
/** The most anyone waits, however big the room. */
export const STAGGER_CAP = 4000

export type Room = {
  me: string | null
  /** The room's members in the lobby's own order. */
  members: string[]
  boss: string | null
  users: Record<string, UserView>
}

/**
 * Members in asking order: the boss, then players before spectators, then
 * higher lobby rank first, then the lobby's own order. Bots — the host
 * among them — do not ask, so they do not hold a place.
 */
export function askOrder(
  room: Pick<Room, 'members' | 'boss' | 'users'>,
): string[] {
  type Key = [boss: number, spectator: number, rank: number]
  const key = (name: string): Key => {
    const user = room.users[name]
    return [
      name === room.boss ? 0 : 1,
      user?.battleStatus?.player ? 0 : 1,
      -(user?.status.rank ?? 0),
    ]
  }
  return room.members
    .filter((name) => !room.users[name]?.status.bot)
    .map((name, index) => ({ name, index, key: key(name) }))
    .sort(
      (a, b) =>
        a.key[0] - b.key[0] ||
        a.key[1] - b.key[1] ||
        a.key[2] - b.key[2] ||
        a.index - b.index,
    )
    .map((entry) => entry.name)
}

/** Milliseconds this client waits before asking, from its place in the room. */
export function askDelay(room: Room): number {
  if (room.me === null) return 0
  const place = askOrder(room).indexOf(room.me)
  if (place < 0) return 0
  return Math.min(place * STAGGER_STEP, STAGGER_CAP)
}
