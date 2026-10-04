import { createEffect, createRoot, createSignal, on } from 'solid-js'
import { keepTrying } from '../ipc/alerts'
import { api } from '../ipc/client'
import { lobby } from './lobby'

/**
 * Whether our engine is up without a window yet.
 *
 * The runtime says `running` at spawn, and the engine takes seconds to open a
 * window after that: long enough for "Back to game" to be a button that hides
 * the lobby over nothing. Windows is asked for the window, the way the alerts
 * ask it for a taskbar entry to flash, and the answer is kept here for the
 * room card to wait on.
 */

/** How long a game gets to open its window before the card stops waiting. */
export const LAUNCH_WAIT_MS = 20_000
const POLL_MS = 250

/**
 * How long the card keeps waiting after the window is found.
 *
 * `IsWindowVisible` is true as soon as the engine's window has `WS_VISIBLE`,
 * which is close to a second before it has presented a frame and is really on
 * screen. Nothing we can ask from outside the process says when that frame
 * lands, so the wait is padded rather than guessed at: stripes over a window
 * already drawn cost a moment, stripes that stop over a window not drawn yet
 * leave the lobby offering "Back to game" with nothing to go back to.
 */
const SETTLE_MS = 1_000

/** The pid whose window has been seen, or whose wait ran out. */
export const [windowSeen, setWindowSeen] = createSignal<number | null>(null)

/** Our engine is running and has no window yet: nothing to go back to. */
export function launching(): boolean {
	const engine = lobby.engine
	// A reaped child has no pid, and so no window to wait for.
	if (engine.state !== 'running' || engine.pid === null) return false
	return windowSeen() !== engine.pid
}

/**
 * Follows the engine: each new pid is watched for its window until it has one
 * or the wait is up. Returns what stops following. `hasWindow`, `wait`,
 * `every` and `settle` are parameters so a test can run it on a fake.
 */
export function watchLaunch(
	hasWindow: () => Promise<boolean> = api.engineHasWindow,
	wait = LAUNCH_WAIT_MS,
	every = POLL_MS,
	settle = SETTLE_MS,
): () => void {
	return createRoot((dispose) => {
		createEffect(
			on(
				() => (lobby.engine.state === 'running' ? lobby.engine.pid : null),
				(pid) => {
					setWindowSeen(null)
					if (pid === null) return
					const done = () => setWindowSeen(pid)
					void keepTrying(hasWindow, wait, every).then(
						// A window that was found gets its settle; one that never
						// came, or a question that could not be answered, has kept
						// the card waiting long enough already.
						(found) => {
							if (found) setTimeout(done, settle)
							else done()
						},
						done,
					)
				},
			),
		)
		return dispose
	})
}
