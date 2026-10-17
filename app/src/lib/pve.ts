import type { Player } from '../ipc/bindings/Player'
import type { Score } from '../ipc/bindings/Score'
import { age } from './age'

/**
 * What the pve.bar widget draws from a stats answer, as plain functions.
 *
 * The in-game widget (`gui_pve_stats`) is the reference: its tabs, columns,
 * sort cascades and help texts are kept so a player who knows one reads the
 * other. Everything here is pure so the panel stays a thin drawing of it.
 */

/** The tabs over each person's record, in the widget's order. */
export const TABS = [
	{ key: 'achievements', label: 'Setting achievements' },
	{ key: 'awards', label: 'Awards' },
	{ key: 'encounters', label: 'Encounters' },
	{ key: 'adventures', label: 'Games & maps' },
] as const
export type Tab = (typeof TABS)[number]['key']

export type Column = {
	label: string
	help: string
	value: (player: Player) => number | null
	/** Scoped to the served setting rather than a lifetime total. */
	setup?: boolean
}

/** A table row: a name the service knows, or one it has never seen play. */
export type Row = { name: string; player: Player | null }

/** Which of the three opponent columns is the room's. */
function opponent(ai: string): 0 | 1 | 2 {
	const folded = ai.toLowerCase()
	if (folded === 'scavengers') return 1
	if (folded === 'barbarian') return 2
	return 0
}

/** The served setting's enemies, named the way the mode's players say them. */
export function enemyNoun(ai: string, count: number): string {
	const one = count === 1
	switch (opponent(ai)) {
		case 1:
			return one ? 'boss' : 'bosses'
		case 2:
			return one ? 'Barbarian AI' : 'Barbarian AIs'
		default:
			return one ? 'queen' : 'queens'
	}
}

const SETUP_HELP: Record<string, [string, string]> = {
	exact: [
		'Eligible victories on this exact lobby setup, at this team size and AI count. Counts only curated eligible games, so it sits below the lifetime totals on Encounters.',
		"The highest enemy count this player has cleared among this setup's enemy-count versions. A player who has cleared both a 20-%s and a 50-%s version shows 50.",
	],
	similar: [
		'Eligible victories on a SIMILAR setting matched by effect vector, not your exact lobby, at this team size and AI count. Counts only curated eligible games.',
		"The highest enemy count this player has cleared among the SIMILAR matched setting's enemy-count versions, not your exact lobby. A player who has cleared both a 20-%s and a 50-%s version shows 50.",
	],
	raw_fallback: [
		'Eligible victories on the CLOSEST RAW setting match, not your exact lobby, at this team size and AI count. Counts only curated eligible games.',
		"The highest enemy count this player has cleared among the CLOSEST RAW matched setting's enemy-count versions, not your exact lobby. A player who has cleared both a 20-%s and a 50-%s version shows 50.",
	],
}

