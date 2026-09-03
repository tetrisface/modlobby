import { render } from '@solidjs/testing-library'
import { describe, expect, test } from 'vitest'
import type { BattleStatusView } from '../ipc/bindings/BattleStatusView'
import type { UserView } from '../ipc/bindings/UserView'
import type { UserStatusView } from '../ipc/bindings/UserStatusView'
import type { Skill } from '../lib/skill'
import { PlayerRow, SpectatorRow } from './PlayerRow'

const status = (over: Partial<UserStatusView> = {}): UserStatusView => ({
  inGame: false,
  away: false,
  rank: 0,
  moderator: false,
  bot: false,
  ...over,
})

const battle = (over: Partial<BattleStatusView> = {}): BattleStatusView => ({
  ready: true,
  team: 0,
  allyTeam: 0,
  player: true,
  handicap: 0,
  sync: 'synced',
  side: 0,
  ...over,
})

const user = (over: Partial<UserView> = {}): UserView => ({
  name: 'DrDandy',
  country: 'SE',
  userId: 1,
  lobbyClient: 'modlobby',
  status: status(),
  battleStatus: battle(),
  battleId: 1,
  ...over,
})

const skill = (over: Partial<Skill> = {}): Skill => ({
  value: 23.4,
  origin: 'plugin',
  sigma: 0.9,
  ...over,
})

/** The `<use href>` of every icon in the row, left to right. */
function icons(container: HTMLElement): string[] {
  return [...container.querySelectorAll('use')].map(
    (use) => use.getAttribute('href') ?? '',
  )
}

describe('a player row', () => {
  test('draws Chobby six columns in Chobby order', () => {
    const { container } = render(() => (
      <PlayerRow user={user()} skill={skill()} me={false} />
    ))

    // status, rank, faction. The flag is a styled span, the skill is text.
    expect(icons(container)).toEqual(['#st-ready', '#chev1', '#side-armada'])
    expect(container.querySelector('.flag')?.className).toContain('fi-se')
    expect(container.querySelector('.skill')?.textContent).toBe('23')
    expect(container.querySelector('.pname')?.textContent).toBe('DrDandy')
  })

  test('the status ladder puts in-game above sync above ready', () => {
    const ladder: Array<[UserView, string]> = [
      [
        user({
          status: status({ inGame: true }),
          battleStatus: battle({ ready: false, sync: 'unsynced' }),
        }),
        '#st-swords',
      ],
      [user({ battleStatus: battle({ sync: 'unsynced' }) }), '#st-sync'],
      [user({ battleStatus: battle({ ready: false }) }), '#st-unready'],
      [user(), '#st-ready'],
    ]

    for (const [who, expected] of ladder) {
      const { container, unmount } = render(() => (
        <PlayerRow user={who} skill={null} me={false} />
      ))
      expect(icons(container)[0]).toBe(expected)
      unmount()
    }
  })

  test('rank 5 is one solid chevron, rank 8 is four', () => {
    for (const [rank, id] of [
      [0, '#chev1'],
      [3, '#chev4'],
      [4, '#chev1-solid'],
      [7, '#chev4-solid'],
    ] as const) {
      const { container, unmount } = render(() => (
        <PlayerRow
          user={user({ status: status({ rank }) })}
          skill={null}
          me={false}
        />
      ))
      expect(icons(container)[1]).toBe(id)
      unmount()
    }
  })

  test('a moderator keeps the rank and gains the shield', () => {
    const { container } = render(() => (
      <PlayerRow
        user={user({ status: status({ rank: 6, moderator: true }) })}
        skill={null}
        me={false}
      />
    ))
    expect(icons(container)).toEqual([
      '#st-ready',
      '#chev3-solid',
      '#rank-shield',
      '#side-armada',
    ])
    expect(container.querySelector('.rank use')?.getAttribute('mask')).toBe(
      'url(#rank-shield-cut)',
    )
  })

  test('the boss wears a crown after the name, an away player a snooze', () => {
    const { container } = render(() => (
      <PlayerRow
        user={user({ status: status({ away: true }) })}
        skill={null}
        me={false}
        boss={true}
      />
    ))
    expect(icons(container)).toEqual([
      '#st-ready',
      '#chev1',
      '#side-armada',
      '#mark-boss',
      '#mark-away',
    ])
  })

  test('an uncertain rating reads ?? and a confident one is bright', () => {
    const unrated = render(() => (
      <PlayerRow user={user()} skill={skill({ sigma: 6.81 })} me={false} />
    ))
    expect(unrated.container.querySelector('.skill')?.textContent).toBe('??')
    unrated.unmount()

    const dim = render(() => (
      <PlayerRow user={user()} skill={skill({ sigma: 3.2 })} me={false} />
    ))
    expect(dim.container.querySelector('.skill')?.className).toContain('tier3')
    dim.unmount()
  })

  test('faction follows the side bits', () => {
    for (const [side, id] of [
      [0, '#side-armada'],
      [1, '#side-cortex'],
      [3, '#side-legion'],
    ] as const) {
      const { container, unmount } = render(() => (
        <PlayerRow
          user={user({ battleStatus: battle({ side }) })}
          skill={null}
          me={false}
        />
      ))
      expect(icons(container)[2]).toBe(id)
      unmount()
    }
  })

  test('no colour is applied to a row beyond the faction mark', () => {
    const { container } = render(() => (
      <PlayerRow user={user()} skill={skill()} me={false} />
    ))
    // Team colours are not knowable before the game starts, so no element in
    // the row may carry one.
    expect(container.querySelector('[class*="team-"]')).toBeNull()
    expect(container.querySelector('.player')?.getAttribute('style')).toBeNull()
  })

  test('an unknown country falls back rather than guessing', () => {
    const { container } = render(() => (
      <PlayerRow user={user({ country: '??' })} skill={null} me={false} />
    ))
    expect(container.querySelector('.flag')?.className).toContain('unknown')
  })
})

describe('a spectator row', () => {
  test('shows neither status nor faction, as Chobby hides both', () => {
    const { container } = render(() => (
      <SpectatorRow
        user={user({ battleStatus: battle({ player: false }) })}
        me={true}
      />
    ))
    expect(icons(container)).toEqual(['#chev1'])
    expect(container.querySelector('.pname')?.classList.contains('me')).toBe(
      true,
    )
  })

  test('a bossing spectator still wears the crown', () => {
    const { container } = render(() => (
      <SpectatorRow
        user={user({ battleStatus: battle({ player: false }) })}
        me={false}
        boss={true}
      />
    ))
    expect(icons(container)).toEqual(['#chev1', '#mark-boss'])
  })

  test('an autohost gets the bot mark instead of a rank', () => {
    const { container } = render(() => (
      <SpectatorRow
        user={user({ name: 'Host[US4][000]', status: status({ bot: true }) })}
        me={false}
      />
    ))
    expect(icons(container)).toEqual(['#rank-bot'])
  })
})
