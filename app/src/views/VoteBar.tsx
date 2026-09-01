import { Show, createMemo } from 'solid-js'
import { BoxDiff } from '../components/BoxDiff'
import { isBoxKey } from '../lib/boxes'
import { api, describeError } from '../ipc/client'
import { pushNotice } from '../store/chat'
import { lobby } from '../store/lobby'

/**
 * The room's vote, scraped from what the host says. `!vote` is open to any
 * player or spectator (`commands.conf` `[vote]`), unlike calling one.
 *
 * `teams` is the room's ally-team count, which selects which arrangement out of
 * a map's set the boxes would resolve to. It is passed in rather than derived
 * again here: the minimap a few centimetres up the page draws the same boxes,
 * and the two disagreeing about how many teams there are would show two
 * different answers to the same question.
 */
export function VoteBar(props: { teams: number }) {
  const vote = () => lobby.myBattle?.vote ?? null

  const room = createMemo(() => {
    const id = lobby.myBattle?.id
    return id === undefined ? undefined : lobby.battles[id]
  })

  /**
   * A vote that would move the start boxes, and what it would move them to.
   *
   * Worth singling out because the value is base64url(zlib(json)): the vote
   * line shows a wall of characters that tells nobody anything.
   */
  const boxProposal = createMemo(() => {
    const proposal = vote()?.proposal
    if (proposal?.type !== 'setOption') return null
    if (!isBoxKey(proposal.key)) return null
    const current =
      lobby.myBattle?.scriptTags[`game/modoptions/${proposal.key}`] ?? ''
    return { current, proposed: proposal.value }
  })

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
          <Show when={boxProposal()}>
            {(change) => (
              <BoxDiff
                current={change().current}
                proposed={change().proposed}
                teams={props.teams}
                mapName={room()?.mapName ?? ''}
              />
            )}
          </Show>
        </div>
      )}
    </Show>
  )
}
