import { Show } from 'solid-js'
import { api, describeError } from '../ipc/client'
import { pushNotice } from '../store/chat'
import { lobby } from '../store/lobby'

/**
 * The room's vote, scraped from what the host says. `!vote` is open to any
 * player or spectator (`commands.conf` `[vote]`), unlike calling one.
 */
export function VoteBar() {
  const vote = () => lobby.myBattle?.vote ?? null

  async function cast(choice: 'y' | 'n' | 'b') {
    try {
      await api.vote(choice)
    } catch (error) {
      pushNotice('warning', describeError(error))
    }
  }

  return (
    <Show when={vote()}>
      {(v) => (
        <div class='vote-bar'>
          <div class='vote-what'>
            <strong>{v().by ?? 'someone'}</strong> called a vote:{' '}
            <code>{v().command}</code>
            <Show when={v().proposal.type === 'setOption'}>
              <span class='muted'> (a modoption change)</span>
            </Show>
          </div>
          <div class='vote-tally'>
            <span class='yes'>
              y {v().yes}/{v().yesNeeded}
            </span>
            <span class='no'>
              n {v().no}/{v().noNeeded}
            </span>
            <Show when={v().remainingSecs > 0}>
              <span class='muted'>{v().remainingSecs}s</span>
            </Show>
          </div>
          <div class='vote-buttons'>
            <button onClick={() => cast('y')}>Yes</button>
            <button onClick={() => cast('n')}>No</button>
            <button onClick={() => cast('b')}>Blank</button>
          </div>
        </div>
      )}
    </Show>
  )
}
