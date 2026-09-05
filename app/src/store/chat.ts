import { createStore, produce } from 'solid-js/store'
import type { ChannelView } from '../ipc/bindings/ChannelView'
import type { ChannelSummaryView } from '../ipc/bindings/ChannelSummaryView'
import type { ChatLine } from '../ipc/bindings/ChatLine'
import type { NoticeLevel } from '../ipc/bindings/NoticeLevel'

export type Notice = { seq: number; level: NoticeLevel; text: string }

/** The room key for the battle we are in; `lobby-ui` writes the same string. */
export const BATTLE_ROOM = '#battle'

/** Where the server's own words go: the message of the day, and broadcasts. */
export const SERVER_ROOM = '#server'

export const privateRoom = (user: string) => `@${user}`
export const isPrivate = (room: string) => room.startsWith('@')
export const partner = (room: string) => room.slice(1)

export type ChatState = {
  /** Backlog per room, oldest first. */
  rooms: Record<string, ChatLine[]>
  /** Channels we are in, by name. */
  channels: Record<string, ChannelView>
  /** The server's channel directory, from the last request. */
  directory: ChannelSummaryView[]
  /** Whether to drop the host's machine-readable lines. Pushed from settings. */
  filterHostChatter: boolean
  /** Rooms with something unread, by key. */
  unread: Record<string, number>
  /** Rooms where one of those lines named us — worth more than a count. */
  named: Record<string, boolean>
  notices: Notice[]
  maxLines: number
}

function empty(): ChatState {
  return {
    rooms: { [BATTLE_ROOM]: [], [SERVER_ROOM]: [] },
    channels: {},
    directory: [],
    filterHostChatter: true,
    unread: {},
    named: {},
    notices: [],
    maxLines: 500,
  }
}

export const [chat, setChat] = createStore<ChatState>(empty())

let noticeSeq = 0

/** The room the reader is looking at, so it never accrues an unread count. */
let watching = BATTLE_ROOM

export function watchRoom(room: string): void {
  watching = room
  setChat('unread', room, 0)
  setChat('named', room, false)
}

export function pushLine(line: ChatLine): void {
  // Dropped rather than hidden at render: a line nobody will read should not
  // be taking up the backlog or an unread count either.
  if (line.kind === 'machine' && chat.filterHostChatter) return
  setChat(
    produce((state) => {
      const lines = state.rooms[line.room] ?? []
      lines.push(line)
      if (lines.length > state.maxLines)
        lines.splice(0, lines.length - state.maxLines)
      state.rooms[line.room] = lines
      if (line.room !== watching) {
        state.unread[line.room] = (state.unread[line.room] ?? 0) + 1
        if (line.mention) state.named[line.room] = true
      }
    }),
  )
}

/**
 * Opens a conversation that has no lines in it yet, so it appears in the room
 * list and can be selected before anyone has said anything.
 */
export function ensureRoom(room: string): void {
  setChat('rooms', (rooms) =>
    room in rooms ? rooms : { ...rooms, [room]: [] },
  )
}

/**
 * Forgets a private conversation, which is entirely a local matter.
 *
 * There is nothing to tell the server: a private room exists only because
 * somebody spoke. Closing it drops what was said, and the next message from
 * that person opens it again with the conversation starting fresh — which is
 * what closing it asked for.
 */
export function closePrivate(room: string): void {
  if (!isPrivate(room)) return
  setChat(
    produce((state) => {
      delete state.rooms[room]
      delete state.unread[room]
      delete state.named[room]
    }),
  )
}

/** A line from the app rather than from anyone on the server. */
export function pushSystem(room: string, text: string): void {
  noticeSeq -= 1
  pushLine({
    seq: noticeSeq,
    room,
    from: '',
    text,
    kind: 'system',
    mention: false,
    at: Math.floor(Date.now() / 1000),
  })
}

export function applyChannel(name: string, channel: ChannelView | null): void {
  setChat(
    produce((state) => {
      if (channel) {
        state.channels[name] = channel
        state.rooms[name] ??= []
      } else {
        delete state.channels[name]
        delete state.unread[name]
        delete state.named[name]
      }
    }),
  )
}

export function applyDirectory(entries: ChannelSummaryView[]): void {
  setChat('directory', entries)
}

/** Channels we are in, sorted. */
export function openChannels(): string[] {
  return Object.keys(chat.channels).sort()
}

/** Every private conversation that has been opened or spoken in. */
export function openPrivates(): string[] {
  return Object.keys(chat.rooms).filter(isPrivate).sort()
}

/** Every room that can be selected, for checking one still exists. */
export function openRooms(): string[] {
  return [BATTLE_ROOM, SERVER_ROOM, ...openChannels(), ...openPrivates()]
}

/** How long a message sits in the corner before it goes. */
const NOTICE_LIFE = 9_000

export function pushNotice(level: NoticeLevel, text: string): void {
  noticeSeq += 1
  const seq = noticeSeq
  setChat('notices', (notices) => [...notices.slice(-19), { seq, level, text }])
  // They leave on their own: these are alerts as much as errors now, and a
  // corner that only ever fills up is a log nobody asked for.
  setTimeout(() => {
    setChat('notices', (notices) =>
      notices.filter((notice) => notice.seq !== seq),
    )
  }, NOTICE_LIFE)
}

export function clearChat(): void {
  watching = BATTLE_ROOM
  setChat(empty())
}

/** Empties one room's backlog: what another room's host said is not this room's. */
export function clearRoom(room: string): void {
  setChat(
    produce((state) => {
      state.rooms[room] = []
      state.unread[room] = 0
      state.named[room] = false
    }),
  )
}