/** The columns a tab shows, with help worded for what the service served. */
export function columns(tab: Tab, score: Score): Column[] {
	const ai = score.aiType
	switch (tab) {
		case 'achievements': {
			const source = score.setup?.source ?? 'exact'
			const [clears, max] = SETUP_HELP[source] ?? SETUP_HELP.exact!
			const count = score.setup?.encounterCount ?? 0
			const fields =
				count > 0
					? ` ${source === 'exact' ? 'This setup' : 'The matched setting'} fields ${count} ${enemyNoun(ai, count)} per game.`
					: ''
			const noun = enemyNoun(ai, 1)
			return [
				{
					label: '20+ clears',
					help: 'Eligible wins at governed challenge 20 or above: modeled population win chance at most 41.2%.',
					value: (p) => p.accomplishments.challenges.challenge20Clears,
				},
				{
					label: '25+ clears',
					help: 'Eligible wins at governed challenge 25 or above: modeled population win chance at most 26.5%.',
					value: (p) => p.accomplishments.challenges.challenge25Clears,
				},
				{
					label: '30+ clears',
					help: 'Eligible wins at governed challenge 30 or above: modeled population win chance at most 11.8%.',
					value: (p) => p.accomplishments.challenges.challenge30Clears,
				},
				{
					label: 'Setup clears',
					help: clears + fields,
					value: (p) => p.setupExperience.clears,
					setup: true,
				},
				{
					label: 'Max here',
					help: max.replaceAll('%s', noun),
					value: (p) => p.setupExperience.maxDefeated,
					setup: true,
				},
			]
		}
		case 'awards':
			return [
				{
					label: 'Raptor most killed',
					help: 'Times this player earned Most Killed against Raptors by ranking first in fighting-unit value destroyed.',
					value: (p) => p.awards.mostKilled.raptors,
				},
				{
					label: 'Scav most killed',
					help: 'Times this player earned Most Killed against Scavengers by ranking first in fighting-unit value destroyed.',
					value: (p) => p.awards.mostKilled.scavengers,
				},
				{
					label: 'BARb most killed',
					help: 'Times this player earned Most Killed against BARbarians by ranking first in fighting-unit value destroyed.',
					value: (p) => p.awards.mostKilled.barbarians,
				},
			]
		case 'encounters':
			return [
				{
					label: 'Queens defeated',
					help: 'Total Raptor queens defeated in victories.',
					value: (p) => p.accomplishments.encounters.raptorQueensDefeated,
				},
				{
					label: 'Bosses defeated',
					help: 'Total Scavenger bosses defeated in victories.',
					value: (p) => p.accomplishments.encounters.scavengerBossesDefeated,
				},
				{
					label: 'BARbarians defeated',
					help: 'Total Barbarian AI opponents defeated in victories.',
					value: (p) => p.accomplishments.encounters.barbarianAisDefeated,
				},
				{
					label: 'Max queens',
					help: 'Most Raptor queens defeated in a single victory.',
					value: (p) => p.accomplishments.personalBests.maxQueensOneVictory,
				},
				{
					label: 'Max bosses',
					help: 'Most Scavenger bosses defeated in a single victory.',
					value: (p) => p.accomplishments.personalBests.maxBossesOneVictory,
				},
				{
					label: 'Max BARbarians',
					help: 'Most Barbarian AI opponents defeated in a single victory.',
					value: (p) =>
						p.accomplishments.personalBests.maxBarbarianAisOneVictory,
				},
			]
		case 'adventures':
			return [
				{
					label: 'Victories',
					help: 'All curated PvE victories in this mode.',
					value: (p) => p.accomplishments.participation.victories,
				},
				{
					label: 'Games',
					help: 'All curated PvE games played in this mode.',
					value: (p) => p.accomplishments.participation.gamesPlayed,
				},
				{
					label: 'Maps',
					help: 'Distinct maps played in this mode.',
					value: (p) => p.accomplishments.participation.distinctMapsPlayed,
				},
			]
	}
}

/** The column a tab sorts by until someone clicks another, 0-based. */
export function defaultSort(tab: Tab, ai: string): number {
	switch (tab) {
		case 'achievements':
			return 4
		case 'awards':
			return opponent(ai)
		// The current mode's Max leads: totals reward grinding, the max is the feat.
		case 'encounters':
			return opponent(ai) + 3
		case 'adventures':
			return 0
	}
}

/**
 * Where a tie on the sorted column goes next. The widget's reasoning, kept:
 * cross-mode ties fall to how rare each mode's ceiling is (bosses, queens,
 * BARbs), awards to mode popularity (BARb, Raptors, Scavengers).
 */
export function cascade(tab: Tab, ai: string): number[] {
	const current = opponent(ai)
	switch (tab) {
		case 'achievements':
			return [4, 3, 2, 1, 0]
		case 'awards':
			return [current, ...[2, 0, 1].filter((c) => c !== current)]
		case 'encounters': {
			const max = current + 3
			return [
				max,
				current,
				...[4, 3, 5].filter((c) => c !== max),
				...[1, 0, 2].filter((c) => c !== current),
			]
		}
		case 'adventures':
			return [0, 1, 2]
	}
}

/** Sorted by a column; 'name' sorts by name. */
export const BY_NAME = 'name'

