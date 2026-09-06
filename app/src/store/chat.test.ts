import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { ChatLine } from '../ipc/bindings/ChatLine'
import {
  chat,
  clearChat,
  closePrivate,
  openPrivates,
  holdNotices,
  privateRoom,
  pushLine,
  pushNotice,
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
