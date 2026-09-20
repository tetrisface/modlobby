import { createSignal } from 'solid-js'
import type { LanRoomView } from '../ipc/bindings/LanRoomView'
import type { Row } from '../lib/battles'
import { lobby } from '../store/lobby'
import { settings } from '../store/settings'
import { lanApi } from './api'
import { LAN, lanRows } from './lan'

/** What the network has said lately, asked every couple of seconds. */
const [heard, setHeard] = createSignal<LanRoomView[]>([])

/** How often the network is asked while something is listening. */
const POLL_MS = 2000

let listeners = 0
let timer: ReturnType<typeof setInterval> | undefined

async function poll() {
	// Switched off, nothing is asked -- and asking is what starts listening
	// on the network at all, since Rust opens the browser on the first call.
	// Read here rather than at `watchLan`, so turning it on fills the list
	// without reopening the page.
	if (!(settings()?.lan.enabled ?? false)) {
		setHeard([])
		return
	}
	try {
		setHeard(await lanApi.rooms())
	} catch {
		setHeard([])
	}
}

/**
 * Starts listening for rooms on the network, for as long as the returned
 * function has not been called: what a page that shows them does on mount.
 */
export function watchLan(): () => void {
	listeners += 1
	if (listeners === 1) {
		void poll()
		timer = setInterval(() => void poll(), POLL_MS)
	}
	return () => {
		listeners -= 1
		if (listeners === 0) clearInterval(timer)
	}
}

/** The rows the battle list adds for the network. */
export function heardRows(): Row[] {
	const session = lobby.servers[LAN]
	const mine = session?.myBattle
		? (session.battles[session.myBattle.id] ?? null)
		: null
	return lanRows(heard(), mine)
}
