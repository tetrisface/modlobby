import type { ChatLine } from '../ipc/bindings/ChatLine'

/**
 * A table a host drew in chat. SPADS answers `!stats`, `!status`, `!list`
 * and a dozen more with its `formatArray` (`spads.pl:1150`): a centred
 * `[ title ]`, a header, a dash run under each column, the rows, and a
 * closing line of `=`, every cell padded to its column and columns two
 * spaces apart. Each of those is a message of its own.
 */
export type HostTable = {
	/** The first of its messages: when it came, and who said it. */
	line: ChatLine
	/** Each message as SPADS padded it. */
	rows: string[]
}

/** What a log draws: a line as it was said, or a table made of several. */
export type Entry = ChatLine | HostTable

export const isTable = (entry: Entry): entry is HostTable => 'rows' in entry

/** What SPADS puts ahead of everything it says in a battle room (`spads.pl:2808`). */
const BATTLE_PREFIX = '* '

const TITLE = /^\s*\[.*\]\s*$/
const RULE = /^-+(?: {2}-+)*$/
const CLOSING = /^=+$/

/**
 * A log's lines with each host table gathered into one entry. Every other
 * line is handed back as it came, so a list keyed by identity keeps it.
 */
export function segment(lines: readonly ChatLine[]): Entry[] {
	const entries: Entry[] = []
	let at = 0
	while (at < lines.length) {
		const first = lines[at]!
		const end = tableEnd(lines, at)
		if (end === null) {
			entries.push(first)
			at += 1
			continue
		}
		const prefix = prefixOf(first)
		entries.push({
			line: first,
			rows: lines.slice(at, end).map((line) => line.text.slice(prefix.length)),
		})
		at = end
	}
	return entries
}

/**
 * `next`, with each table that is in `previous` as well swapped for the one
 * already drawn: a new chat line would otherwise draw every table again, and
 * throw away how far it was scrolled.
 */
export function keep(previous: readonly Entry[], next: Entry[]): Entry[] {
	const drawn = new Map(
		previous.filter(isTable).map((table) => [table.line, table]),
	)
	return next.map((entry) => {
		if (!isTable(entry)) return entry
		const before = drawn.get(entry.line)
		return before && before.rows.length === entry.rows.length ? before : entry
	})
}

const prefixOf = (line: ChatLine) =>
	line.text.startsWith(BATTLE_PREFIX) ? BATTLE_PREFIX : ''

/**
 * Where a table that starts at `start` ends, exclusive; `null` when none
 * does. It runs to its closing line, or to the first line that is not the
 * same speaker's in the same voice: rows a host is still sending are drawn
 * as the table so far.
 */
function tableEnd(lines: readonly ChatLine[], start: number): number | null {
	const first = lines[start]!
	const prefix = prefixOf(first)
	const said = (at: number): string | null => {
		const line = lines[at]
		if (line?.from !== first.from || !line.text.startsWith(prefix)) return null
		return line.text.slice(prefix.length)
	}

	const titled =
		TITLE.test(said(start) ?? '') && heads(said(start + 1), said(start + 2))
	if (!titled && !heads(said(start), said(start + 1))) return null

	let end = start + (titled ? 3 : 2)
	while (end < lines.length) {
		const row = said(end)
		if (row === null) return end
		end += 1
		if (CLOSING.test(row.trimEnd())) return end
	}
	return end
}

/**
 * Whether `header` over `rule` is the head of a table: the rule a dash run
 * under each column, and the header no wider, with the two spaces SPADS
 * leaves before every column but the first.
 */
function heads(header: string | null, rule: string | null): boolean {
	if (header === null || rule === null) return false
	const dashes = rule.trimEnd()
	if (!RULE.test(dashes) || header.trimEnd().length > dashes.length)
		return false
	return columnStarts(dashes).every(
		(at) => at === 0 || header.slice(at - 2, at) === '  ',
	)
}

function columnStarts(rule: string): number[] {
	return [...rule].flatMap((char, at) =>
		char === '-' && (at === 0 || rule[at - 1] === ' ') ? [at] : [],
	)
}
