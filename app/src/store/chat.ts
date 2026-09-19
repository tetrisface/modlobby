import { createStore, produce } from 'solid-js/store'
import type { ChannelView } from '../ipc/bindings/ChannelView'
import type { ChannelSummaryView } from '../ipc/bindings/ChannelSummaryView'
import type { ChatLine } from '../ipc/bindings/ChatLine'
import type { NoticeLevel } from '../ipc/bindings/NoticeLevel'
import { api } from '../ipc/client'

export type Notice = {
  seq: number
  level: NoticeLevel
  text: string
  /** The server that said it, by id; `null` for the app's own. */
  server: string | null
  /** When this one leaves, pushed back while the pointer is in the corner. */
  expiresAt: number
}

/**
 * The room key for the battle we are in; `lobby-ui` writes the same string.
 * One across every server, since there is one room at most.
 */
export const BATTLE_ROOM = '#battle'

/** A skirmish room's own log: what its console answered, and what changed. */
export const SKIRMISH_ROOM = '#skirmish'

/**
 * Where a server's own words go: the message of the day, and broadcasts.
 * Each server has one.
 */
export const SERVER_ROOM = '#server'

/**
 * Where a server's conversation is kept: the server, a space, and the room as
 * the runtime names it — a channel, `@name`, `#server`. Every server has a
 * `main` and a `@bob` of its own. The battle and skirmish rooms are one each
 * whatever the server, and keep their bare names.
 */
export function roomKey(server: string | null, room: string): string {
  return server === null || room === BATTLE_ROOM || room === SKIRMISH_ROOM
    ? room
    : `${server} ${room}`
}

/** The server a key is on, `null` for one of its own, and the room itself. */
export function parseKey(key: string): { server: string | null; room: string } {
  const space = key.indexOf(' ')
  return space < 0
    ? { server: null, room: key }
    : { server: key.slice(0, space), room: key.slice(space + 1) }
}

/** A room as it reads, without the server. */
export const roomName = (key: string) => parseKey(key).room
export const privateRoom = (server: string, user: string) =>
  roomKey(server, `@${user}`)
export const serverRoom = (server: string) => roomKey(server, SERVER_ROOM)
export const isPrivate = (key: string) => roomName(key).startsWith('@')
export const partner = (key: string) => roomName(key).slice(1)

/** Rooms by name, and one name's rooms by server. */
const byName = (a: string, b: string) =>
  roomName(a).localeCompare(roomName(b)) || a.localeCompare(b)

