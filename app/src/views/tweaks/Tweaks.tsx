import { Show, onMount } from 'solid-js'
import { Portal } from 'solid-js/web'
import { describeError } from '../../ipc/client'
import { slotId } from '../../lib/tweakspace'
import { pushNotice } from '../../store/chat'
import { tweakspace as space } from '../../store/tweakspaceInstance'
import { Workspace } from './Workspace'

/**
 * The tweak editor, where the pane puts it -- or over the whole window.
 *
 * The workspace itself is mounted in exactly one of the two places at a time.
 * A second Monaco on the same model would work, but two cursors in one
 * document is a thing nobody asked for, and the pane has better uses for the
 * space than a mirror.
 */
export function Tweaks(props: { initial?: string }) {
  onMount(() => {
    if (props.initial) space.open(slotId(props.initial))
    void space
      .refreshDrafts()
      .catch((error) =>
        pushNotice('warning', `drafts: ${describeError(error)}`),
      )
  })

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
        <Workspace />
      </Show>
      <Show when={space.ws.fullscreen}>
        <Portal>
          <div class='tweak-full'>
            <Workspace />
          </div>
        </Portal>
      </Show>
    </>
  )
}
