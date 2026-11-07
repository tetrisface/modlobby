import { describe, expect, it } from 'vitest'

import type { ChatLine } from '../ipc/bindings/ChatLine'
import { isTable, keep, segment } from './hostTable'

const HOST = 'Host[EU7][024]'

let seq = 0
function said(text: string, from = HOST): ChatLine {
	seq += 1
	return {
		seq,
		room: `s @${HOST}`,
		from,
		text,
		kind: 'private',
		mention: false,
		at: 1_700_000_000,
	}
}

const pad = (text: string, width: number) => text.padEnd(width, ' ')

/**
 * SPADS's `formatArray` (`spads.pl:1150`): a centred `[ title ]`, the
 * header, a dash run under each column, the rows, and a closing line of `=`.
 * Columns are as wide as their widest cell and two spaces apart.
 */
function formatArray(
	title: string,
	fields: string[],
	entries: string[][],
): string[] {
	const widths = fields.map((field, i) =>
		Math.max(field.length, ...entries.map((row) => row[i]!.length)),
	)
	const join = (cells: string[], gap = '  ') => cells.join(gap)
	const width = widths.reduce((a, b) => a + b, 0) + 2 * (fields.length - 1)
	const framed = `[ ${title} ]`
	const left = Math.floor((width - framed.length) / 2)
	return [
		pad(' '.repeat(left) + framed, width),
		join(fields.map((field, i) => pad(field, widths[i]!))),
		join(widths.map((w) => '-'.repeat(w))),
		...entries.map((row) => join(row.map((cell, i) => pad(cell, widths[i]!)))),
		join(
			widths.map((w) => '='.repeat(w)),
			'==',
		),
	]
}

const STATS = formatArray(
	'Game statistics',
	['Name', 'Team', 'DamageDealt'],
	[
		['RaptorsDefenseAI(1) (bot)', '1', '523M. (23%)'],
		['tetrisface', '0', '449M. (20%)'],
	],
)

describe('segment', () => {
	it('gathers a SPADS table, title to closing line, into one entry', () => {
		const lines = STATS.map((text) => said(text))
		const entries = segment(lines)

		expect(entries).toHaveLength(1)
		const [table] = entries
		expect(table && isTable(table)).toBe(true)
		expect(table && isTable(table) && table.rows).toEqual(STATS)
		expect(table && isTable(table) && table.line).toBe(lines[0])
	})

	it('leaves chat around a table as the lines it was', () => {
		const before = said('Game over')
		const after = said('thanks for playing')
		const lines = [before, ...STATS.map((text) => said(text)), after]

		const entries = segment(lines)

		expect(entries).toHaveLength(3)
		expect(entries[0]).toBe(before)
		expect(entries[2]).toBe(after)
	})

	it('needs no title', () => {
		const rows = STATS.slice(1)
		const entries = segment(rows.map((text) => said(text)))

		expect(entries).toHaveLength(1)
		expect(entries[0] && isTable(entries[0]) && entries[0].rows).toEqual(rows)
	})

	it('ends a table someone else speaks into, before its closing line', () => {
		const open = STATS.slice(0, 4).map((text) => said(text))
		const other = said('gg', 'tetrisface')

		const entries = segment([...open, other])

		expect(entries).toHaveLength(2)
		expect(entries[0] && isTable(entries[0]) && entries[0].rows).toEqual(
			STATS.slice(0, 4),
		)
		expect(entries[1]).toBe(other)
	})

	it('takes the battle room table out from under the `* ` SPADS says it with', () => {
		const lines = STATS.map((text) => said(`* ${text}`))
		const entries = segment(lines)

		expect(entries).toHaveLength(1)
		expect(entries[0] && isTable(entries[0]) && entries[0].rows).toEqual(STATS)
	})

	it('reads a short line over a dash run as chat, not a header', () => {
		const lines = [said('hello there'), said('---')]
		expect(segment(lines)).toEqual(lines)
	})
})

describe('keep', () => {
	it('hands back the table it was given before, so it is not drawn again', () => {
		const lines = [said('Game over'), ...STATS.map((text) => said(text))]
		const first = segment(lines)
		const again = keep(first, segment([...lines, said('rematch?')]))

		expect(again[1]).toBe(first[1])
	})

	it('takes the new one when the table has grown', () => {
		const lines = STATS.map((text) => said(text))
		const growing = segment(lines.slice(0, 4))
		const grown = keep(growing, segment(lines))

		expect(grown[0]).not.toBe(growing[0])
		expect(grown[0] && isTable(grown[0]) && grown[0].rows).toEqual(STATS)
	})
})