export type ChatState = {
  /** Backlog per room, oldest first. */
  rooms: Record<string, ChatLine[]>
  /** Channels we are in, by key. */
  channels: Record<string, ChannelView>
  /** Each server's channel directory, from the last request to it. */
  directory: Record<string, ChannelSummaryView[]>
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
    rooms: { [BATTLE_ROOM]: [] },
    channels: {},
    directory: {},
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
      // The message of the day is the same greeting on every connect.
      if (line.room !== watching && line.kind !== 'motd') {
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

export function applyChannel(key: string, channel: ChannelView | null): void {
  setChat(
    produce((state) => {
      if (channel) {
        state.channels[key] = channel
        state.rooms[key] ??= []
      } else {
        delete state.channels[key]
        delete state.unread[key]
        delete state.named[key]
      }
    }),
  )
}

/**
 * A session ended: the server forgets we were in its channels the moment
 * the connection goes, so they are no longer ours. The backlog stays, as
 * does every other server's membership — and the next login rejoins what
 * the settings remember, which it would skip for a channel still held here.
 */
export function leaveChannelsOf(server: string): void {
  setChat(
    produce((state) => {
      for (const key of Object.keys(state.channels))
        if (parseKey(key).server === server) delete state.channels[key]
      delete state.directory[server]
    }),
  )
}

export function applyDirectory(
  server: string,
  entries: ChannelSummaryView[],
): void {
  setChat('directory', server, entries)
}

/** Channels we are in, by name. */
export function openChannels(): string[] {
  return Object.keys(chat.channels).sort(byName)
}

/** Every private conversation that has been opened or spoken in. */
export function openPrivates(): string[] {
  return Object.keys(chat.rooms).filter(isPrivate).sort(byName)
}

/**
 * Whether a room is kept out of the Chat tab's count. Muting is by the room's
 * name, so muting `main` mutes it on every server.
 */
export function muteOf(muted: readonly string[], key: string): boolean {
  return muted.includes(roomName(key))
}

/**
 * The unread count on the Chat tab: everything outside the muted rooms, and a
 * muted room's too once one of its lines names you.
 */
export function unreadTotal(muted: readonly string[]): number {
  return Object.entries(chat.unread)
    .filter(([room]) => chat.named[room] || !muteOf(muted, room))
    .reduce((total, [, count]) => total + count, 0)
}

/** When a conversation last moved, in seconds; 0 when nothing has been said. */
export function lastSaid(room: string): number {
  return chat.rooms[room]?.at(-1)?.at ?? 0
}

/**
 * Conversations with people — private room keys — by who you can talk to now,
 * then who spoke last, then name. Muting plays no part: a muted person who
 * writes still comes up to the top.
 */
export function byActivity(online: (key: string) => boolean) {
  return (a: string, b: string) =>
    Number(online(b)) - Number(online(a)) ||
    lastSaid(b) - lastSaid(a) ||
    byName(a, b)
}

/** How long a message sits in the corner before it goes. */
const NOTICE_LIFE = 9_000

/** How often the corner is checked for something to drop. */
const SWEEP_EVERY = 250

let sweeping: ReturnType<typeof setInterval> | undefined
/** When the pointer entered the corner, or `null` when it is not in it. */
let heldSince: number | null = null

function sweep(): void {
  // Held: the pointer is in the corner and somebody is reading.
  if (heldSince !== null) return
  const now = Date.now()
  setChat('notices', (notices) =>
    notices.filter((notice) => notice.expiresAt > now),
  )
  if (chat.notices.length === 0) {
    clearInterval(sweeping)
    sweeping = undefined
  }
}

export function pushNotice(
  level: NoticeLevel,
  text: string,
  server: string | null = null,
): void {
  noticeSeq += 1
  // They leave on their own: these are alerts as much as errors now, and a
  // corner that only ever fills up is a log nobody asked for. One sweeper for
  // all of them rather than a timer each, so that holding them all back while
  // the pointer is in the corner is a single decision.
  const notice = {
    seq: noticeSeq,
    level,
    text,
    server,
    expiresAt: Date.now() + NOTICE_LIFE,
  }
  setChat('notices', (notices) => [...notices.slice(-19), notice])
  sweeping ??= setInterval(sweep, SWEEP_EVERY)
  // An error is the app's own failing, so the next look for a fix comes
  // sooner. Losable bookkeeping, like the rest of the update memory.
  if (level === 'error') api.noteTrouble().catch(() => {})
}

/**
 * Stop the corner emptying itself while somebody is reading it.
 *
 * The time spent held is given back to every notice on release rather than
 * their being pinned outright: a message you have already read should not
 * need dismissing, and one that arrived as you moved the mouse there should
 * still get its full span.
 */
export function holdNotices(held: boolean): void {
  if (held) {
    heldSince ??= Date.now()
    return
  }
  if (heldSince === null) return
  const paused = Date.now() - heldSince
  heldSince = null
  setChat('notices', (notices) =>
    notices.map((notice) => ({
      ...notice,
      expiresAt: notice.expiresAt + paused,
    })),
  )
}

export function clearChat(): void {
  watching = BATTLE_ROOM
  // The corner goes with it, and so does the timer emptying it: an interval
  // left running against notices that no longer exist is a handle nobody owns.
  clearInterval(sweeping)
  sweeping = undefined
  heldSince = null
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
