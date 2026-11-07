import { describe, expect, test } from 'vitest'
import type { Player } from '../ipc/bindings/Player'
import type { Bar } from './pve'
import type { Score } from '../ipc/bindings/Score'
import {
	BY_NAME,
	OWN_RATE_FLOOR,
	bars,
	bestEffortText,
	caption,
	declinedText,
	opponentText,
	cascade,
	cell,
	columns,
	dataWindow,
	defaultSort,
	diagnosticRows,
	fieldRatio,
	guides,
	histogramHelp,
	matchSummary,
	miniature,
	place,
	matchText,
	rows,
	sortRows,
} from './pve'

function player(
	name: string,
	held: Partial<Player['accomplishments']['challenges']> & {
		clears?: number
		max?: number | null
	} = {},
): Player {
	return {
		playerId: name.length,
		playerName: name,
		setupExperience: {
			clears: held.clears ?? 0,
			maxDefeated: held.max ?? null,
		},
		accomplishments: {
			participation: { gamesPlayed: 0, victories: 0, distinctMapsPlayed: 0 },
			encounters: {
				raptorQueensDefeated: 0,
				scavengerBossesDefeated: 0,
				barbarianAisDefeated: 0,
			},
			personalBests: {
				maxQueensOneVictory: 0,
				maxBossesOneVictory: 0,
				maxBarbarianAisOneVictory: 0,
			},
			challenges: {
				challenge20Clears: held.challenge20Clears ?? 0,
				challenge25Clears: held.challenge25Clears ?? 0,
				challenge30Clears: held.challenge30Clears ?? 0,
				highestChallengeCleared: held.highestChallengeCleared ?? null,
				clearHistogram: held.clearHistogram ?? [],
				playHistogram: held.playHistogram ?? [],
			},
		},
		awards: { mostKilled: { raptors: 0, scavengers: 0, barbarians: 0 } },
	}
}

const SCORE: Score = {
	challenge: 21.5,
	percentile: 88,
	winChance: 0.31,
	evidenceGames: 4200,
	bestEffort: false,
	dataUntil: '2026-06-20T22:46:17Z',
	dataSince: '2025-01-02T03:04:05Z',
	aiType: 'Raptors',
	aiFamily: 'RaptorsAI',
	histogram: [
		{ lowerBound: 0, upperBound: 17, games: 100, wins: 90 },
		{ lowerBound: 17, upperBound: 34, games: 300, wins: 30 },
	],
	totalGames: 400,
	matched: {
		status: 'closest',
		method: 'raw_fallback',
		similarity: null,
		differenceCount: 3,
		hiddenDifferences: 1,
		diffs: [{ column: 'raptor_endless', current: '1', closest: '0' }],
	},
	degradation: null,
	completeness: {
		providedHashColumns: 7,
		defaultedHashColumns: 1,
		missingHashColumns: 1,
		totalHashColumns: 9,
		unknownSettingCount: 0,
		derivedHashColumnNames: ['a'],
	},
	players: [
		player('me', {
			clearHistogram: [1, 3],
			playHistogram: [2, 12],
			highestChallengeCleared: 24.8,
			clears: 2,
			max: 50,
		}),
		player('Zed', { clears: 2, max: 20, challenge20Clears: 9 }),
		player('amy', { clears: 5 }),
	],
	unresolvedPlayers: ['nobody', 'ME'],
	setup: { source: 'raw_fallback', encounterCount: 20 },
	settingHash: 'abc123def456789',
	contractHash: 'feedfacefeedface',
}

