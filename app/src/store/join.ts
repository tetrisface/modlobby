/**
 * Milestones of one room join, to read beside the runtime's `join:` lines.
 *
 * A join is a handshake through the server and the host bot, then a burst
 * of room state, then a render; which of the three is slow can only be told
 * from a live join, so each step logs its time since the click.
 */

let askedAt: number | null = null
const logged = new Set<string>()

/** The moment the room was asked for. */
export function markJoinAsked(): void {
  askedAt = performance.now()
  logged.clear()
}

/** Logs `label` once per join, with the time since asking. */
export function joinMilestone(label: string): void {
  if (askedAt === null || logged.has(label)) return
  logged.add(label)
  console.info(
    `join: ${label} at ${Math.round(performance.now() - askedAt)} ms`,
  )
}
