import { createVirtualizer } from '@tanstack/solid-virtual'
import {
  For,
  Show,
  createEffect,
  createMemo,
  createSignal,
  onMount,
} from 'solid-js'
import type { ChatLine } from '../ipc/bindings/ChatLine'
import { api, describeError } from '../ipc/client'
import {
  BATTLE_ROOM,
  SERVER_ROOM,
  chat,
  ensureRoom,
  closePrivate,
  isPrivate,
  partner,
  privateRoom,
  pushNotice,
  pushSystem,
  openChannels,
  openPrivates,
  watchRoom,
} from '../store/chat'
import { Composer } from '../components/Composer'
import { Linkify } from '../components/Linkify'
import { showPlayerMenu } from '../components/PlayerMenu'
import { rememberChannel } from '../store/channels'
import { TabStrip, type Tab as StripTab } from '../components/TabStrip'
import { ordered } from '../lib/reorder'
import { lobby } from '../store/lobby'

/**
 * Channels and private messages.
 *
 * The room we are in is listed here too, so every conversation is reachable
 * from one place, but it stays in the battle room as well — you should not
 * have to leave the players to read what they are saying.
 */
const ROSTER_ROW = 19

export function Chat() {
  const [room, setRoom] = createSignal(BATTLE_ROOM)
  const [showDirectory, setShowDirectory] = createSignal(false)
  const [findPerson, setFindPerson] = createSignal('')
  const [showMembers, setShowMembers] = createSignal(false)
  /**
   * A channel we have asked to join. The server answers a join with the
   * channel's state rather than an acknowledgement, so the reader is taken
   * there when it arrives — asking to join and then staying where you were is
   * not what anyone means by it.
   */
  const [joining, setJoining] = createSignal<string | null>(null)
  let log: HTMLDivElement | undefined

  // These read the store, so they recompute as channels and people come and go.
  const channels = createMemo(() => {
    chat.channels
    return openChannels()
  })
  const privates = createMemo(() => {
    chat.rooms
    return openPrivates()
  })
  /** Online users matching the search, friends first, capped so it stays a list. */
  const people = createMemo(() => {
    const needle = findPerson().trim().toLowerCase()
    if (needle.length < 2) return []
    const friends = new Set(lobby.friends.friends)
    return Object.keys(lobby.users)
      .filter((name) => name.toLowerCase().includes(needle))
      .sort((a, b) => {
        const known = Number(friends.has(b)) - Number(friends.has(a))
        return known || a.localeCompare(b)
      })
      .slice(0, 30)
  })

  /** Friends, online ones first: an offline friend is not one you can talk to. */
  const friends = createMemo(() =>
    [...lobby.friends.friends].sort((a, b) => {
      const here = Number(b in lobby.users) - Number(a in lobby.users)
      return here || a.localeCompare(b)
    }),
  )

  const rooms = createMemo(() => [
    BATTLE_ROOM,
    SERVER_ROOM,
    ...channels(),
    ...privates(),
  ])

  /**
   * The reader's own tab order, kept for this session only.
   *
   * Not persisted: where you like your conversations depends on which ones are
   * open, and half of them are people who happened to message you today. A
   * saved order would mostly describe a room you are no longer in.
   */
  const [order, setOrder] = createSignal<string[]>([])

  /** What is open, in the reader's order, with new arrivals at the end. */
  const tabs = createMemo<StripTab[]>(() =>
    ordered(rooms(), order()).map((key) => ({
      key,
      label:
        key === BATTLE_ROOM
          ? 'Battle room'
          : key === SERVER_ROOM
            ? 'Server'
            : key,
      badge: chat.unread[key],
      urgent: chat.named[key],
      // The battle room and the server are always there; a channel or a person
      // is something you opened and can close.
      closable: key !== BATTLE_ROOM && key !== SERVER_ROOM,
      title: isPrivate(key) ? `Messages with ${key}` : key,
    })),
  )

  async function close(key: string) {
    if (isPrivate(key)) {
      // A conversation with a person is only ours; nothing to tell the server.
      closePrivate(key)
      return
    }
    await act('leave', async () => {
      await api.leaveChannel(key)
      await rememberChannel(key, false)
    })
  }

  const lines = () => chat.rooms[room()] ?? []
  const members = () => chat.channels[room()]?.members ?? []
  /** Friends first, then alphabetical — the same order as everywhere else. */
  const sortedMembers = createMemo(() =>
    [...members()].sort((a, b) => {
      const known =
        Number(lobby.friends.friends.includes(b)) -
        Number(lobby.friends.friends.includes(a))
      return known || a.localeCompare(b)
    }),
  )

  // The server never announces a friendship changing, so the list is asked
  // for when this view opens.
  onMount(() => void act('friends', () => api.refreshFriends()))

  createEffect(() => {
    watchRoom(room())
  })
  createEffect(() => {
    lines().length
    log?.scrollTo({ top: log.scrollHeight })
  })

  createEffect(() => {
    const wanted = joining()
    if (wanted !== null && wanted in chat.channels) {
      setRoom(wanted)
      setJoining(null)
    }
  })

  /** A room that has gone away leaves the reader somewhere that still exists. */
  createEffect(() => {
    if (!rooms().includes(room())) setRoom(BATTLE_ROOM)
  })

  async function act(what: string, run: () => Promise<void>) {
    try {
      await run()
    } catch (error) {
      pushNotice('warning', `${what}: ${describeError(error)}`)
    }
  }

  /**
   * Slash commands, which is how every lobby client has spelled these since
   * the protocol was written. `/me` is left alone: the server has its own verb
   * for an emote, so it travels as text and is turned into `SAYEX` below.
   */
  async function run(input: string) {
    const [word, ...rest] = input.slice(1).split(' ')
    const argument = rest.join(' ').trim()
    const command = (word ?? '').toLowerCase()

    switch (command) {
      case 'join':
        if (!argument) return pushSystem(room(), 'usage: /join <channel>')
        // teiserver does not answer a join for a channel you are already in,
        // so without this the command would look like it did nothing.
        if (argument in chat.channels) return setRoom(argument)
        setJoining(argument)
        return act('join', async () => {
          await api.joinChannel(argument, null)
          await rememberChannel(argument, true)
        })
      case 'leave': {
        const target = argument || room()
        if (target === BATTLE_ROOM || isPrivate(target))
          return pushSystem(room(), 'that is not a channel')
        return act('leave', async () => {
          await api.leaveChannel(target)
          await rememberChannel(target, false)
        })
      }
      case 'msg':
      case 'pm': {
        const [who, ...words] = rest
        const body = words.join(' ').trim()
        if (!who) return pushSystem(room(), 'usage: /msg <user> <message>')
        // The conversation has to exist before it can be selected, or the
        // guard below sends the reader straight back to the battle room.
        ensureRoom(privateRoom(who))
        setRoom(privateRoom(who))
        if (body) return act('message', () => api.sayPrivate(who, body))
        return
      }
      case 'ignore':
      case 'unignore': {
        if (!argument) return pushSystem(room(), `usage: /${command} <user>`)
        return act(command, () => api.friendAction(command, argument))
      }
      case 'channels':
        setShowDirectory(true)
        return act('channels', () => api.listChannels())
      case 'me':
        // Handled by the server; fall through to sending it verbatim.
        return send(input)
      default:
        return pushSystem(room(), `no such command: /${command}`)
    }
  }

  async function send(body: string) {
    const where = room()
    if (where === SERVER_ROOM)
      return pushSystem(where, 'nobody is listening in here')
    if (where === BATTLE_ROOM) return act('say', () => api.sayBattle(body))
    if (isPrivate(where))
      return act('say', () => api.sayPrivate(partner(where), body))
    return act('say', () => api.sayChannel(where, body))
  }

  function submit(line: string) {
    const input = line.trim()
    if (!input) return
    if (input.startsWith('/') && !input.startsWith('/me ')) {
      void run(input)
      return
    }
    void send(input)
  }

  /**
   * Whose names Tab may finish here: the channel's members, or the people in
   * the room, or — in a private conversation — the one person in it.
   */
  const nameable = () => {
    const where = room()
    if (isPrivate(where)) return [partner(where)]
    if (where === BATTLE_ROOM) {
      const id = lobby.myBattle?.id
      return id === undefined ? [] : (lobby.battles[id]?.members ?? [])
    }
    return members()
  }

  const title = () => {
    const where = room()
    if (where === BATTLE_ROOM) return 'Battle room'
    if (where === SERVER_ROOM) return 'Server'
    return isPrivate(where) ? partner(where) : where
  }

  return (
    <section class='chat-view'>
      <aside class='room-list'>
        <div class='room-list-head'>
          <span class='filter-label'>Rooms</span>
          <button
            class='chip-choice'
            classList={{ on: showDirectory() }}
            onClick={() => {
              setShowDirectory(!showDirectory())
              if (!showDirectory()) return
              void act('channels', () => api.listChannels())
            }}
          >
            Browse
          </button>
        </div>

        {/* Everyone online is already in the store, so finding someone is a
            filter rather than a request — and it beats `/msg` with an exact
            name when the name you want is `[Crd]XxStormKittyxX`. */}
        <input
          class='find-person'
          placeholder='Find someone'
          value={findPerson()}
          onInput={(e) => setFindPerson(e.currentTarget.value)}
        />
        <Show when={findPerson().trim().length > 1}>
          <div class='room-list-head'>
            <span class='filter-label'>Matches</span>
          </div>
          <For
            each={people()}
            fallback={<p class='muted setup-empty'>Nobody by that name.</p>}
          >
            {(name) => (
              <button
                class='room-tab'
                onClick={() => {
                  ensureRoom(privateRoom(name))
                  setRoom(privateRoom(name))
                  setFindPerson('')
                }}
              >
                <span class='room-name'>{name}</span>
                <Show when={lobby.friends.friends.includes(name)}>
                  <span class='room-count'>friend</span>
                </Show>
              </button>
            )}
          </For>
        </Show>

        <Tab
          room={BATTLE_ROOM}
          label='Battle room'
          on={room() === BATTLE_ROOM}
          onClick={setRoom}
        />
        <Tab
          room={SERVER_ROOM}
          label='Server'
          on={room() === SERVER_ROOM}
          onClick={setRoom}
        />

        {/* Channels and people are listed apart, because a channel and a
            person can carry the same name and mean different conversations. */}
        <Show when={channels().length > 0}>
          <div class='room-list-head'>
            <span class='filter-label'>Channels</span>
          </div>
          <For each={channels()}>
            {(key) => (
              <Tab
                room={key}
                label={key}
                on={room() === key}
                onClick={setRoom}
              />
            )}
          </For>
        </Show>

        <Show when={lobby.friends.requests.length > 0}>
          <div class='room-list-head'>
            <span class='filter-label'>Wants to be friends</span>
          </div>
          <For each={lobby.friends.requests}>
            {(name) => (
              <div class='friend-request'>
                <span class='room-name'>{name}</span>
                <button
                  class='chip-choice'
                  title={`Accept ${name}`}
                  onClick={() =>
                    void act('accept', () => api.friendAction('accept', name))
                  }
                >
                  Yes
                </button>
                <button
                  class='chip-choice'
                  title={`Decline ${name}`}
                  onClick={() =>
                    void act('decline', () => api.friendAction('decline', name))
                  }
                >
                  No
                </button>
              </div>
            )}
          </For>
        </Show>

        <Show when={lobby.friends.friends.length > 0}>
          <div class='room-list-head'>
            <span class='filter-label'>Friends</span>
          </div>
          <For each={friends()}>
            {(name) => (
              <button
                class='room-tab friend'
                onClick={() => {
                  ensureRoom(privateRoom(name))
                  setRoom(privateRoom(name))
                }}
              >
                <span class='room-name'>{name}</span>
                <Show when={lobby.users[name]}>
                  <span class='room-count'>online</span>
                </Show>
              </button>
            )}
          </For>
        </Show>

        <Show when={lobby.friends.ignored.length > 0}>
          <div class='room-list-head'>
            <span class='filter-label'>Ignored</span>
          </div>
          <For each={lobby.friends.ignored}>
            {(name) => (
              <div class='friend-request'>
                <span class='room-name muted'>{name}</span>
                <button
                  class='chip-choice'
                  title={`Stop ignoring ${name}`}
                  onClick={() =>
                    void act('unignore', () =>
                      api.friendAction('unignore', name),
                    )
                  }
                >
                  Undo
                </button>
              </div>
            )}
          </For>
        </Show>

        <Show when={privates().length > 0}>
          <div class='room-list-head'>
            <span class='filter-label'>People</span>
          </div>
          <For each={privates()}>
            {(key) => (
              <Tab
                room={key}
                label={partner(key)}
                on={room() === key}
                onClick={setRoom}
              />
            )}
          </For>
        </Show>

        <Show when={showDirectory()}>
          <div class='room-list-head'>
            <span class='filter-label'>All channels</span>
          </div>
          <For
            each={chat.directory}
            fallback={<p class='muted setup-empty'>Asking the server…</p>}
          >
            {(entry) => (
              <button
                class='room-tab'
                disabled={entry.name in chat.channels}
                onClick={() => {
                  setJoining(entry.name)
                  void act('join', async () => {
                    await api.joinChannel(entry.name, null)
                    await rememberChannel(entry.name, true)
                  })
                }}
              >
                <span class='room-name'>{entry.name}</span>
                <span class='room-count'>{entry.members}</span>
              </button>
            )}
          </For>
        </Show>
      </aside>

      <div class='chat-main'>
        {/* What is open, in the reader's order. The list on the left is for
            finding a conversation; this is for living in the ones you have. */}
        <TabStrip
          tabs={tabs()}
          active={room()}
          onSelect={setRoom}
          onClose={(key) => void close(key)}
          onReorder={setOrder}
        />
        <header class='chat-head'>
          <h1>{title()}</h1>
          <Show when={members().length > 0}>
            <button
              class='link'
              title='Who is in this channel'
              onClick={() => setShowMembers(!showMembers())}
            >
              {members().length} here
            </button>
          </Show>
          <span class='spacer' />
          <Show when={!isPrivate(room()) && room() !== BATTLE_ROOM}>
            <button
              onClick={() =>
                void act('leave', async () => {
                  const where = room()
                  await api.leaveChannel(where)
                  await rememberChannel(where, false)
                })
              }
            >
              Leave
            </button>
          </Show>
        </header>

        {/* The roster is a column beside the log rather than a list above it:
            a channel with two hundred people in it would otherwise push the
            conversation off the screen. */}
        <div class='chat-body' classList={{ roster: showMembers() }}>
          <div class='chat-log' ref={log}>
            <For
              each={lines()}
              fallback={
                <p class='muted setup-empty'>
                  Nothing here yet. <code>/join &lt;channel&gt;</code> or{' '}
                  <code>/msg &lt;user&gt; …</code>
                </p>
              }
            >
              {(line) => <Line line={line} me={lobby.me} />}
            </For>
          </div>

          <Show when={showMembers() && members().length > 0}>
            <Roster names={sortedMembers()} />
          </Show>
        </div>

        <Composer
          placeholder={
            room() === BATTLE_ROOM
              ? 'Say something, or a !command'
              : 'Say something, or /join /leave /msg /ignore /channels'
          }
          names={nameable}
          onSend={submit}
        />
      </div>
    </section>
  )
}

