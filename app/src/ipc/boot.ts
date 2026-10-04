import type { Boot } from './bindings/Boot'

declare global {
	interface Window {
		__MODLOBBY_BOOT__?: Boot
	}
}

/**
 * What Rust knew before this page existed, handed over with it rather than
 * asked for: every question is a call into Rust, and at the start those wait
 * in line behind one another while the first screen is drawn without its
 * answers.
 *
 * Only for the load it was made with. A reload is given the same script
 * again, saying what was true when the app opened, so a reloaded page gets
 * `null` and asks instead.
 */
export function boot(): Boot | null {
	const [navigation] = performance.getEntriesByType('navigation')
	const first =
		(navigation as PerformanceNavigationTiming | undefined)?.type === 'navigate'
	return first ? (window.__MODLOBBY_BOOT__ ?? null) : null
}
