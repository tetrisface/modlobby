import { beforeEach, describe, expect, it } from 'vitest'
import type { ChatLine } from '../ipc/bindings/ChatLine'
import {
  chat,
  clearChat,
  closePrivate,
  openPrivates,
  privateRoom,
  pushLine,
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
