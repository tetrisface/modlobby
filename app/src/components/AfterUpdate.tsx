import { Show, createSignal, onMount, type ParentProps } from 'solid-js'
import { boot } from '../ipc/boot'
import { api, describeError } from '../ipc/client'
import { pushNotice } from '../store/chat'
import { resumeUpdate } from '../store/update'

/**
 * Holds the app back until the start's update has had its turn.
 *
 * A download an earlier run kept is installed ahead of the app, and that ends
 * in a restart: drawn first, the app would vanish a moment after it appeared.
 * So nothing inside is drawn until that is settled. Rust says with the page
 * whether anything was kept (`boot`), so an ordinary start draws the app at
 * once; a reloaded page has to ask.
 *
 * What is drawn meanwhile names the version going in, and only once there is
 * known to be one: on an ordinary start a line of text would only flash.
 */
export function AfterUpdate(props: ParentProps) {
	const start = boot()
	const [settled, setSettled] = createSignal(start?.keptUpdate === null)
	const [installing, setInstalling] = createSignal(start?.keptUpdate ?? null)
	onMount(async () => {
		if (settled()) return
		try {
			const kept = start ? start.keptUpdate : await api.keptUpdate()
			if (kept !== null) {
				setInstalling(kept)
				await resumeUpdate()
			}
		} catch (error) {
			pushNotice('error', describeError(error))
		}
		setSettled(true)
	})
	return (
		<Show
			when={settled()}
			fallback={
				<Show when={installing()}>
					{(version) => (
						// The window has no title bar, and for now no nav to drag it by.
						<p class='installing muted' data-tauri-drag-region>
							Installing version {version()}…
						</p>
					)}
				</Show>
			}
		>
			{props.children}
		</Show>
	)
}