function valueOf(row: Row, column: Column): number | null {
	return row.player ? column.value(row.player) : null
}

/**
 * One column's verdict, or `undefined` on a tie. A missing value sorts after
 * every present one whichever way the column runs.
 */
function order(
	a: Row,
	b: Row,
	column: Column,
	descending: boolean,
): number | undefined {
	const left = valueOf(a, column)
	const right = valueOf(b, column)
	if (left == null && right == null) return undefined
	if (left == null) return 1
	if (right == null) return -1
	if (left === right) return undefined
	return descending ? right - left : left - right
}

function byName(a: Row, b: Row): number {
	return a.name.localeCompare(b.name, undefined, { sensitivity: 'base' })
}

export function sortRows(
	rows: Row[],
	cols: Column[],
	by: number | typeof BY_NAME,
	descending: boolean,
	fallbacks: number[],
): Row[] {
	return [...rows].sort((a, b) => {
		if (by === BY_NAME) return descending ? byName(b, a) : byName(a, b)
		const first = cols[by]
		const verdict = first && order(a, b, first, descending)
		if (verdict !== undefined) return verdict
		// Ties fall through the tab's cascade before names; the clicked column
		// stays primary and the cascade only decides what it left undecided.
		for (const next of fallbacks) {
			const column = cols[next]
			if (next === by || !column) continue
			const later = order(a, b, column, descending)
			if (later !== undefined) return later
		}
		return byName(a, b)
	})
}

/** Zeros and absent values both blank, so the nonzero results carry the table. */
export function cell(value: number | null): string {
	return value ? String(Math.round(value)) : ''
}

/** Everyone the service answered about, then the names it did not know. */
export function rows(score: Score): Row[] {
	const known = score.players.map((player) => ({
		name: player.playerName,
		player,
	}))
	const seen = new Set(known.map((row) => row.name.toLowerCase()))
	const unknown = score.unresolvedPlayers
		.filter((name) => !seen.has(name.toLowerCase()))
		.map((name) => ({ name, player: null }))
	return [...known, ...unknown]
}

export function ownPlayer(score: Score, me: string | null): Player | undefined {
	const folded = me?.toLowerCase()
	return score.players.find((p) => p.playerName.toLowerCase() === folded)
}

/** The scale the challenge runs on. */
export const TOP = 34

/** Labels along the challenge axis: every five, and the top. */
export const TICKS = [0, 5, 10, 15, 20, 25, 30, TOP]

/**
 * The score row's icon: the games histogram in little, and where this setup
 * sits. Bars run across in challenge units (0-34) and up from 0 to 1; `at`
 * is the current challenge, `null` while unplaced.
 */
export type Miniature = {
	bars: { lo: number; hi: number; height: number }[]
	at: number | null
}

const onScale = (value: number) => Math.min(Math.max(value, 0), TOP)

/**
 * The icon before any answer: nine bars at zero, the count the service's
 * seventeen bins make, and no tick, as nothing is placed yet.
 */
export const NOTHING_YET: Miniature = {
	bars: Array.from({ length: 9 }, (_, j) => ({
		lo: (j * TOP) / 9,
		hi: ((j + 1) * TOP) / 9,
		height: 0,
	})),
	at: null,
}

export function miniature(score: Score): Miniature {
	const bins = score.histogram.map((bin) => {
		const lo = onScale(bin.lowerBound)
		return { lo, hi: Math.max(onScale(bin.upperBound), lo), games: bin.games }
	})
	// Half as many bars as bins, so at icon size each stays a few pixels wide
	// instead of a comb of hairlines, and all of them equally wide: each bin's
	// games go to the bars it overlaps, in proportion to the overlap, taking a
	// bin's games as spread evenly across its range. Pairing bins instead
	// would leave an odd last one alone at half width.
	const count = Math.ceil(bins.length / 2)
	const step = TOP / Math.max(count, 1)
	const games = Array.from({ length: count }, (_, j) => {
		const lo = j * step
		const hi = lo + step
		return bins.reduce((sum, bin) => {
			const overlap = Math.min(hi, bin.hi) - Math.max(lo, bin.lo)
			return overlap > 0 && bin.hi > bin.lo
				? sum + (bin.games * overlap) / (bin.hi - bin.lo)
				: sum
		}, 0)
	})
	const top = Math.max(0, ...games)
	return {
		bars: games.map((held, j) => ({
			lo: j * step,
			hi: (j + 1) * step,
			height: top > 0 ? held / top : 0,
		})),
		at: score.challenge == null ? null : onScale(score.challenge),
	}
}

