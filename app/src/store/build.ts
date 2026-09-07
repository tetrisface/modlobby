import { createSignal } from 'solid-js'
import type { VersionView } from '../ipc/bindings/VersionView'

/** What this build is, read once at startup. */
export const [build, setBuild] = createSignal<VersionView | null>(null)

/**
 * Whether this build may talk to the lobby server.
 *
 * False on macOS, where the only engine that exists is a third-party Apple
 * Silicon build whose author has asked that it not be pointed at Beyond All
 * Reason's community servers until they approve it. That build disables online
 * play by neutering Chobby's server address, which modlobby never reads — so
 * nothing about the engine stops us and honouring it is ours to do.
 *
 * Optimistic until the answer arrives: the call is one local IPC round trip
 * away, and every platform but one says yes.
 */
export const online = (): boolean => build()?.online ?? true
