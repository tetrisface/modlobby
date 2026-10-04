import { Show, createSignal, onMount, type ParentProps } from 'solid-js'
import { api, describeError } from '../ipc/client'
import { pushNotice } from '../store/chat'
import { resumeUpdate } from '../store/update'

/**
 * Holds the app back until the start's update has had its turn.
 *
 * A download an earlier run kept is installed ahead of the app, and that ends
 * in a restart. Drawn first, the app would vanish a moment after it appeared;
 * logged in first, the restart would spend a second login on the server's
 * count. So nothing inside is drawn, and with it nothing logs in, until Rust
 * has answered. With nothing kept that is one round trip.
 *
 * What is drawn meanwhile names the version going in, and only once Rust has
 * said there is one: on an ordinary start a line of text would only flash.
 */
export function AfterUpdate(props: ParentProps) {
	const [settled, setSettled] = createSignal(false)
	const [installing, setInstalling] = createSignal<string | null>(null)
	onMount(async () => {
		try {
			const kept = await api.keptUpdate()
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
