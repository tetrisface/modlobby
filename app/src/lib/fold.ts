/**
 * How many of a row's items fit in it, and so which fold into a menu.
 *
 * Kept apart from the nav so the arithmetic can be tested without a layout
 * engine: the test runner's DOM has no widths, and the one interesting case
 * -- the button that stands in for the folded items takes room of its own,
 * so hiding a short item can cost more than it frees -- is exactly the kind
 * a quick try in the app does not cover.
 */

/**
 * The most items, most important first, that fit in `room`.
 *
 * `needs` is each item's width in the order they are kept, `gap` the space
 * between neighbours, and `menu` the width of the button that stands in for
 * whatever is folded -- which takes room only once something is.
 */
export function fitCount(
  needs: readonly number[],
  room: number,
  gap: number,
  menu: number,
): number {
  for (let kept = needs.length; kept > 0; kept--) {
    const folded = kept < needs.length
    if (width(needs.slice(0, kept), folded, gap, menu) <= room) return kept
  }
  return 0
}

function width(
  needs: readonly number[],
  folded: boolean,
  gap: number,
  menu: number,
): number {
  const items = needs.length + (folded ? 1 : 0)
  const sum = needs.reduce((total, need) => total + need, 0)
  return sum + (folded ? menu : 0) + gap * Math.max(0, items - 1)
}
