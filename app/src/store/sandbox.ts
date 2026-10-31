import { createSignal } from 'solid-js'
import { api, describeError } from '../ipc/client'

/**
 * Whether this machine can sandbox a modded game: the reason it cannot,
 * `null` when it can, `undefined` until the runtime has been asked. Asked
 * once per run; the answer does not change while the app runs.
 */
export const [sandboxReason, setSandboxReason] = createSignal<
	string | null | undefined
>(undefined)

let asked: Promise<void> | null = null

/** Asks the runtime once; later calls wait on the same answer. */
export function loadSandboxReason(): Promise<void> {
	if (sandboxReason() !== undefined) return Promise.resolve()
	if (asked) return asked
	asked = (async () => {
		try {
			setSandboxReason(await api.sandboxUnavailable())
		} catch (error) {
			// Unknown is treated as a machine with a sandbox: the launch says
			// otherwise, with the way out, if it turns out there is none.
			console.warn(`sandbox: ${describeError(error)}`)
			setSandboxReason(null)
		} finally {
			asked = null
		}
	})()
	return asked
}
