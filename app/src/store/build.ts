import { createSignal } from 'solid-js'
import type { VersionView } from '../ipc/bindings/VersionView'

/** What this build is, read once at startup. */
export const [build, setBuild] = createSignal<VersionView | null>(null)

/**
 * Whether the engine here may be run against somebody's hosted game.
 *
 * False on macOS, where the only engine that exists is a third-party Apple
 * Silicon build its author asks not be used on the community servers. Rooms,
 * chat and the battle list cost those servers nothing and stay; a seat, a
 * ready flag and a launch are what go, because each of them ends in the engine
 * being started on somebody's game.
 *
 * This only shapes what is drawn. What actually holds the line is
 * `recoil::refuse_target`, at the one place the engine is ever spawned.
 *
 * Optimistic until the answer arrives: the call is one local IPC round trip
 * away, and every platform but one says yes.
 */
export const playsOnline = (): boolean => build()?.playsOnline ?? true