/**
 * What the bars' heights measure: how many games sit at each challenge, or how
 * often humans won them.
 */
export type Measure = 'count' | 'winrate'

/**
 * Games at which a bin's win rate is drawn at full strength. At 400 games the
 * standard error of a rate near one half is about 2.5 points; below that the
 * bar fades, so a rate resting on a handful of games does not look as sure as
 * one resting on thousands.
 */
export const SURE_AT = 400

/**
 * One histogram bar: where it sits on the 0-34 axis and how wide, both as
 * fractions, the middle of its range as a whole challenge score, its heights
 * (as a share of the tallest for counts, as the rate itself for win rates),
 * how strongly to draw it, and its tooltip, one fact a line.
 *
 * `popHeight` and `ownHeight` are what is drawn, in percent of the chart,
 * with the floors that keep a non-zero bar visible; `popValue` and
 * `ownValue` are what those heights stand for (a share of games or of your
 * clears, or a win rate), `null` where there is no bar.
 */
export type Bar = {
	left: number
	width: number
	mid: number
	share: number
	own: number
	popHeight: number
	ownHeight: number
	popValue: number
	ownValue: number | null
	sure: number
	current: boolean
	help: string
}

function holds(lower: number, upper: number, at: number): boolean {
	return at >= lower && (at < upper || (upper >= TOP && at <= TOP))
}

export function bars(
	score: Score,
	me: Player | undefined,
	measure: Measure = 'count',
): Bar[] {
	const bins = score.histogram
	const total =
		score.totalGames || bins.reduce((sum, bin) => sum + bin.games, 0)
	const ownBins = me?.accomplishments.challenges.clearHistogram ?? []
	const ownTotal = ownBins.reduce((sum, n) => sum + n, 0)
	const shares = bins.map((bin, i) => ({
		pop: total > 0 ? bin.games / total : 0,
		own: ownTotal > 0 ? (ownBins[i] ?? 0) / ownTotal : 0,
	}))
	const tallest = Math.max(0, ...shares.flatMap((s) => [s.pop, s.own]))
	const at = score.challenge
	return bins.map((bin, i) => {
		const { pop, own } = shares[i]!
		const current = at != null && holds(bin.lowerBound, bin.upperBound, at)
		// Where the bin sits is shown on the axis under it, so the text starts
		// with what is in it.
		const parts = [
			`${bin.games} eligible games (${(pop * 100).toFixed(1)}% of played games).`,
			`${bin.wins} human wins (${bin.games > 0 ? ((bin.wins / bin.games) * 100).toFixed(1) : '0.0'}%).`,
		]
		if (me)
			parts.push(
				`Your clears: ${ownBins[i] ?? 0} of ${ownTotal} (${(own * 100).toFixed(1)}%).`,
			)
		if (current && at != null)
			parts.push(`Your current setup is here at ${at.toFixed(1)}.`)
		// Placed by its own bounds, so the axis labels line up with the bars
		// whatever widths the service's bins have.
		const lower = Math.min(Math.max(bin.lowerBound, 0), TOP)
		const upper = Math.min(Math.max(bin.upperBound, lower), TOP)
		const base = {
			left: lower / TOP,
			width: (upper - lower) / TOP,
			mid: Math.round((lower + upper) / 2),
			current,
			help: parts.join('\n'),
		}
		// A win rate is its own scale, 0 to 1; there is no rate of your own to
		// set beside it, as the service counts your clears but not your games.
		if (measure === 'winrate') {
			const rate = bin.games > 0 ? bin.wins / bin.games : 0
			return {
				...base,
				...drawn(rate, 0),
				popValue: rate,
				ownValue: null,
				sure: Math.max(0.25, Math.min(1, Math.sqrt(bin.games / SURE_AT))),
			}
		}
		return {
			...base,
			...drawn(
				tallest > 0 ? pop / tallest : 0,
				tallest > 0 ? own / tallest : 0,
			),
			popValue: pop,
			ownValue: (ownBins[i] ?? 0) > 0 ? own : null,
			sure: 1,
		}
	})
}

