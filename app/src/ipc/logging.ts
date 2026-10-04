import { invoke } from '@tauri-apps/api/core'

type Level = 'error' | 'warn' | 'info' | 'debug'

const FORWARDED: Level[] = ['error', 'warn', 'info', 'debug']

function describe(value: unknown): string {
	if (typeof value === 'string') return value
	if (value instanceof Error) return `${value.message}\n${value.stack ?? ''}`
	try {
		return JSON.stringify(value)
	} catch {
		return String(value)
	}
}

const milestones: string[] = []

/**
 * One milestone of the page's start: what has happened, and how many
 * milliseconds after the page began to load. Held until `reportStart`, since
 * every line logged is a call into Rust and the start has enough of those
 * waiting in line.
 */
export function milestone(what: string): void {
	milestones.push(`${what} ${Math.round(performance.now())}`)
}

/**
 * Logs the milestones so far as one line, to read beside Rust's `startup:`
 * lines. The last of them is the moment of sending.
 */
export function reportStart(): void {
	console.debug(`startup: page ${milestones.splice(0).join(', ')}`)
}

/**
 * Sends the webview's console into the same file the Rust side writes, so a UI
 * error and the protocol traffic around it sit on one timeline. The original
 * console still runs, so devtools are unaffected.
 */
export function captureConsole(): void {
	for (const level of FORWARDED) {
		const original = console[level].bind(console)
		console[level] = (...args: unknown[]) => {
			original(...args)
			void invoke('log_message', {
				level,
				message: args.map(describe).join(' '),
			}).catch(() => {
				// A failed log must never take the app down with it.
			})
		}
	}

	// The two things that never reach console.error on their own.
	window.addEventListener('error', (event) => {
		console.error(`uncaught: ${describe(event.error ?? event.message)}`)
	})
	window.addEventListener('unhandledrejection', (event) => {
		console.error(`unhandled rejection: ${describe(event.reason)}`)
	})

	// Proves the pipe on every run: a log file with no webview line means this
	// never reached the Rust side.
	console.info('webview started')
}
