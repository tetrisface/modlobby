import { For, Show, createMemo, createSignal } from 'solid-js'
import { Segmented } from '../components/Segmented'
import { TabStrip } from '../components/TabStrip'
import { Thinking } from '../components/Thinking'
import type { Score } from '../ipc/bindings/Score'
import {
	BY_NAME,
	type Measure,
	type Row,
	TABS,
	TICKS,
	TOP,
	type Tab,
	bars,
	bestEffortText,
	caption,
	cascade,
	cell,
	columns,
	dataWindow,
	defaultSort,
	diagnosticRows,
	guides,
	histogramHelp,
	matchHelp,
	matchSummary,
	matchText,
	ownPlayer,
	rows,
	sortRows,
} from '../lib/pve'
import { sticky } from '../lib/sticky'
import { useRoom } from './room/model'

const percent = (value: number | null) =>
	value == null ? '—' : `${Math.round(value * 100)}%`

const MEASURES = [
	{
		value: 'count',
		label: 'count',
		title: 'How many eligible games were played at each challenge score',
	},
	{
		value: 'winrate',
		label: 'win rate',
		title: 'How often human teams won the games at each challenge score',
	},
] as const

/**
 * The in-game widget's whole panel, in the lobby.
 *
 * The widget cannot be kept working, so this is where its view lives on: the
 * same five figures, the histogram with your clears over everyone's games,
 * the data window, the match evidence, and each person's record under the
 * widget's four tabs, with its Diagnostics as a fifth. Nothing moves when a
 * control is used: every view scrolls inside one fixed box, so spectator
 * rows and tab changes never resize the card.
 */