/** Heights as drawn: a bar with anything in it never vanishes to a hairline. */
function drawn(share: number, own: number) {
	return {
		share,
		own,
		popHeight: share > 0 ? Math.max(2, share * 100) : 0,
		ownHeight: own > 0 ? Math.max(3, own * 100) : 0,
	}
}

/** A reading line's label height, in percent of the chart's height. */
const LABEL = 15
/** How far over the chart's top a label may reach: into its padding. */
const HEADROOM = 8

/**
 * Which labels sit below their line, for lines at these heights: above when
 * there is room, below when a line is near the top, and two close lines'
 * labels on opposite sides so they do not print over each other. With no
 * clean placement, the higher label above (over the top if it must) and the
 * lower one below, which at least keeps them apart.
 */
export function place(heights: number[]): boolean[] {
	const span = (h: number, below: boolean) =>
		below ? [h - LABEL, h] : [h, h + LABEL]
	const fits = (h: number, below: boolean) =>
		below ? h - LABEL >= 0 : h + LABEL <= 100 + HEADROOM
	const options: boolean[][] =
		heights.length === 1
			? [[false], [true]]
			: [
					[false, false],
					[false, true],
					[true, false],
					[true, true],
				]
	const clear = (choice: boolean[]) => {
		if (!choice.every((below, i) => fits(heights[i]!, below))) return false
		if (heights.length < 2) return true
		const [a0, a1] = span(heights[0]!, choice[0]!)
		const [b0, b1] = span(heights[1]!, choice[1]!)
		return a1! <= b0! || b1! <= a0!
	}
	// The first of the highest stays above; every other label goes below.
	const top = heights.indexOf(Math.max(...heights))
	return options.find(clear) ?? heights.map((_, i) => i !== top)
}

/** One reading line: how high, what it says, whose bar, and which side of
 * the line its label sits so two close lines do not print over each other. */
export type Guide = {
	height: number
	value: number
	own: boolean
	below: boolean
}

/**
 * The reading lines for a hovered bar: from the top of each of its bars out
 * to one edge of the chart, where the value is written.
 *
 * One side only, not both, which would be noise: the edge whose bar is the
 * shorter, because that bar is what the written values sit over and the line
 * on its way there is only a guide. A tie goes to the nearer edge, the
 * shorter line. `from` and `to` are where the line runs, as fractions of the
 * chart.
 */
export function guides(
	shown: Bar[],
	at: number,
): { side: 'left' | 'right'; from: number; to: number; lines: Guide[] } {
	const bar = shown[at]
	if (!bar) return { side: 'left', from: 0, to: 0, lines: [] }
	const lines: Guide[] = []
	if (bar.popHeight > 0)
		lines.push({
			height: bar.popHeight,
			value: bar.popValue,
			own: false,
			below: false,
		})
	if (bar.ownHeight > 0 && bar.ownValue != null)
		lines.push({
			height: bar.ownHeight,
			value: bar.ownValue,
			own: true,
			below: false,
		})
	const placed = place(lines.map((line) => line.height))
	for (const [i, line] of lines.entries()) line.below = placed[i]!
	const tall = (edge: Bar | undefined) =>
		edge ? Math.max(edge.popHeight, edge.ownHeight) : 0
	const first = tall(shown[0])
	const last = tall(shown[shown.length - 1])
	const left =
		first !== last ? first < last : bar.left <= 1 - (bar.left + bar.width)
	return left
		? { side: 'left', from: 0, to: bar.left, lines }
		: { side: 'right', from: bar.left + bar.width, to: 1, lines }
}

