/**
 * Moving one item of a list to where it was dropped.
 *
 * Pure, because drag-and-drop is the classic place for an off-by-one that only
 * shows up when you drag rightwards: removing the item first shifts every
 * later index down by one, so the naive "remove then insert at `to`" lands one
 * short. That is a rule, not a feel, so it is tested rather than fiddled with.
 */
export function move<T>(items: readonly T[], from: number, to: number): T[] {
  if (
    from === to ||
    from < 0 ||
    to < 0 ||
    from >= items.length ||
    to >= items.length
  ) {
    return [...items]
  }
  const next = [...items]
  const [held] = next.splice(from, 1)
  // `splice` has already closed the gap, so `to` addresses the post-removal
  // list and needs no adjustment — which is exactly the part that looks wrong.
  next.splice(to, 0, held as T)
  return next
}

/**
 * A saved order applied to whatever is actually open.
 *
 * The two drift constantly: a channel is left, someone messages you for the
 * first time, a saved order names a channel from last week. So the saved order
 * is a preference rather than a truth — anything it names that is not open is
 * dropped, and anything open it does not name goes to the end, in the order it
 * arrived.
 */
export function ordered<T>(present: readonly T[], saved: readonly T[]): T[] {
  const open = new Set(present)
  const known = new Set(saved)
  return [
    ...saved.filter((item) => open.has(item)),
    ...present.filter((item) => !known.has(item)),
  ]
}
