const MINUTE = 60_000
const HOUR = 60 * MINUTE
const DAY = 24 * HOUR

/**
 * How long ago a moment was, in its largest unit and the next: "1y 1mo ago",
 * "3mo 12d ago", "2h 5m ago", "40s ago", or "just now". Months are thirty
 * days and years 365 -- a label to read at a glance, with the exact moment
 * in the tooltip beside it, not a calendar.
 */
export function age(iso: string, now: number = Date.now()): string {
	const at = Date.parse(iso)
	if (Number.isNaN(at)) return ''
	const gone = Math.max(0, now - at)
	const days = Math.floor(gone / DAY)
	const units: ReadonlyArray<[number, string]> = [
		[Math.floor(days / 365), 'y'],
		[Math.floor((days % 365) / 30), 'mo'],
		[(days % 365) % 30, 'd'],
		[Math.floor((gone % DAY) / HOUR), 'h'],
		[Math.floor((gone % HOUR) / MINUTE), 'm'],
		[Math.floor((gone % MINUTE) / 1000), 's'],
	]
	const first = units.findIndex(([count]) => count > 0)
	if (first < 0) return 'just now'
	const shown = units.slice(first, first + 2).filter(([count]) => count > 0)
	return `${shown.map(([count, unit]) => `${count}${unit}`).join(' ')} ago`
}

/** The exact local date and time, for the tooltip behind `age`. */
export function exactly(iso: string): string {
	const at = Date.parse(iso)
	return Number.isNaN(at) ? '' : new Date(at).toLocaleString()
}

const CLOCK = new Intl.DateTimeFormat([], {
	hour: '2-digit',
	minute: '2-digit',
})

/**
 * `14:07` — the hour and minute is all a backlog needs. `at` is seconds
 * since the epoch; zero is a line with no time.
 */
export function clock(at: number): string {
	return at ? CLOCK.format(at * 1000) : ''
}