function Tab(props: {
  room: string
  label: string
  on: boolean
  onClick: (room: string) => void
}) {
  return (
    <button
      class='room-tab'
      classList={{ on: props.on }}
      onClick={() => props.onClick(props.room)}
    >
      <span class='room-name'>{props.label}</span>
      <Show when={chat.unread[props.room]}>
        <span class='badge' classList={{ named: chat.named[props.room] }}>
          {chat.unread[props.room]}
        </span>
      </Show>
    </button>
  )
}

function Line(props: { line: ChatLine; me: string | null }) {
  const mine = () => props.line.from === props.me
  return (
    <div
      class={`line ${props.line.kind}`}
      classList={{ mine: mine(), named: props.line.mention }}
    >
      <span class='at' title={new Date(props.line.at * 1000).toLocaleString()}>
        {clock(props.line.at)}
      </span>
      <span
        class='from'
        onClick={(event) =>
          // A system line's "from" names the app or the server, not a person
          // there is anything to be done about.
          props.line.kind !== 'system' &&
          props.line.from &&
          showPlayerMenu(props.line.from, event)
        }
      >
        {props.line.from}
      </span>
      <span class='text'>
        <Linkify text={props.line.text} />
      </span>
    </div>
  )
}

/** `14:07` — the hour and minute is all a backlog needs. */
function clock(at: number): string {
  if (!at) return ''
  return new Date(at * 1000).toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
  })
}

