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
  isPrivate,
  partner,
  privateRoom,
  pushNotice,
  pushSystem,
  openChannels,
  openPrivates,
  watchRoom,
} from '../store/chat'
import { showPlayerMenu } from '../components/PlayerMenu'
import { lobby } from '../store/lobby'

/**
 * Channels and private messages.
 *
 * The room we are in is listed here too, so every conversation is reachable
 * from one place, but it stays in the battle room as well — you should not
 * have to leave the players to read what they are saying.
 */
export function Chat() {
  const [room, setRoom] = createSignal(BATTLE_ROOM)
  const [text, setText] = createSignal('')
  const [showDirectory, setShowDirectory] = createSignal(false)
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
  const rooms = createMemo(() => [
    BATTLE_ROOM,
    SERVER_ROOM,
    ...channels(),
    ...privates(),
  ])

  const lines = () => chat.rooms[room()] ?? []
  const members = () => chat.channels[room()]?.members ?? []

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
        return act('join', () => api.joinChannel(argument, null))
      case 'leave': {
        const target = argument || room()
        if (target === BATTLE_ROOM || isPrivate(target))
          return pushSystem(room(), 'that is not a channel')
        return act('leave', () => api.leaveChannel(target))
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

  async function submit(event: Event) {
    event.preventDefault()
    const input = text().trim()
    if (!input) return
    setText('')
    if (input.startsWith('/') && !input.startsWith('/me ')) return run(input)
    return send(input)
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
          <For each={lobby.friends.friends}>
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
                onClick={() =>
                  void act('join', () => api.joinChannel(entry.name, null))
                }
              >
                <span class='room-name'>{entry.name}</span>
                <span class='room-count'>{entry.members}</span>
              </button>
            )}
          </For>
        </Show>
      </aside>

      <div class='chat-main'>
        <header class='chat-head'>
          <h1>{title()}</h1>
          <Show when={members().length > 0}>
            <span class='muted'>{members().length} here</span>
          </Show>
          <span class='spacer' />
          <Show when={!isPrivate(room()) && room() !== BATTLE_ROOM}>
            <button
              onClick={() => void act('leave', () => api.leaveChannel(room()))}
            >
              Leave
            </button>
          </Show>
        </header>

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

        <form class='chat-input' onSubmit={submit}>
          <input
            value={text()}
            onInput={(e) => setText(e.currentTarget.value)}
            placeholder={
              room() === BATTLE_ROOM
                ? 'Say something, or a !command'
                : 'Say something, or /join /leave /msg /ignore /channels'
            }
          />
          <button type='submit'>Send</button>
        </form>
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
        <span class='badge'>{chat.unread[props.room]}</span>
      </Show>
    </button>
  )
}

function Line(props: { line: ChatLine; me: string | null }) {
  const mine = () => props.line.from === props.me
  return (
    <div class={`line ${props.line.kind}`} classList={{ mine: mine() }}>
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
      <span class='text'>{props.line.text}</span>
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