describe('pve view logic', () => {
	test('the histogram scales both series to the tallest bar and marks the current bin', () => {
		const drawn = bars(SCORE, SCORE.players[0])
		// Played: 25% / 75%. Own: 25% / 75%. Tallest share is 0.75.
		expect(drawn.map((b) => b.share)).toEqual([1 / 3, 1])
		expect(drawn.map((b) => b.own)).toEqual([1 / 3, 1])
		expect(drawn.map((b) => b.current)).toEqual([false, true])
		// 0-17 and 17-34: each half the axis, the second starting halfway.
		expect(drawn.map((b) => [b.left, b.width])).toEqual([
			[0, 0.5],
			[0.5, 0.5],
		])
		// One fact a line; the range itself is on the axis, not in the text.
		expect(drawn[1]!.help.split('\n')).toEqual([
			'300 eligible games (75.0% of played games).',
			'30 human wins (10.0%).',
			'Your record here: 3 of 12 played (25.0%).',
			'Your current setup is here at 21.5.',
		])
		// 0-17 and 17-34: middles 8.5 and 25.5, rounded.
		expect(drawn.map((b) => b.mid)).toEqual([9, 26])
	})

	test('a service that counts no plays gets the share-of-clears wording and no own rate', () => {
		const older = player('me', {
			clearHistogram: [1, 3],
			highestChallengeCleared: 24.8,
		})
		const counted = bars(SCORE, older)
		// A share of clears, said as one: 3 of your 4 clears fall here.
		expect(counted[1]!.help.split('\n')[2]).toBe('Your clears: 3 of 4 (75.0%).')
		expect(counted.map((b) => b.own)).toEqual([1 / 3, 1])
		const rated = bars(SCORE, older, 'winrate')
		expect(rated.map((b) => b.ownText)).toEqual([null, null])
		expect(rated.map((b) => b.own)).toEqual([0, 0])
		expect(fieldRatio(SCORE, older)).toBeNull()
		expect(caption(SCORE, older)).toBe(
			'Challenge 21.5 — harder than 88% of played Raptors games — your best 24.8',
		)
		expect(histogramHelp(SCORE, older, 'winrate')).not.toContain('Bright bars')
	})

	test('reading lines run to the edge whose bar is shorter, whatever is between', () => {
		const slot = (i: number, pop: number, own = 0): Bar => ({
			left: i / 5,
			width: 1 / 5,
			mid: 0,
			share: 0,
			own: 0,
			popHeight: pop,
			ownHeight: own,
			popText: `${pop}%`,
			ownText: own > 0 ? `${own}` : null,
			sure: 1,
			current: false,
			help: '',
		})
		// The values are written over the edge bars, so only those count: a
		// clears spike on the first (100) loses to a games bar of 90 on the
		// last, however tall the bars in between.
		const shown = [
			slot(0, 10, 100),
			slot(1, 95),
			slot(2, 30),
			slot(3, 40),
			slot(4, 90),
		]
		expect(guides(shown, 2).side).toBe('right')
		expect(guides(shown, 1).side).toBe('right')
		// Equal edges: the nearer one.
		const flat = [slot(0, 5), slot(1, 80), slot(2, 50), slot(3, 5)]
		expect(guides(flat, 2).side).toBe('right')
		expect(guides(flat, 1).side).toBe('left')
	})

	test('reading lines and their values for a real answer', () => {
		const shown = bars(SCORE, SCORE.players[0])
		// Two bins: the first a third as tall as the second, so the values go
		// to the left edge whichever is hovered, and the line runs from the
		// edge to the hovered bar.
		const first = guides(shown, 0)
		expect(first.side).toBe('left')
		expect([first.from, first.to]).toEqual([0, 0])
		const second = guides(shown, 1)
		expect(second.side).toBe('left')
		expect([second.from, second.to]).toEqual([0, 0.5])
		// Both bars of the second bin, with what is written for them: 75% of
		// games, and your 3 clears there as a count. Your share of your own
		// clears is a percent of another whole, which beside the games' percent
		// would read as the same kind of number. Same height, so one label goes
		// below.
		expect(second.lines.map((l) => [l.own, l.text])).toEqual([
			[false, '75.0%'],
			[true, '3'],
		])
		expect(second.lines.filter((l) => l.below)).toHaveLength(1)
		// The count is the one the tooltip gives, with or without plays.
		expect(shown[1]!.help).toContain('3 of 12 played')
		const older = player('me', { clearHistogram: [1, 3] })
		expect(guides(bars(SCORE, older), 1).lines[1]!.text).toBe('3')
		// Win rate: one line, the rate.
		const rate = guides(bars(SCORE, SCORE.players[0], 'winrate'), 0)
		expect(rate.lines.map((l) => l.text)).toEqual(['90.0%'])
	})

	test('reading labels stay in the chart and off each other', () => {
		// Room above: above. At the very top: below. On the floor: above.
		expect(place([50])).toEqual([false])
		expect(place([100])).toEqual([true])
		expect(place([5])).toEqual([false])
		// Far apart: both above. Close: split around the pair.
		expect(place([20, 70])).toEqual([false, false])
		expect(place([50, 55])).toEqual([true, false])
		// Close and near the top, no clean placement: the higher one's label
		// goes above anyway and the lower one's below, so they stay apart.
		expect(place([100, 90])).toEqual([false, true])
		expect(place([90, 100])).toEqual([true, false])
	})

	test('win-rate bars are the rate itself, yours beside it where you have played enough', () => {
		const drawn = bars(SCORE, SCORE.players[0], 'winrate')
		// 90 of 100 and 30 of 300: rates on their own 0-1 scale, not shares.
		expect(drawn.map((b) => b.share)).toEqual([0.9, 0.1])
		// 1 of 2 is no rate yet; 3 of 12 is, and it beats the field's 10%.
		expect(OWN_RATE_FLOOR).toBe(5)
		expect(drawn.map((b) => b.own)).toEqual([0, 0.25])
		expect(drawn.map((b) => b.ownText)).toEqual([null, '25.0%'])
		// sqrt(100/400) and sqrt(300/400); counts are always full strength.
		expect(drawn[0]!.sure).toBeCloseTo(0.5)
		expect(drawn[1]!.sure).toBeCloseTo(Math.sqrt(0.75))
		expect(bars(SCORE, SCORE.players[0]).every((b) => b.sure === 1)).toBe(true)
		// The tooltip says the same in either view.
		expect(drawn[1]!.help).toBe(bars(SCORE, SCORE.players[0])[1]!.help)
		expect(histogramHelp(SCORE, SCORE.players[0], 'winrate')).toContain(
			'at least 5 eligible games',
		)
	})

	test('the field ratio weighs your clears against the field at what you play', () => {
		// 2 games at a 90% range and 12 at a 10% one would give 3 clears; 4 is 1.33x.
		expect(fieldRatio(SCORE, SCORE.players[0])).toBeCloseTo(4 / 3)
		expect(fieldRatio({ ...SCORE, histogram: [] }, SCORE.players[0])).toBeNull()
	})

	test('the caption reads like the widget', () => {
		expect(caption(SCORE, SCORE.players[0])).toBe(
			"Challenge 21.5 — harder than 88% of played Raptors games — your best 24.8 — 1.33× the field's clear rate",
		)
		expect(
			caption({ ...SCORE, challenge: null, percentile: null }, undefined),
		).toBe('Current setup: not yet placed')
	})

	test('a skirmish room is placed among the BARb games the histogram counts, whatever AI it has', () => {
		for (const aiFamily of ['BARb', 'SMRTBARb', 'SimpleAI', null])
			expect(
				caption({ ...SCORE, aiType: 'Barbarian', aiFamily }, undefined),
			).toBe('Challenge 21.5 — harder than 88% of played BARb games')
	})

	test('the data window spans the oldest game to how long ago the newest was', () => {
		const now = Date.parse('2026-06-22T22:46:17Z')
		expect(dataWindow(SCORE, now)).toBe('2025-01-02 – 2d ago')
		expect(dataWindow({ ...SCORE, dataUntil: null }, now)).toBe('')
	})

	test('match wording follows status and method', () => {
		expect(matchText(SCORE)).toBe('Raw fallback')
		expect(
			matchText({ ...SCORE, matched: { ...SCORE.matched, status: 'exact' } }),
		).toBe('Exact')
		expect(
			matchText({
				...SCORE,
				matched: { ...SCORE.matched, method: 'similar', similarity: 0.9 },
			}),
		).toBe('Similar 0.900')
		expect(matchSummary(SCORE)).toBe(
			'Match: raw lobby fallback; 1 visible difference; 1 additional field difference hidden',
		)
	})

	test('best effort says which kind of gap the service reported', () => {
		expect(bestEffortText(SCORE)).toBe('')
		expect(
			bestEffortText({
				...SCORE,
				degradation: {
					reason: 'unknown_modoptions',
					unknownSettingCount: 1,
					unknownSettingNames: ['x'],
					unsupportedSettingNames: [],
					unknownAiNames: [],
				},
			}),
		).toMatch(/^This lobby uses 1 option this server/)
		expect(
			bestEffortText({
				...SCORE,
				degradation: {
					reason: 'unsupported_modoptions',
					unknownSettingCount: 0,
					unknownSettingNames: [],
					unsupportedSettingNames: ['a', 'b', '__unseen_tweak_profile__'],
					unknownAiNames: [],
				},
			}),
		).toMatch(
			/^No recorded games have changed 2 options this lobby uses, and no recorded game used this lobby's tweak files\./,
		)
	})

	test('a room the service declined to score says why, and the header names the family', () => {
		const declined = (reason: string, over: Partial<Score> = {}): Score => ({
			...SCORE,
			aiType: 'Barbarian',
			aiFamily: 'SMRTBARb',
			challenge: null,
			winChance: null,
			degradation: {
				reason,
				unknownSettingCount: 0,
				unknownSettingNames: [],
				unsupportedSettingNames: [],
				unknownAiNames: [],
			},
			...over,
		})
		expect(declinedText(SCORE)).toBe('')
		expect(declinedText(declined('unsupported_ai_family'))).toBe(
			"No win chance or challenge: the service has no model for SMRTBARb yet. The records below are each person's lifetime record against Barbarian AIs.",
		)
		// A release that knows the family measures its exact setting.
		expect(
			declinedText(
				declined('unsupported_ai_family', {
					matched: { ...SCORE.matched, status: 'exact', method: '' },
				}),
			),
		).toMatch(/exact-setting figures are measured from recorded SMRTBARb games/)
		expect(
			declinedText(declined('mixed_ai_families', { aiFamily: null })),
		).toMatch(/mixes AI families/)
		const stranger = declined('unknown_ai_family', { aiFamily: null })
		stranger.degradation!.unknownAiNames = ['Circuit']
		expect(declinedText(stranger)).toBe(
			'No win chance or challenge: this room has an AI the service does not recognise (Circuit).',
		)
		// The same text is what the best-effort line shows, so the panel's one
		// explanation slot carries it without a second slot.
		expect(bestEffortText(declined('unsupported_ai_family'))).toMatch(
			/^No win chance/,
		)
		expect(opponentText(SCORE)).toBe('Raptors')
		expect(opponentText(declined('unsupported_ai_family'))).toBe(
			'Skirmish · SMRTBARb',
		)
		expect(
			opponentText({ ...SCORE, aiType: 'Barbarian', aiFamily: 'BARb' }),
		).toBe('Barbarian')
	})

	test('diagnostic rows carry the overlap, the field counts and the ids', () => {
		const held = Object.fromEntries(
			diagnosticRows(SCORE).map((r) => [r.label, r.value]),
		)
		expect(held.Match).toBe(
			'raw lobby fallback; 1 visible difference; 1 additional field difference hidden',
		)
		// 9 total - 1 missing = 8 compared; 3 - 1 = 2 known differences; 6 match.
		expect(held['Raw overlap']).toBe('75.0% (6/8 compared; 1 unknown)')
		expect(held['Request fields']).toBe(
			'provided 7; derived 1; defaulted 1; missing 1; total 9',
		)
		expect(held.IDs).toBe('contract feedfacefeed; query abc123def456')
	})

	test('rows list the known players then the names the service never saw', () => {
		expect(rows(SCORE).map((r) => r.name)).toEqual([
			'me',
			'Zed',
			'amy',
			'nobody',
		])
	})

	test('sorting puts missing values last whichever way, then falls through the cascade', () => {
		const cols = columns('achievements', SCORE)
		const all = rows(SCORE)
		const by = defaultSort('achievements', 'Raptors')
		expect(cols[by]!.label).toBe('Max here')
		expect(
			sortRows(all, cols, by, true, cascade('achievements', 'Raptors')).map(
				(r) => r.name,
			),
		).toEqual(['me', 'Zed', 'amy', 'nobody'])
		expect(
			sortRows(all, cols, by, false, cascade('achievements', 'Raptors')).map(
				(r) => r.name,
			),
		).toEqual(['Zed', 'me', 'amy', 'nobody'])
		// Equal setup clears (me, Zed) split by the cascade's next column.
		expect(
			sortRows(all, cols, 3, true, cascade('achievements', 'Raptors')).map(
				(r) => r.name,
			),
		).toEqual(['amy', 'me', 'Zed', 'nobody'])
		expect(sortRows(all, cols, BY_NAME, false, []).map((r) => r.name)).toEqual([
			'amy',
			'me',
			'nobody',
			'Zed',
		])
	})

	test('the setup columns say what the service actually served', () => {
		const cols = columns('achievements', SCORE)
		expect(cols[3]!.setup).toBe(true)
		expect(cols[3]!.help).toContain('CLOSEST RAW')
		expect(cols[3]!.help).toContain(
			'The matched setting fields 20 queens per game.',
		)
		expect(cols[4]!.help).toContain('20-queen and a 50-queen')
		expect(columns('encounters', SCORE)).toHaveLength(6)
		expect(defaultSort('encounters', 'Scavengers')).toBe(4)
		expect(cascade('encounters', 'Scavengers')).toEqual([4, 1, 3, 5, 0, 2])
		expect(cascade('awards', 'Barbarian')).toEqual([2, 0, 1])
	})

	test('the icon has half as many bars as bins, all equally wide', () => {
		const three: Score = {
			...SCORE,
			challenge: 40,
			histogram: [
				{ lowerBound: 0, upperBound: 10, games: 100, wins: 0 },
				{ lowerBound: 10, upperBound: 20, games: 300, wins: 0 },
				{ lowerBound: 20, upperBound: 34, games: 140, wins: 0 },
			],
		}
		const drawn = miniature(three)
		// Three bins make two bars, 0-17 and 17-34. The middle bin straddles
		// the split, 7 of its 10 points left: 100 + 210 = 310 against
		// 90 + 140 = 230, so the second is 230/310 of the first.
		expect(drawn.bars.map((bar) => [bar.lo, bar.hi])).toEqual([
			[0, 17],
			[17, 34],
		])
		expect(drawn.bars[0]!.height).toBe(1)
		expect(drawn.bars[1]!.height).toBeCloseTo(230 / 310)
		// Seventeen two-point bins make nine bars, none of them narrower.
		const seventeen = miniature({
			...SCORE,
			histogram: Array.from({ length: 17 }, (_, i) => ({
				lowerBound: i * 2,
				upperBound: i * 2 + 2,
				games: 10,
				wins: 0,
			})),
		})
		expect(seventeen.bars).toHaveLength(9)
		expect(
			seventeen.bars.every((bar) => Math.abs(bar.hi - bar.lo - 34 / 9) < 1e-9),
		).toBe(true)
		// A challenge past the scale is drawn at its end.
		expect(drawn.at).toBe(34)
		expect(miniature({ ...three, challenge: null }).at).toBeNull()
	})

	test('a cell blanks zero and absent alike', () => {
		expect(cell(0)).toBe('')
		expect(cell(null)).toBe('')
		expect(cell(12.4)).toBe('12')
	})
})