/**
 * Who is in a channel.
 *
 * `#main` holds most of the server — seventeen hundred people on a quiet
 * evening — so the rows are virtualised like the battle list. Drawing them all
 * does not merely cost time: the column grows to their full height and pushes
 * the conversation off the screen.
 */
function Roster(props: { names: string[] }) {
  let scrollRef: HTMLElement | undefined

  const virtualizer = createVirtualizer({
    get count() {
      return props.names.length
    },
    getScrollElement: () => scrollRef ?? null,
    estimateSize: () => ROSTER_ROW,
    overscan: 12,
  })

  return (
    <aside class='chat-roster' ref={scrollRef}>
      <div
        style={{
          height: `${virtualizer.getTotalSize()}px`,
          position: 'relative',
          width: '100%',
        }}
      >
        <For each={virtualizer.getVirtualItems()}>
          {(item) => {
            const name = () => props.names[item.index]
            return (
              <Show when={name()}>
                {(who) => (
                  <button
                    class='pname'
                    classList={{
                      me: who() === lobby.me,
                      friend: lobby.friends.friends.includes(who()),
                    }}
                    style={{
                      position: 'absolute',
                      top: `${item.start}px`,
                      height: `${ROSTER_ROW}px`,
                      width: '100%',
                    }}
                    onClick={(event) => showPlayerMenu(who(), event)}
                  >
                    {who()}
                  </button>
                )}
              </Show>
            )
          }}
        </For>
      </div>
    </aside>
  )
}
