import {
	invoke as tauriInvoke,
	type InvokeArgs,
	type InvokeOptions,
} from '@tauri-apps/api/core'

type GlobalTauri = { core: { invoke: typeof tauriInvoke } }

/**
 * Tauri's `invoke`, looked up on `window.__TAURI__` at each call in dev. The
 * MCP bridge's IPC monitor can only wrap that global: Tauri defines the
 * `__TAURI_INTERNALS__.invoke` underneath as non-writable. A release build
 * calls the import directly.
 */
export function invoke<T>(
	cmd: string,
	...rest: [args?: InvokeArgs, options?: InvokeOptions]
): Promise<T> {
	const global = import.meta.env.DEV
		? (window as { __TAURI__?: GlobalTauri }).__TAURI__
		: undefined
	return (global?.core.invoke ?? tauriInvoke)<T>(cmd, ...rest)
}
