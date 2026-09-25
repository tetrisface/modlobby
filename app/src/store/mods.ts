import { createSignal } from 'solid-js'
import type { MutatorView } from '../ipc/bindings/MutatorView'
import {
	type ModSet,
	type Pick,
	readSets,
	remember,
	writeSets,
} from '../lib/mods'
import { localStore } from '../lib/resize'

/**
 * The draft of what a room should load, and the sets rooms have loaded.
 *
 * The draft lives here rather than in the pane because the pane is one face
 * of three: switching to Setup and back should not lose what was being put
 * together. It belongs to one room, by id, so a draft never follows you into
 * the next room.
 */

const [draft, setDraftHeld] = createSignal<{
	room: number
	picks: Pick[]
} | null>(null)

export function draftFor(room: number): Pick[] | null {
	const held = draft()
	return held?.room === room ? held.picks : null
}

export function setDraft(room: number, picks: Pick[] | null) {
	setDraftHeld(picks ? { room, picks } : null)
}

const [sets, setSets] = createSignal<ModSet[]>(readSets(localStore()))

/** What rooms loaded while you were in them, newest first. */
export const modSets = sets

/** Notes what a room loads, whenever the runtime reports it. */
export function rememberSet(loaded: readonly MutatorView[]) {
	const next = remember(sets(), loaded, new Date().toISOString())
	if (next.length === sets().length && next[0] === sets()[0]) return
	setSets(next)
	writeSets(localStore(), next)
}

export function forgetSet(at: string) {
	const next = sets().filter((set) => set.at !== at)
	setSets(next)
	writeSets(localStore(), next)
}
