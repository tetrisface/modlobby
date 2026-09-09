import { createEffect, createSignal } from 'solid-js'
import { api, describeError } from '../ipc/client'
import { pushNotice } from '../store/chat'
import { over } from '../store/overlay'

/**
 * The two ways to stop playing, wherever the lobby is drawn while a game
 * runs: in the room card while our engine is up, and in the corner of the
 * pages that have no room card.
 *
 * Two clicks for anything that ends a game in progress. The first arms it and
 * the second does it, and arming one disarms the other -- so a mis-click never
 * ends a match, and never quits the lobby. The doubt resets whenever the
 * overlay comes or goes: an "End the game?" left armed in the room card would
 * otherwise still be one click from ending a game the next time the lobby is
 * raised.
 *
 * A fragment rather than a box: the buttons take the column or the row of
 * whoever draws them.
 */
export function GameActions() {
  const [confirming, setConfirming] = createSignal<'leave' | 'quit' | null>(
    null,
  )
  createEffect(() => {
    over()
    setConfirming(null)
  })

  function guarded(which: 'leave' | 'quit', run: () => void) {
    if (confirming() !== which) {
      setConfirming(which)
      return
    }
    setConfirming(null)
    run()
  }

  return (
    <>
      <button
        class='quit'
        title='Ends the game and leaves you here in the lobby'
        onClick={() =>
          guarded(
            'leave',
            () =>
              void api
                .stopGame()
                .catch((error) => pushNotice('warning', describeError(error))),
          )
        }
      >
        {confirming() === 'leave' ? 'End the game?' : 'Leave game'}
      </button>
      <button
        class='quit'
        title='Ends the game and closes modlobby'
        onClick={() => guarded('quit', () => void api.quitAll())}
      >
        {confirming() === 'quit' ? 'Quit everything?' : 'Quit'}
      </button>
    </>
  )
}
