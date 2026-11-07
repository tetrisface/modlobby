import { createSignal } from 'solid-js'
import type { ListedScore } from '../ipc/bindings/ListedScore'
import { api } from '../ipc/client'
import { settings } from './settings'

/**
 * What each room in the battle list last scored, by battle list key.
 *
 * Read off the pve.bar live games list, which holds whatever someone in a
 * room last asked the service about; see `lib/listed` for what that means
 * for how fresh a number is. A store, like `store/running`, so a remounted
 * list shows its numbers at once instead of a blank column for a poll.
 */
const [listed, setListed] = createSignal<Record<string, ListedScore>>({})

export { listed }

/**
 * How often the list is asked while something shows it. A room's score
 * moves only when someone in it changes the setup, and the service's edge
 * holds the list for seconds anyway: a minute is plenty, and cheap.
 */
export const POLL_EVERY = 60_000

let listeners = 0
let timer: ReturnType<typeof setInterval> | undefined
let asking = false

/**
 * The panel's own switch. On until the settings have arrived, as there;
 * Rust checks it again before anything leaves.
 */
function enabled(): boolean {
	return settings()?.play.pveStats ?? true
}

async function poll() {
	// Read here rather than at `watchListedScores`, so flipping the setting
	// takes effect on the next poll without reopening the page.
	if (!enabled()) {
		setListed({})
		return
	}
	// Nobody is looking at a hidden window; coming back asks at once.
	if (document.visibilityState === 'hidden' || asking) return
	asking = true
	try {
		setListed(await api.pveListedScores())
	} catch {
		// The numbers already shown stay: older is better than none, and
		// the tooltip says how old.
	} finally {
		asking = false
	}
}

function onVisibility() {
	if (document.visibilityState === 'visible') void poll()
}

/**
 * Starts asking what rooms scored, for as long as the returned function has
 * not been called: what a page that shows them does on mount.
 */
export function watchListedScores(): () => void {
	listeners += 1
	if (listeners === 1) {
		void poll()
		timer = setInterval(() => void poll(), POLL_EVERY)
		document.addEventListener('visibilitychange', onVisibility)
	}
	return () => {
		listeners -= 1
		if (listeners > 0) return
		clearInterval(timer)
		document.removeEventListener('visibilitychange', onVisibility)
	}
}
