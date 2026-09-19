import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { ChatLine } from '../ipc/bindings/ChatLine'
import {
  byActivity,
  chat,
  clearChat,
  closePrivate,
  openPrivates,
  holdNotices,
  privateRoom,
  pushLine,
  pushNotice,
  unreadTotal,
  watchRoom,
} from './chat'

function said(room: string, text: string, mention = false): ChatLine {
  return {
    seq: 1,
    room,
    from: 'someone',
    text,
    kind: 'private',
    mention,
    at: 0,
  }
}

describe('closing a private conversation', () => {
  beforeEach(() => {
    clearChat()
    // Somewhere else, so the room under test accrues its unread count.
    watchRoom('#battle')
  })

  it('takes the room out of every map it was in', () => {
    const room = privateRoom('someone')
    pushLine(said(room, 'hello', true))
    expect(chat.rooms[room]).toHaveLength(1)
    expect(chat.unread[room]).toBe(1)
    expect(chat.named[room]).toBe(true)

    closePrivate(room)

    // Deleted, not merely emptied: an empty backlog would still list the room.
    expect(room in chat.rooms).toBe(false)
    expect(room in chat.unread).toBe(false)
    expect(room in chat.named).toBe(false)
    expect(openPrivates()).not.toContain(room)
  })

  it('leaves a channel alone — closing one is a server matter', () => {
    pushLine(said('#main', 'hello'))
    closePrivate('#main')
    expect(chat.rooms['#main']).toHaveLength(1)
  })

  it('opens again fresh when that person speaks next', () => {
    const room = privateRoom('someone')
    pushLine(said(room, 'first'))
    closePrivate(room)
    pushLine(said(room, 'second'))

    expect(chat.rooms[room]?.map((line) => line.text)).toEqual(['second'])
  })

  it('is harmless on a conversation that was never open', () => {
    expect(() => closePrivate(privateRoom('nobody'))).not.toThrow()
  })
})

describe('unread counts', () => {
  beforeEach(() => {
    clearChat()
    watchRoom('#battle')
  })

  it('leaves the message of the day out: it comes on every connect', () => {
    pushLine({ ...said('#server', 'Welcome to Teiserver'), kind: 'motd' })
    expect(chat.rooms['#server']).toHaveLength(1)
    expect(chat.unread['#server']).toBeUndefined()
  })

  it('still counts a broadcast', () => {
    pushLine({ ...said('#server', 'Restarting soon'), kind: 'system' })
    expect(chat.unread['#server']).toBe(1)
  })
})

describe('the count on the Chat tab', () => {
  beforeEach(() => {
    clearChat()
    watchRoom('#battle')
  })

  it('leaves a muted room out and counts the rest', () => {
    pushLine(said('main', 'hello'))
    pushLine(said('main', 'anyone?'))
    pushLine(said(privateRoom('friend'), 'hi'))
    expect(unreadTotal(['main'])).toBe(1)
    expect(unreadTotal([])).toBe(3)
  })

  it('lets a muted room back in once it names you', () => {
    pushLine(said('main', 'hello'))
    pushLine(said('main', 'hey you', true))
    expect(unreadTotal(['main'])).toBe(2)
  })
})

describe('people by activity', () => {
  beforeEach(() => clearChat())

  it('puts who you can talk to first, then who spoke last, then names', () => {
    pushLine({ ...said(privateRoom('offline'), 'late'), at: 300 })
    pushLine({ ...said(privateRoom('early'), 'first'), at: 100 })
    pushLine({ ...said(privateRoom('recent'), 'then'), at: 200 })
    const online = (name: string) => name !== 'offline'
    expect(
      ['offline', 'zed', 'early', 'recent', 'abe'].sort(byActivity(online)),
    ).toEqual(['recent', 'early', 'abe', 'zed', 'offline'])
  })
})

describe('the corner notices', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    clearChat()
  })
  afterEach(() => {
    holdNotices(false)
    vi.useRealTimers()
  })

  it('leaves on its own, so the corner is not a log', () => {
    pushNotice('info', 'something happened')
    expect(chat.notices.length).toBe(1)

    vi.advanceTimersByTime(9_500)
    expect(chat.notices.length).toBe(0)
  })

  it('stays while the pointer is in the corner', () => {
    pushNotice('info', 'read me')
    holdNotices(true)

    // Long past its span, and still there to be read.
    vi.advanceTimersByTime(30_000)
    expect(chat.notices.length).toBe(1)
  })

  it('gets the held time back rather than being pinned', () => {
    pushNotice('info', 'read me')
    holdNotices(true)
    vi.advanceTimersByTime(30_000)
    holdNotices(false)

    // Released with its whole span still ahead of it...
    vi.advanceTimersByTime(8_000)
    expect(chat.notices.length).toBe(1)

    // ...and then it goes.
    vi.advanceTimersByTime(1_500)
    expect(chat.notices.length).toBe(0)
  })

  it('holds every notice in the corner, not just the one under the pointer', () => {
    pushNotice('info', 'first')
    vi.advanceTimersByTime(8_000)
    pushNotice('warning', 'second')
    holdNotices(true)

    vi.advanceTimersByTime(30_000)
    expect(chat.notices.map((notice) => notice.text)).toEqual([
      'first',
      'second',
    ])
  })
})
