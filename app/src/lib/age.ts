/**
 * How long ago a published date was, in its largest unit and the next: "1y
 * 1m ago", "3m 12d ago", "5d ago", or "today". Months are thirty days and
 * years 365 -- a label to read at a glance, with the exact moment in the
 * tooltip beside it, not a calendar.
 */
export function age(iso: string, now: number = Date.now()): string {
	const at = Date.parse(iso)
	if (Number.isNaN(at)) return ''
	const days = Math.floor((now - at) / 86_400_000)
	if (days < 1) return 'today'
	const units: ReadonlyArray<[number, string]> = [
		[Math.floor(days / 365), 'y'],
		[Math.floor((days % 365) / 30), 'm'],
		[(days % 365) % 30, 'd'],
	]
	const first = units.findIndex(([count]) => count > 0)
	const shown = units.slice(first, first + 2).filter(([count]) => count > 0)
	return `${shown.map(([count, unit]) => `${count}${unit}`).join(' ')} ago`
}

/** The exact local date and time, for the tooltip behind `age`. */
export function exactly(iso: string): string {
	const at = Date.parse(iso)
	return Number.isNaN(at) ? '' : new Date(at).toLocaleString()
}