export function PveStats(props: {
	score: Score | null | undefined
	waiting: boolean
	failure: string | null
	onClose: () => void
}) {
	const room = useRoom()
	/** The record tabs, plus the widget's Diagnostics as a view of its own. */
	const [view, setView] = createSignal<Tab | 'diagnostics'>('achievements')
	const [tab, setTab] = createSignal<Tab>('achievements')
	const [sort, setSort] = createSignal<number | typeof BY_NAME>(
		defaultSort('achievements', props.score?.aiType ?? ''),
	)
	const [descending, setDescending] = createSignal(true)
	const [spectators, setSpectators] = createSignal(false)
	/** What the histogram's bars measure, kept from one opening to the next. */
	const [held, setMeasure] = sticky<Measure>('pve.measure', 'count')
	const measure = (): Measure => (held() === 'winrate' ? 'winrate' : 'count')

	const ai = () => props.score?.aiType ?? ''
	const me = () => (props.score ? ownPlayer(props.score, room.me()) : undefined)

	/** The histogram bar under the pointer, which the reading lines follow. */
	const [hovered, setHovered] = createSignal<number | null>(null)
	const shown = createMemo(() =>
		props.score ? bars(props.score, me(), measure()) : [],
	)
	const reading = () => {
		const at = hovered()
		return at === null ? null : guides(shown(), at)
	}
	const cols = createMemo(() =>
		props.score ? columns(tab(), props.score) : [],
	)

	const pick = (next: Tab | 'diagnostics') => {
		setView(next)
		if (next === 'diagnostics') return
		setTab(next)
		setSort(defaultSort(next, ai()))
		setDescending(true)
	}
	const sortBy = (by: number | typeof BY_NAME) => {
		if (sort() === by) setDescending(!descending())
		else {
			setSort(by)
			setDescending(by !== BY_NAME)
		}
	}
	const mark = (by: number | typeof BY_NAME) =>
		sort() === by ? (descending() ? 'descending' : 'ascending') : 'none'

	/** Seated people and watchers, each sorted the same way. */
	const groups = createMemo(() => {
		if (!props.score) return { players: [] as Row[], watchers: [] as Row[] }
		const users = room.users()
		const watching = (name: string) =>
			users[name]?.battleStatus?.player === false
		const all = rows(props.score)
		const order = (held: Row[]) =>
			sortRows(held, cols(), sort(), descending(), cascade(tab(), ai()))
		return {
			players: order(all.filter((row) => !watching(row.name))),
			watchers: order(all.filter((row) => watching(row.name))),
		}
	})

	const inexact = () => {
		const source = props.score?.setup?.source
		return cols().some((c) => c.setup) && !!source && source !== 'exact'
	}

	const Figure = (held: {
		label: string
		value: string
		help: string
		class?: string
	}) => (
		<div class='pve-metric' title={held.help}>
			<span class='pve-label'>{held.label}</span>
			<span class={`pve-figure ${held.class ?? ''}`}>{held.value}</span>
		</div>
	)

	const Table = (held: { rows: Row[]; empty: string }) => (
		<Show
			when={held.rows.length > 0}
			fallback={
				<tr class='pve-empty'>
					<td colspan={cols().length + 1}>{held.empty}</td>
				</tr>
			}
		>
			<For each={held.rows}>
				{(row) => (
					<tr classList={{ own: row.name === room.me() }}>
						<td class='pve-name'>{row.name}</td>
						<For each={cols()}>
							{(column) => (
								<td class='num' classList={{ 'pve-setup': column.setup }}>
									{row.player ? cell(column.value(row.player)) : ''}
								</td>
							)}
						</For>
					</tr>
				)}
			</For>
		</Show>
	)

	return (
		<div class='sheet' onMouseDown={props.onClose}>
			<div
				class='sheet-card pve-stats'
				onMouseDown={(event) => event.stopPropagation()}
			>
				<header class='ed-head'>
					<h2>
						PvE stats <span class='muted'>{ai()}</span>
					</h2>
					<span class='spacer' />
					<button
						type='button'
						classList={{ on: spectators() }}
						title='Records of the people watching, under the players'
						onClick={() => setSpectators(!spectators())}
					>
						Spectators
					</button>
					<button type='button' onClick={props.onClose}>
						Close
					</button>
				</header>

				<Show
					when={props.score}
					fallback={
						<Show
							when={!props.waiting && props.failure}
							fallback={
								<Thinking still title='asking how hard this setup is' />
							}
						>
							{(why) => <p class='muted'>Unavailable: {why()}</p>}
						</Show>
					}
				>
					{(score) => (
						<>
							<div class='pve-summary'>
								<Figure
									label='Win chance'
									value={percent(score().winChance)}
									help='Estimated chance a representative current BAR human team wins this map and setup. The people in this room are not part of that estimate.'
								/>
								<Figure
									label='Challenge'
									value={
										score().challenge == null
											? 'Unplaced'
											: score().challenge!.toFixed(1)
									}
									help='Absolute difficulty on a 0-34 scale. 17 is an estimated even game for a representative human team; higher is harder. Difficulty percentile is the relative placement among played games.'
								/>
								<Figure
									label='Difficulty percentile'
									value={
										score().percentile == null
											? 'Unplaced'
											: `P${Math.round(score().percentile!)}`
									}
									help={`Share of eligible played ${ai()} games whose setup had a lower Challenge than this one. A relative placement, not the Challenge score itself.`}
								/>
								<Figure
									label='Training games'
									value={
										score().evidenceGames == null
											? '—'
											: String(Math.round(score().evidenceGames!))
									}
									help={`Eligible ${ai()} games used to train this model after validity and grace-period filtering. Overall model data, not the number of exact or nearby matches and not a confidence score.`}
								/>
								<Figure
									label='Match'
									value={matchText(score())}
									help={matchHelp(score())}
									class={
										score().matched.status === 'exact'
											? 'exact'
											: bestEffortText(score())
												? 'best-effort'
												: ''
									}
								/>
							</div>

							<Show when={score().histogram.length > 0}>
								<div class='pve-hist'>
									<div
										class='pve-bars'
										classList={{ rate: measure() === 'winrate' }}
										onMouseLeave={() => setHovered(null)}
									>
										<Show when={measure() === 'winrate'}>
											<i class='pve-half' />
											<Show
												when={
													score().winChance != null && score().challenge != null
												}
											>
												<i
													class='pve-predicted'
													style={{
														left: `${(Math.min(score().challenge!, TOP) / TOP) * 100}%`,
														bottom: `${score().winChance! * 100}%`,
													}}
												/>
											</Show>
										</Show>
										<For each={shown()}>
											{(bar, index) => (
												<div
													class='pve-bar'
													classList={{
														current: bar.current,
														// Near an edge the panel opens inward, so it is never
														// cut off by the card.
														start: bar.left + bar.width / 2 < 1 / 3,
														end: bar.left + bar.width / 2 > 2 / 3,
													}}
													style={{
														left: `${bar.left * 100}%`,
														width: `${bar.width * 100}%`,
													}}
													onMouseEnter={() => setHovered(index())}
												>
													<i
														class='pop'
														style={{
															height: `${bar.popHeight}%`,
															'--sure': bar.sure,
														}}
													/>
													<Show when={bar.ownHeight > 0}>
														<i
															class='own'
															style={{ height: `${bar.ownHeight}%` }}
														/>
													</Show>
													{/* The bin's place on the axis, over the ticks while
													    hovered, then the widget's panel below the axis:
													    instant and styled, unlike a native title. */}
													<span class='pve-at'>{bar.mid}</span>
													<span class='pve-tip' role='tooltip'>
														{bar.help}
													</span>
												</div>
											)}
										</For>
										{/* Reading lines for the hovered bar: from each of its
										    bars' tops to the quieter edge, the value written there. */}
										<Show when={reading()}>
											{(read) => (
												<For each={read().lines}>
													{(line) => (
														<div
															class='pve-guide'
															classList={{
																own: line.own,
																below: line.below,
																right: read().side === 'right',
															}}
															style={{
																left: `${read().from * 100}%`,
																width: `${(read().to - read().from) * 100}%`,
																bottom: `${line.height}%`,
															}}
														>
															<span>{(line.value * 100).toFixed(1)}%</span>
														</div>
													)}
												</For>
											)}
										</Show>
									</div>
									<div class='pve-scale'>
										<For each={TICKS}>
											{(at) => (
												<span
													class='pve-tick'
													style={{ left: `${(at / TOP) * 100}%` }}
												>
													{at}
												</span>
											)}
										</For>
									</div>
									<div class='pve-legend-row'>
										<div
											class='pve-legend'
											title={histogramHelp(score(), me(), measure())}
										>
											<Show
												when={measure() === 'winrate'}
												fallback={
													<>
														<span>
															<i class='pve-swatch pop' />
															Played games
														</span>
														<span>
															<i class='pve-swatch own' />
															Your clears
														</span>
													</>
												}
											>
												<span>
													<i class='pve-swatch pop' />
													Human win rate
												</span>
												<Show
													when={
														score().winChance != null &&
														score().challenge != null
													}
												>
													<span>
														<i class='pve-swatch predicted' />
														Predicted
													</span>
												</Show>
											</Show>
											<span>
												<i class='pve-swatch current' />
												Current
											</span>
										</div>
										<Segmented
											label='Bar height'
											value={measure()}
											options={MEASURES}
											onChange={setMeasure}
										/>
									</div>
									<p class='pve-caption'>{caption(score(), me())}</p>
								</div>
							</Show>

							<Show when={dataWindow(score())}>
								{(window) => (
									<div
										class='pve-line'
										title='The games the service rests on: the oldest to how long ago the newest was played. Games after that are not part of these numbers yet.'
									>
										<span class='pve-label'>Data</span>
										<span>{window()}</span>
									</div>
								)}
							</Show>
							<p class='pve-line muted'>{matchSummary(score())}</p>
							<Show when={bestEffortText(score())}>
								{(text) => <p class='pve-line warn'>{text()}</p>}
							</Show>

							<section class='pve-players'>
								<TabStrip
									tabs={[
										...TABS.map((held) => ({ ...held })),
										{
											key: 'diagnostics',
											label: 'Diagnostics',
											title:
												'Field differences, request coverage, match details and ids',
										},
									]}
									active={view()}
									onSelect={(key) => pick(key as Tab | 'diagnostics')}
								/>
								<div class='pve-scroll'>
									<Show
										when={view() !== 'diagnostics'}
										fallback={
											<div class='pve-diag'>
												<For each={diagnosticRows(score())}>
													{(row) => (
														<div class='pve-diag-row'>
															<span class='pve-label'>{row.label}</span>
															<span>{row.value}</span>
														</div>
													)}
												</For>
												<Show when={score().matched.diffs.length > 0}>
													<table class='widget-table pve-table pve-diffs'>
														<thead>
															<tr>
																<th>Field</th>
																<th>Current</th>
																<th>
																	{score().matched.method === 'raw_fallback'
																		? 'Fallback'
																		: 'Similar'}
																</th>
															</tr>
														</thead>
														<tbody>
															<For each={score().matched.diffs}>
																{(diff) => (
																	<tr>
																		<td>{diff.column}</td>
																		<td>{diff.current}</td>
																		<td>{diff.closest}</td>
																	</tr>
																)}
															</For>
														</tbody>
													</table>
												</Show>
											</div>
										}
									>
										<table class='widget-table pve-table'>
											<thead>
												<tr>
													<th aria-sort={mark(BY_NAME)}>
														<button
															type='button'
															class='sort-head'
															onClick={() => sortBy(BY_NAME)}
														>
															Player
															<span class='sort-mark' aria-hidden='true'>
																{sort() === BY_NAME
																	? descending()
																		? '↓'
																		: '↑'
																	: ''}
															</span>
														</button>
													</th>
													<For each={cols()}>
														{(column, index) => (
															<th
																class='num'
																classList={{ 'pve-setup': column.setup }}
																aria-sort={mark(index())}
																title={column.help}
															>
																<button
																	type='button'
																	class='sort-head'
																	onClick={() => sortBy(index())}
																>
																	{column.label}
																	{inexact() && column.setup ? '*' : ''}
																	<span class='sort-mark' aria-hidden='true'>
																		{sort() === index()
																			? descending()
																				? '↓'
																				: '↑'
																			: ''}
																	</span>
																</button>
															</th>
														)}
													</For>
												</tr>
											</thead>
											<tbody>
												<Table
													rows={groups().players}
													empty='No player records'
												/>
												<Show when={spectators()}>
													<tr class='pve-group'>
														<td colspan={cols().length + 1}>Spectators</td>
													</tr>
													<Table
														rows={groups().watchers}
														empty='No spectator records'
													/>
												</Show>
											</tbody>
										</table>
									</Show>
								</div>
							</section>
						</>
					)}
				</Show>
			</div>
		</div>
	)
}
