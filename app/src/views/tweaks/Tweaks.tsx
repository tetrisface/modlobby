import { Show, onMount } from 'solid-js'
import { Portal } from 'solid-js/web'
import { describeError } from '../../ipc/client'
import { pushNotice } from '../../store/chat'
import { tweakspaceFor } from '../../store/tweakspaceInstance'
import { useRoom } from '../room/model'
import { Workspace } from './Workspace'

/**
 * The tweak editor, where it was opened -- under a settings row, or as the
 * drafts editor with its list -- or over the whole window.
 *
 * The workspace itself is mounted in exactly one of the two places at a time.
 * A second Monaco on the same model would work, but two cursors in one
 * document is a thing nobody asked for, and the pane has better uses for the
 * space than a mirror.
 */
export function Tweaks(props: { drafts?: boolean; onClose?: () => void }) {
	const space = tweakspaceFor(useRoom())
	onMount(() => {
		void space
			.refreshDrafts()
			.catch((error) =>
				pushNotice('warning', `drafts: ${describeError(error)}`),
			)
	})

	const workspace = () => (
		<Workspace drafts={props.drafts ?? false} onClose={props.onClose} />
	)

	return (
		<>
			<Show
				when={!space.ws.fullscreen}
				fallback={
					<div class='tweaks-away'>
						<p class='muted setup-empty'>The editor is filling the window.</p>
						<button onClick={() => space.setFullscreen(false)}>
							Bring it back here
						</button>
					</div>
				}
			>
				{workspace()}
			</Show>
			<Show when={space.ws.fullscreen}>
				<Portal>
					<div class='tweak-full'>{workspace()}</div>
				</Portal>
			</Show>
		</>
	)
}
