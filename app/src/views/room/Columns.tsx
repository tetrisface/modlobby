import { onCleanup, onMount } from 'solid-js'

/**
 * How the teams share their row.
 *
 * A team reads one name a row, a card wide, and stays that way while the
 * roster has the height for it. A team too deep for that flows its rows into
 * columns and is that many cards wide. How many is counted from the room
 * there is -- the row's width and the roster's cap -- and never from where
 * anything landed, so the answer cannot depend on itself.
 */

/** What the teams have to stand in. */
export type Space = {
	/** Cards the row holds side by side. */
	across: number
	/** Rows a team holds before the roster would scroll. */
	deep: number
}

/** A row nobody has measured: nothing stands beside anything, nothing flows. */
export const UNMEASURED: Space = { across: 0, deep: Infinity }

/** How many cards stand side by side on a row `row` wide. */
export function cardsAcross(card: number, gap: number, row: number): number {
	if (card <= 0 || row <= 0) return 0
	return Math.floor((row + gap) / (card + gap))
}

/**
 * How many columns each team's rows flow into, in the teams' order.
 *
 * One each while the deepest team fits. Past that, the deepest takes the
 * fewest columns that keep it from scrolling and every team flows at that
 * depth, so the sides of one game are drawn alike whatever their sizes. The
 * teams stay on one row: where it cannot hold that many columns the depth
 * grows until it can, and where nothing shallower than a column each fits,
 * a column each it is.
 */
export function teamColumns(sizes: readonly number[], space: Space): number[] {
	const single = sizes.map(() => 1)
	const deepest = Math.max(0, ...sizes)
	if (deepest <= space.deep) return single
	const fewest = Math.ceil(deepest / Math.max(1, space.deep))
	for (let depth = Math.ceil(deepest / fewest); depth < deepest; depth++) {
		const columns = sizes.map((size) => Math.max(1, Math.ceil(size / depth)))
		if (columns.reduce((sum, n) => sum + n, 0) <= space.across) return columns
	}
	return single
}

/**
 * Reads the space off the page. `ruler` is drawn by the stylesheet one card
 * wide and as tall as the roster may stand, so both lengths stay the
 * stylesheet's; a team's header and one of its rows say what a row costs.
 */
export function measureSpace(row: HTMLElement, ruler: HTMLElement): Space {
	const pane = row.parentElement
	if (!pane) return UNMEASURED
	const px = (length: string) => parseFloat(length) || 0
	const ruled = ruler.getBoundingClientRect()
	const across = cardsAcross(
		ruled.width,
		px(getComputedStyle(row).columnGap),
		row.getBoundingClientRect().width,
	)
	// No row drawn yet is nothing to flow, however short the roster.
	const first = row.querySelector(':scope > .team > .rows > *')
	const rows = first?.parentElement
	const team = rows?.parentElement
	if (!first || !rows || !team) return { across, deep: Infinity }
	const padding = getComputedStyle(pane)
	const head =
		rows.getBoundingClientRect().top - team.getBoundingClientRect().top
	const height =
		ruled.height - px(padding.paddingTop) - px(padding.paddingBottom) - head
	return {
		across,
		deep: Math.floor(height / first.getBoundingClientRect().height),
	}
}

/**
 * The ruler, and the watch on it: says what space the teams have whenever
 * their row or the roster's cap changes size. Drawn inside the box the cap
 * is a share of, which is what makes its height the cap.
 */
export function RosterRuler(props: {
	row: () => HTMLElement | undefined
	onChange: (space: Space) => void
}) {
	let ruler: HTMLDivElement | undefined
	onMount(() => {
		const row = props.row()
		if (!ruler || !row || typeof ResizeObserver === 'undefined') return
		const rule = ruler
		const watching = new ResizeObserver(() =>
			props.onChange(measureSpace(row, rule)),
		)
		watching.observe(row)
		watching.observe(rule)
		onCleanup(() => watching.disconnect())
	})
	return <div class='roster-ruler' ref={ruler} />
}