export function histogramHelp(
	score: Score,
	me: Player | undefined,
	measure: Measure = 'count',
): string {
	if (measure === 'winrate') {
		const predicted =
			score.winChance != null && score.challenge != null
				? ` The diamond is the model's ${Math.round(score.winChance * 100)}% for this setup, at its challenge ${score.challenge.toFixed(1)}: above the bars means it expects more wins than games there produced.`
				: ''
		return `Bars show how often human teams won the eligible games at each challenge score, from 0 to 100% with a line at 50%. A bar resting on few games is drawn fainter.${predicted}`
	}
	const ownTotal =
		me?.accomplishments.challenges.clearHistogram.reduce((s, n) => s + n, 0) ??
		0
	const own = me
		? ` Bright bars show your ${ownTotal} eligible clears.`
		: ' Bright bars appear when your own record is available.'
	return `Dark bars show ${score.totalGames} eligible games by challenge score.${own} Both use one percentage scale, so bright above dark means this range holds a larger share of your clears than of all played games.`
}

export function caption(score: Score, me: Player | undefined): string {
	let text = 'Current setup: not yet placed'
	if (score.challenge != null) {
		text = `Challenge ${score.challenge.toFixed(1)}`
		if (score.percentile != null)
			text += ` — harder than ${Math.round(score.percentile)}% of played ${score.aiType || 'PvE'} games`
	}
	const best = me?.accomplishments.challenges.highestChallengeCleared
	if (best != null) text += ` — your best ${best.toFixed(1)}`
	return text
}

/** "2024-03-10 – 4d 2h ago": the span of games the answer rests on. */
export function dataWindow(score: Score, now = Date.now()): string {
	const until = score.dataUntil ? age(score.dataUntil, now) : ''
	if (!until) return ''
	const since = score.dataSince?.slice(0, 10)
	return since ? `${since} – ${until}` : until
}

/** The Match figure: how the served setting relates to this room. */
export function matchText(score: Score): string {
	switch (score.matched.status) {
		case 'exact':
			return 'Exact'
		case 'closest':
			if (score.matched.method === 'raw_fallback') return 'Raw fallback'
			return score.matched.similarity != null
				? `Similar ${score.matched.similarity.toFixed(3)}`
				: 'Similar'
		case 'not_found':
			return 'Not found'
		default:
			return score.matched.status || '—'
	}
}

/** Why the numbers are estimates rather than measurements, or ''. */
export function bestEffortText(score: Score): string {
	const held = score.degradation
	if (!held) return ''
	if (held.reason === 'unknown_modoptions') {
		const n = held.unknownSettingCount
		const what = n > 0 ? `${n} ${n === 1 ? 'option' : 'options'}` : 'options'
		return `This lobby uses ${what} this server has not catalogued yet, so an exact match cannot be confirmed. Values shown are best-effort estimates; see Diagnostics for details.`
	}
	if (held.reason !== 'unsupported_modoptions') return ''
	const names = held.unsupportedSettingNames
	const options = names.filter((n) => !UNSEEN[n]).length
	const clauses: string[] = []
	if (options > 0)
		clauses.push(
			`no recorded games have changed ${options} ${options === 1 ? 'option' : 'options'} this lobby uses`,
		)
	if (names.includes('__unseen_tweak_profile__'))
		clauses.push("no recorded game used this lobby's tweak files")
	if (names.includes('__unseen_default_identity__'))
		clauses.push('no recorded game used this exact setup')
	if (clauses.length === 0)
		clauses.push('no recorded games cover some of what this lobby uses')
	const summary = clauses.join(', and ')
	return `${summary[0]!.toUpperCase()}${summary.slice(1)}. Values shown are best-effort estimates rather than measured results; see Diagnostics for details.`
}

/** The service's two placeholders for "unsupported" things that are not options. */
const UNSEEN: Record<string, string> = {
	__unseen_default_identity__: 'this exact setup',
	__unseen_tweak_profile__: "this lobby's tweak files",
}

