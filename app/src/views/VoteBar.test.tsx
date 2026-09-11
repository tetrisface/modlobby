import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { createSignal } from 'solid-js'
import { afterEach, describe, expect, test, vi } from 'vitest'
import type { VoteView } from '../ipc/bindings/VoteView'
import { fakeRoom, myBattle } from './room/fixture'
import { RoomProvider } from './room/model'
import { VoteBar } from './VoteBar'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => null),
  convertFileSrc: (path: string, scheme: string) => `${scheme}://${path}`,
}))

const vote = (over: Partial<VoteView> = {}): VoteView => ({
  command: 'resign [Cookie]swiftplant TEAM',
  by: '[Cookie]swiftplant',
  proposal: { type: 'other' },
  yes: 1,
  yesNeeded: 8,
  no: 0,
  noNeeded: 8,
  remainingSecs: 24,
  ...over,
})

/** The bar over a room whose vote the test can change under it. */
function open(first: VoteView | null) {
  const [current, setVote] = createSignal(first)
  const model = fakeRoom({ my: () => myBattle({ vote: current() }) })
  const rendered = render(() => (
    <RoomProvider value={model}>
      <VoteBar teams={2} />
    </RoomProvider>
  ))
  return { ...rendered, setVote }
}

/** Lets awaited answers reach the component and the DOM. */
async function settle() {
  for (let turn = 0; turn < 8; turn++) await Promise.resolve()
}

const ballots = (container: HTMLElement) =>
  [...container.querySelectorAll('.vote-cast button')] as HTMLButtonElement[]

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('VoteBar', () => {
  test('draws nothing while there is no vote', () => {
    const { container } = open(null)
    expect(container.querySelector('.vote-bar')).toBeNull()
  })

  test('says who asked what, with the three answers in their colours', () => {
    const { container } = open(vote())
    expect(container.querySelector('.vote-what')?.textContent).toBe(
      '[Cookie]swiftplant called a vote: resign [Cookie]swiftplant TEAM',
    )
    const [yes, no, blank] = ballots(container)
    expect(yes?.className).toContain('yes')
    expect(no?.className).toContain('no')
    expect(blank?.className).toContain('blank')
  })

  test('each side fills with its count towards what it needs', () => {
    const { container } = open(
      vote({ yes: 2, yesNeeded: 8, no: 1, noNeeded: 4 }),
    )
    const [yes, no, blank] = ballots(container)
    expect(yes?.textContent).toBe('Yes2/8')
    expect(yes?.style.getPropertyValue('--fill')).toBe('25%')
    expect(no?.textContent).toBe('No1/4')
    expect(no?.style.getPropertyValue('--fill')).toBe('25%')
    // Blank is not a side: no count, no fill.
    expect(blank?.textContent).toBe('Blank')
    expect(blank?.style.getPropertyValue('--fill')).toBe('0%')
  })

  test('a side the host is not counting shows its count alone', () => {
    const { container } = open(vote({ yes: 1, yesNeeded: 0 }))
    expect(ballots(container)[0]?.textContent).toBe('Yes1')
  })

  test('casting sends the choice and marks the answer, until the next vote', async () => {
    const { container, setVote } = open(vote())
    const [yes, no] = ballots(container)
    fireEvent.click(no!)
    await settle()
    expect(vi.mocked(invoke)).toHaveBeenCalledWith('vote', { choice: 'n' })
    expect(no?.className).toContain('on')
    expect(yes?.className).not.toContain('on')

    // The same command, called by somebody else: a fresh ballot.
    setVote(vote({ by: 'someone' }))
    expect(ballots(container)[1]?.className).not.toContain('on')
  })

  test('shows the seconds left and how much of the vote is still to run', () => {
    const { container } = open(vote({ remainingSecs: 24 }))
    expect(container.querySelector('.vote-left')?.textContent).toBe('24s')
    const bar = container.querySelector<HTMLElement>('.vote-bar')
    expect(bar?.style.getPropertyValue('--left')).toBe('1')
  })
})
