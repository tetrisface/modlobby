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
  /** Rooms with something unread, by key. */
  unread: Record<string, number>
  notices: Notice[]
  maxLines: number
}

function empty(): ChatState {
  return {
    rooms: { [BATTLE_ROOM]: [], [SERVER_ROOM]: [] },
    channels: {},
    directory: [],
    unread: {},
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
}

export function pushLine(line: ChatLine): void {
  setChat(
    produce((state) => {
      const lines = state.rooms[line.room] ?? []
      lines.push(line)
      if (lines.length > state.maxLines)
        lines.splice(0, lines.length - state.maxLines)
      state.rooms[line.room] = lines
      if (line.room !== watching)
        state.unread[line.room] = (state.unread[line.room] ?? 0) + 1
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

/** A line from the app rather than from anyone on the server. */
export function pushSystem(room: string, text: string): void {
  noticeSeq -= 1
  pushLine({
    seq: noticeSeq,
    room,
    from: '',
    text,
    kind: 'system',
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

export function pushNotice(level: NoticeLevel, text: string): void {
  noticeSeq += 1
  setChat('notices', (notices) => [
    ...notices.slice(-19),
    { seq: noticeSeq, level, text },
  ])
}

export function clearChat(): void {
  watching = BATTLE_ROOM
  setChat(empty())
}