export function matchHelp(score: Score): string {
	let text =
		'The matched lobby setting supplies setting-specific statistics and displayed differences. Match is separate from Win chance and is not a confidence score.'
	if (score.matched.status === 'closest')
		text =
			score.matched.method === 'raw_fallback'
				? 'Raw fallback compares available lobby fields for the setting-specific statistics and differences shown. The overlap is not model confidence and does not say which setup is harder. Match selection is separate from Win chance.'
				: 'Similarity summarizes the comparison used for the setting-specific statistics and differences shown. 1.000 is the closest possible match; it is not confidence and does not say which setup is harder. Match selection is separate from Win chance.'
	const best = bestEffortText(score)
	return best ? `${text} ${best}` : text
}

function hiddenText(score: Score): string {
	const n = score.matched.hiddenDifferences
	if (n <= 0) return ''
	return `${n} additional field difference${n === 1 ? '' : 's'} hidden`
}

/** "Match: raw lobby fallback; 2 visible differences; 1 additional field difference hidden" */
export function matchSummary(score: Score): string {
	if (score.matched.status === 'exact') return 'Match: exact setting'
	if (!score.matched.method && score.matched.status !== 'closest')
		return 'Match: no trusted setting match'
	const parts = [
		score.matched.method === 'similar'
			? 'similar setting'
			: 'raw lobby fallback',
	]
	const visible = score.matched.diffs.length
	parts.push(`${visible} visible difference${visible === 1 ? '' : 's'}`)
	const hidden = hiddenText(score)
	if (hidden) parts.push(hidden)
	return `Match: ${parts.join('; ')}`
}

function rawOverlap(score: Score): string {
	if (score.matched.method !== 'raw_fallback') return ''
	const held = score.completeness
	if (!held || held.totalHashColumns <= 0)
		return 'lobby-field comparison unavailable'
	const missing = held.missingHashColumns
	const compared = Math.max(0, held.totalHashColumns - missing)
	const known = Math.max(0, score.matched.differenceCount - missing)
	const matching = Math.max(0, compared - known)
	if (compared <= 0) return `${missing} unknown fields`
	const unknown = missing > 0 ? `; ${missing} unknown` : ''
	return `${((100 * matching) / compared).toFixed(1)}% (${matching}/${compared} compared${unknown})`
}

/** At most eight names, the rest counted. */
function some(names: string[], total = names.length): string {
	const shown = names.slice(0, 8).map((n) => UNSEEN[n] ?? n)
	const more = total - shown.length
	return more > 0 ? `${shown.join(', ')} (+${more} more)` : shown.join(', ')
}

/** The widget's support rows, without the transport ones a lobby does not keep. */
export function diagnosticRows(
	score: Score,
): { label: string; value: string }[] {
	const out: { label: string; value: string }[] = []
	const add = (label: string, value: string) => {
		if (value) out.push({ label, value })
	}
	add('Match', matchSummary(score).replace(/^Match: /, ''))
	add('Raw overlap', rawOverlap(score))
	const held = score.completeness
	if (held) {
		const fields = [
			`provided ${held.providedHashColumns}`,
			`derived ${held.derivedHashColumnNames.length}`,
			`defaulted ${held.defaultedHashColumns}`,
			`missing ${held.missingHashColumns}`,
			`total ${held.totalHashColumns}`,
		]
		if (held.unknownSettingCount > 0)
			fields.push(`unknown ${held.unknownSettingCount}`)
		add('Request fields', fields.join('; '))
	}
	const bad = score.degradation
	if (bad?.unknownSettingNames.length)
		add(
			'Uncatalogued',
			some(
				bad.unknownSettingNames,
				Math.max(bad.unknownSettingCount, bad.unknownSettingNames.length),
			),
		)
	if (bad?.unsupportedSettingNames.length)
		add('No evidence for', some(bad.unsupportedSettingNames))
	const ids = [
		score.contractHash && `contract ${score.contractHash.slice(0, 12)}`,
		score.settingHash && `query ${score.settingHash.slice(0, 12)}`,
	].filter(Boolean)
	add('IDs', ids.join('; '))
	return out
}
