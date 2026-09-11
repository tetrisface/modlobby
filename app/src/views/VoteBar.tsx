import { Show, createEffect, createMemo, createSignal, on } from 'solid-js'
import { BoxDiff } from '../components/BoxDiff'
import { isBoxKey } from '../lib/boxes'
import { createCountdown, createSpan, share, tally, voteKey } from '../lib/vote'
import { api, describeError, type VoteChoice } from '../ipc/client'
import { pushNotice } from '../store/chat'
import { useRoom } from './room/model'

/**
 * The room's vote, scraped from what the host says. `!vote` is open to any
 * player or spectator (`commands.conf` `[vote]`), unlike calling one.
 *
 * Everything about the vote sits together at the left -- what was asked, the
 * three answers with their side's count filling each one, the seconds left --
 * so a glance takes it in, and the time left drains along the bar's foot so
 * that everyone in the room can see it running without being shouted at.
 * Most votes are somebody else's business; this one row is all they get.
 *
 * `teams` is the room's ally-team count, which selects which arrangement out of
 * a map's set the boxes would resolve to. It is passed in rather than derived
 * again here: the minimap a few centimetres up the page draws the same boxes,
 * and the two disagreeing about how many teams there are would show two
 * different answers to the same question.
 */
export function VoteBar(props: { teams: number }) {
  const room = useRoom()
  const vote = () => room.my()?.vote ?? null
  const key = createMemo(() => {
    const v = vote()
    return v === null ? null : voteKey(v)
  })

  const said = () => vote()?.remainingSecs ?? 0
  const left = createCountdown(said)
  const span = createSpan(said, key)
  /** How much of the vote's time is still to run, for the line at the foot. */
  const running = () => (span() > 0 ? left() / span() : 0)

  /** What we cast, so the button shows it. Forgotten with the vote. */
  const [mine, setMine] = createSignal<VoteChoice | null>(null)
  createEffect(on(key, () => setMine(null)))

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
      room.my()?.scriptTags[`game/modoptions/${proposal.key}`] ?? ''
    return { current, proposed: proposal.value }
  })

  async function cast(choice: VoteChoice) {
    try {
      await api.vote(choice)
      setMine(choice)
    } catch (error) {
      pushNotice('warning', describeError(error))
    }
  }

  return (
    <Show when={vote()}>
      {(v) => (
        <div class='vote-bar' style={{ '--left': String(running()) }}>
          <div class='vote-what'>
            <strong>{v().by ?? 'someone'}</strong> called a vote:{' '}
            <code>{v().command}</code>
            <Show when={v().proposal.type === 'setOption'}>
              <span class='muted'> (a modoption change)</span>
            </Show>
          </div>
          <div class='vote-cast' role='group' aria-label='Your vote'>
            <Ballot
              choice='y'
              label='Yes'
              have={v().yes}
              needed={v().yesNeeded}
              mine={mine()}
              cast={cast}
            />
            <Ballot
              choice='n'
              label='No'
              have={v().no}
              needed={v().noNeeded}
              mine={mine()}
              cast={cast}
            />
            <Ballot choice='b' label='Blank' mine={mine()} cast={cast} />
          </div>
          <Show when={left() > 0}>
            <span class='vote-left' title='Seconds until the vote closes'>
              {left()}s
            </span>
          </Show>
          <Show when={boxProposal()}>
            {(change) => (
              <BoxDiff
                current={change().current}
                proposed={change().proposed}
                teams={props.teams}
                mapName={room.battle()?.mapName ?? ''}
              />
            )}
          </Show>
          <div class='vote-clock' aria-hidden='true' />
        </div>
      )}
    </Show>
  )
}

/** The class each answer is coloured by. */
const SIDE: Record<VoteChoice, string> = { y: 'yes', n: 'no', b: 'blank' }

/**
 * One answer, filled to its side's share of what is needed. Blank carries no
 * count: SPADS reports none, and it is not a side -- it says you were here
 * and abstain, which lowers what the sides need.
 */
function Ballot(props: {
  choice: VoteChoice
  label: string
  have?: number
  needed?: number
  mine: VoteChoice | null
  cast: (choice: VoteChoice) => void
}) {
  const counted = () => props.have !== undefined && props.needed !== undefined
  const fill = () => Math.round(share(props.have ?? 0, props.needed ?? 0) * 100)
  const title = () =>
    counted()
      ? `Vote ${props.label.toLowerCase()}: ${tally(props.have ?? 0, props.needed ?? 0)} needed to pass`
      : 'Abstain, but count as having voted'

  return (
    <button
      type='button'
      class={SIDE[props.choice]}
      classList={{ on: props.mine === props.choice }}
      style={{ '--fill': `${fill()}%` }}
      title={title()}
      aria-pressed={props.mine === props.choice}
      onClick={() => props.cast(props.choice)}
    >
      {props.label}
      <Show when={counted()}>
        <span class='tally'>{tally(props.have ?? 0, props.needed ?? 0)}</span>
      </Show>
    </button>
  )
}
