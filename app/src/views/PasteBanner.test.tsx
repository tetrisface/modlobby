import { render } from '@solidjs/testing-library'
import { afterEach, describe, expect, test, vi } from 'vitest'
import { emptyLobby, setLobby } from '../store/lobby'
import { PasteBanner } from './PasteBanner'

afterEach(() => {
  setLobby(emptyLobby())
  vi.useRealTimers()
})

describe('the paste banner', () => {
  test('nothing is drawn while nothing is being pasted', () => {
    const { container } = render(() => <PasteBanner />)
    expect(container.querySelector('.paste-banner')).toBeNull()
  })

  test('a running paste follows the host, weighted by bytes', () => {
    setLobby('paste', {
      state: 'running',
      total: 27,
      sent: 27,
      commands: 20,
      applied: 5,
      skipped: 156,
      work: 200_000,
      done: 80_000,
    })
    const { container } = render(() => <PasteBanner />)
    const text = container.querySelector('.paste-text')?.textContent
    expect(text).toContain('Applying commands 5/20')
    expect(text).toContain('156 already set')
    expect(text).not.toContain('left')
    expect(
      (container.querySelector('.paste-bar > div') as HTMLElement).style.width,
    ).toBe('40%')
    expect(container.querySelector('button.paste-cancel')).not.toBeNull()
  })

  test('a finished paste lingers, then goes', () => {
    vi.useFakeTimers()
    setLobby('paste', {
      state: 'done',
      total: 0,
      commands: 0,
      applied: 0,
      skipped: 27,
      cancelled: false,
    })
    const { container } = render(() => <PasteBanner />)
    expect(container.querySelector('.paste-text')?.textContent).toContain(
      'already has all 27 settings',
    )
    expect(container.querySelector('button.paste-cancel')).toBeNull()
    vi.advanceTimersByTime(6000)
    expect(container.querySelector('.paste-banner')).toBeNull()
  })

  test('a cancelled paste says how far it got', () => {
    setLobby('paste', {
      state: 'done',
      total: 27,
      commands: 20,
      applied: 7,
      skipped: 0,
      cancelled: true,
    })
    const { container } = render(() => <PasteBanner />)
    expect(container.querySelector('.paste-text')?.textContent).toContain(
      'cancelled: the host answered 7 of 20',
    )
  })
})
