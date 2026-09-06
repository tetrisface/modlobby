import { useNavigate } from '@solidjs/router'
import {
  For,
  Index,
  Match,
  Show,
  Switch,
  createEffect,
  createMemo,
  createResource,
  createSignal,
} from 'solid-js'
import { Composer } from '../components/Composer'
import { GetEngine } from '../components/GetEngine'
import { Linkify } from '../components/Linkify'
import { MapEditor } from '../components/MapEditor'
import { MapPicker, pickMap } from '../components/MapPicker'
import { MapPicture } from '../components/MapPicture'
import { showPlayerMenu } from '../components/PlayerMenu'
import {
  BotRow,
  EmptySeat,
  GuessedRow,
  PlayerRow,
  SpectatorRow,
} from '../components/PlayerRow'
import { SyncIcon } from '../components/icons'
import type { BattleView } from '../ipc/bindings/BattleView'
import type { BotView } from '../ipc/bindings/BotView'
import type { ChatLine } from '../ipc/bindings/ChatLine'
import type { DownloadStatus } from '../ipc/bindings/DownloadStatus'
import type { StartRectView } from '../ipc/bindings/StartRectView'
import type { UserView } from '../ipc/bindings/UserView'
import { api, describeError } from '../ipc/client'
import { boxSignature, centre, outline } from '../lib/boxes'
import { downloadFraction } from '../lib/download'
import { TILES } from '../lib/maps'
import { type Roster, arrange, emptySeats } from '../lib/roster'
import { readSkills, teamSkill, type Skill } from '../lib/skill'
import { chat, pushNotice } from '../store/chat'
import { joinMilestone } from '../store/join'
import { lobby } from '../store/lobby'
import { settings } from '../store/settings'
import { HostBar } from './HostBar'
import { PveScore } from './PveScore'
import { RoomTitle } from './RoomTitle'
import { useRoom, type RoomModel } from './room/model'
import { Seat, seatsAllowed, sitOn } from './Seat'
import { StartBoxes } from './StartBoxes'
import { Setup } from './Setup'
import { VoteBar } from './VoteBar'

export function Room() {
  const navigate = useNavigate()
  const room = useRoom()
  let log: HTMLDivElement | undefined

  const battle = createMemo(room.battle)

  const friends = createMemo(() => new Set(lobby.friends.friends))
  const isFriend = (name: string) => friends().has(name)

  /** SPADS keys its player tags by lowercased name. */
  const skills = createMemo(() => readSkills(room.my()?.scriptTags))
  const skillOf = (name: string): Skill | null =>
    skills()[name.toLowerCase()] ?? null

  const occupants = createMemo((): Roster => {
    const b = battle()
    if (!b) return { teams: [], spectators: [], pending: [], spectatorCount: 0 }
    return arrange(b, room.users(), room.me())
  })

  /**
   * SPADS takes `!rename` from a player — outright from the boss, as a vote
   * from anyone else — and refuses it from a spectator (`[rename]` in BAR's
   * `BarManagerCmd.conf`). Not drawing the pen beats a silent refusal.
   */
  const canRename = createMemo(() => {
    const me = room.me()
    if (me === null) return false
    if (!room.caps.spads) return true
    if (room.my()?.boss === me) return true
    return room.users()[me]?.battleStatus?.player ?? false
  })

  /** A team's header offers a seat on it unless we already hold one there. */
  const canJoin = (allyTeam: number): boolean => {
    if (!seatsAllowed(room)) return false
    const me = room.me()
    const status = me === null ? undefined : room.users()[me]?.battleStatus
    return !(status?.player && status.allyTeam === allyTeam)
  }

  async function join(allyTeam: number) {
    try {
      await sitOn(room, allyTeam)
    } catch (error) {
      pushNotice(
        'warning',
        `join team ${allyTeam + 1}: ${describeError(error)}`,
      )
    }
  }

  // Whoever provided the room decides what a missing one means; the router
  // only lives here, so the going is done here.
  createEffect(() => {
    const away = room.exit()
    if (away !== null) navigate(away, { replace: true })
  })
  createEffect(() => {
    const roster = occupants()
    if (roster.teams.length > 0 && roster.pending.length === 0)
      joinMilestone('roster settled')
  })
  /** Whether the large map with the start-box editor is open over the room. */
  const [editing, setEditing] = createSignal(false)
  /** Whether the list of installed maps is open over it. */
  const [picking, setPicking] = createSignal(false)

  const lines = () => chat.rooms[room.log] ?? []
  createEffect(() => {
    lines().length
    log?.scrollTo({ top: log.scrollHeight })
  })

  async function send(line: string) {
    try {
      await room.io.sayBattle(line.trim())
    } catch (error) {
      pushNotice('warning', describeError(error))
    }
  }

  async function launch() {
    try {
      await room.io.launch()
    } catch (error) {
      pushNotice('error', describeError(error))
    }
  }

  return (
    <Show when={battle()}>
      {(b) => (
        <section class='room'>
          <header class='room-card'>
            <Minimap
              rects={b().startRects}
              mapName={b().mapName}
              teams={occupants().teams.length}
              onOpen={() => setEditing(true)}
            />
            <Show when={editing()}>
              <MapEditor
                mapName={b().mapName}
                teams={Math.max(occupants().teams.length, 2)}
                onClose={() => setEditing(false)}
              />
            </Show>
            <Show when={picking()}>
              <MapPicker
                current={b().mapName}
                onPick={(name) => {
                  setPicking(false)
                  void pickMap(room.io.setMap, name)
                }}
                onClose={() => setPicking(false)}
              />
            </Show>
            <div class='card-main'>
              <RoomTitle
                title={b().title}
                canRename={canRename()}
                onRename={(name) => void send(`!rename ${name}`)}
              />
              <div class='card-meta'>
                <span>
                  Map{' '}
                  {/* Where the map is ours to choose, its name is the way to
                      choose it. Where it is the host's, the name is a link to
                      the page Chobby opens for a map, so it lands where people
                      already expect it to. */}
                  <Show
                    when={room.caps.picksContent}
                    fallback={
                      <b
                        class='chat-link'
                        title='Open this map on beyondallreason.info'
                        onClick={() =>
                          void api
                            .openUrl(
                              `https://www.beyondallreason.info/maps?mapname=${encodeURIComponent(b().mapName)}`,
                            )
                            .catch((error) =>
                              pushNotice('warning', describeError(error)),
                            )
                        }
                      >
                        {b().mapName}
                      </b>
                    }
                  >
                    <b
                      class='chat-link'
                      title='Play a different map'
                      onClick={() => setPicking(true)}
                    >
                      {b().mapName}
                    </b>
                  </Show>
                </span>
                <span>
                  Host <b>{b().founder}</b>
                </span>
                <span>
                  Engine <b>{b().engineVersion}</b>
                </span>
                <span>
                  Game <b>{b().gameName}</b>
                </span>
              </div>
              <Chips battle={b()} />
            </div>
            <div class='card-actions'>
              {/* While our own engine runs, the useful button is not another
                  launch — it is the way back to the game the lobby is
                  sitting on top of. */}
              <Switch>
                <Match when={lobby.engine.state === 'running'}>
                  <button
                    class='primary'
                    title={`Or press ${settings()?.overlay.hotkey ?? 'the overlay shortcut'}`}
                    onClick={() => void api.overlayToggle()}
                  >
                    Back to game
                  </button>
                </Match>
                <Match when={room.running()}>
                  <button class='primary' onClick={launch}>
                    Watch the game
                  </button>
                </Match>
                {/* Nobody else is going to start this one. */}
                <Match when={room.caps.startsGame}>
                  <button
                    class='primary'
                    disabled={missingParts(room).length > 0}
                    onClick={launch}
                  >
                    Start
                  </button>
                </Match>
              </Switch>
              <Show when={room.caps.leave}>
                <button onClick={() => room.io.leaveBattle()}>Leave</button>
              </Show>
            </div>
          </header>

          <PveScore />
          {/* Votes and the host bar are SPADS: there is nothing behind them
              in a room that answers to nobody. */}
          <Show when={room.caps.spads}>
            <VoteBar teams={Math.max(occupants().teams.length, 2)} />
          </Show>
          <StartBoxes
            teams={Math.max(occupants().teams.length, 2)}
            mapName={b().mapName}
          />
          <Show when={room.caps.spads}>
            <HostBar />
          </Show>
          <Seat />

          <div class='room-body'>
            <div class='room-main'>
              <div class='rosters'>
                <div class='teams'>
                  {/* By position, not by object: the memo builds new team
                      objects on every status line, and a `For` keyed on
                      them rebuilt every row each time. The users inside are
                      the store's own objects, so their rows do survive. */}
                  <Index each={occupants().teams}>
                    {(team) => (
                      <section class='team'>
                        <header class='team-head'>
                          <span class='name'>Team {team().allyTeam + 1}</span>
                          <span class='count'>{team().expected}</span>
                          <Show when={team().users.length > 0}>
                            <span class='os'>
                              Σ{' '}
                              {teamSkill(
                                team().users.map((u) => skillOf(u.name)),
                              ).toFixed(1)}
                            </span>
                          </Show>
                          <Show when={canJoin(team().allyTeam)}>
                            <button
                              class='team-join'
                              title={`Take a seat on team ${team().allyTeam + 1}`}
                              onClick={() => void join(team().allyTeam)}
                            >
                              Join
                            </button>
                          </Show>
                        </header>
                        <For each={team().users}>
                          {(user) => (
                            <PlayerRow
                              user={user}
                              skill={skillOf(user.name)}
                              me={user.name === room.me()}
                              friend={isFriend(user.name)}
                              boss={room.my()?.boss === user.name}
                              download={
                                user.name === room.me()
                                  ? lobby.download
                                  : undefined
                              }
                            />
                          )}
                        </For>
                        <For each={team().guessed}>
                          {(user) => (
                            <GuessedRow
                              user={user}
                              me={user.name === room.me()}
                              friend={isFriend(user.name)}
                            />
                          )}
                        </For>
                        <For each={team().bots}>
                          {(bot) => (
                            <BotRow
                              bot={bot}
                              onRemove={
                                // The server takes REMOVEBOT from the owner,
                                // the host and moderators; a boss is none of
                                // those. Not drawing the action beats a
                                // silent refusal.
                                bot.owner === room.me()
                                  ? () =>
                                      room.io
                                        .removeBot(bot.name)
                                        .catch((error) =>
                                          pushNotice(
                                            'warning',
                                            describeError(error),
                                          ),
                                        )
                                  : undefined
                              }
                            />
                          )}
                        </For>
                        <Index
                          each={Array.from({ length: emptySeats(team()) })}
                        >
                          {() => <EmptySeat />}
                        </Index>
                      </section>
                    )}
                  </Index>
                </div>

                <section class='spectators'>
                  <header class='team-head'>
                    <span class='name'>Spectators</span>
                    <span class='count'>{occupants().spectatorCount}</span>
                  </header>
                  <div class='spectator-list'>
                    <For each={occupants().spectators}>
                      {(user) => (
                        <SpectatorRow
                          user={user}
                          me={user.name === room.me()}
                          friend={isFriend(user.name)}
                          boss={room.my()?.boss === user.name}
                        />
                      )}
                    </For>
                    {/* Not placed by the server yet: listed so the room has
                        its names at once, dimmed because most are about to
                        take a seat above. */}
                    <For each={occupants().pending}>
                      {(user) => (
                        <SpectatorRow
                          user={user}
                          me={user.name === room.me()}
                          friend={isFriend(user.name)}
                          pending
                        />
                      )}
                    </For>
                  </div>
                </section>
              </div>

              <div class='room-chat'>
                <div class='chat-log' ref={log}>
                  <For each={lines()}>{(line) => <Line line={line} />}</For>
                </div>
                <Composer
                  placeholder={
                    room.caps.chat
                      ? 'Say something, or a !command'
                      : 'A !command — !start, !bSet, !map'
                  }
                  names={() => b().members}
                  onSend={(line) => void send(line)}
                />
              </div>
            </div>

            <Setup />
          </div>
        </section>
      )}
    </Show>
  )
}

/**
 * Start boxes over the map, when we can find its picture, and over a plain
 * square when we cannot. `ADDSTARTRECT` is normalised to 0-200 on both axes,
 * so the boxes are right either way and the image is decoration on top.
 */
function Minimap(props: {
  rects: StartRectView[]
  mapName: string
  teams: number
  /** Opens the large map, where the boxes can be drawn. */
  onOpen: () => void
}) {
  const room = useRoom()
  /**
   * The modoption boxes, which are a different system from `props.rects`.
   *
   * Asked for again whenever the team count changes — that is what selects an
   * arrangement out of the map's set — or whenever the start-box modoptions
   * themselves do, which is what a passed `!bSet` vote changes. Watching only
   * the team count would leave the old boxes drawn over the new arrangement.
   * `null` means the modoptions say nothing and the start rects are the whole
   * story.
   */
  const [boxes] = createResource(
    () =>
      [
        props.teams > 0 ? props.teams : 1,
        boxSignature(room.my()?.scriptTags),
      ] as const,
    ([teams]) => room.io.startBoxes(teams).catch(() => null),
  )

  return (
    <div
      class='minimap'
      role='button'
      tabIndex={0}
      title='Open the map and edit the start boxes'
      onClick={props.onOpen}
      onKeyDown={(event) => {
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault()
          props.onOpen()
        }
      }}
    >
      <MapPicture
        class='map-under'
        mapName={props.mapName}
        width={TILES.minimap.width}
        height={TILES.minimap.height}
      />
      <svg viewBox='0 0 200 200' role='img'>
        <title>Start boxes</title>
        {/* The modoption arrangement, when one applies. Drawn under the
            protocol rects so that a room using both shows which is which. */}
        <For each={boxes()?.polys ?? []}>
          {(poly, index) => (
            <g class='mm-box meta'>
              <path d={outline(poly)} />
              <text x={centre(poly).x} y={centre(poly).y + 6}>
                {index() + 1}
              </text>
            </g>
          )}
        </For>
        <For each={props.rects}>
          {(rect) => (
            <g class='mm-box'>
              <rect
                x={rect.left}
                y={rect.top}
                width={rect.right - rect.left}
                height={rect.bottom - rect.top}
              />
              <text
                x={(rect.left + rect.right) / 2}
                y={(rect.top + rect.bottom) / 2 + 6}
              >
                {rect.allyTeam + 1}
              </text>
            </g>
          )}
        </For>
      </svg>
      {/* Which system the game will actually read, since a room can carry
          both and they need not agree. */}
      <Show
        when={boxes()}
        fallback={
          <Show when={props.rects.length > 0}>
            <span class='mm-tag'>start boxes · {props.rects.length}</span>
          </Show>
        }
      >
        {(resolved) => (
          <span
            class='mm-tag'
            title={
              resolved().source === 'override'
                ? 'Set for this room, overriding the map'
                : `The map's own boxes for ${resolved().teams} teams`
            }
          >
            {resolved().source === 'override' ? 'custom' : 'map'} boxes ·{' '}
            {resolved().polys.length}
          </span>
        )}
      </Show>
    </div>
  )
}

/**
 * What the room needs and this machine lacks. Empty while the answer has not
 * arrived, which reads the same as having everything and is why the caller
 * that offers a download asks whether the content is known at all.
 */
function missingParts(room: RoomModel): readonly string[] {
  const content = room.content()
  if (!content) return []
  return (['engine', 'game', 'map'] as const).filter((part) => !content[part])
}

function Chips(props: { battle: BattleView }) {
  const room = useRoom()
  const parts = createMemo(() => missingParts(room))

  return (
    <div class='chips'>
      <Show when={room.running()}>
        <span class='chip running'>In game</span>
      </Show>
      {/* Nothing until the content has been looked at: an empty answer and a
          complete one are the same list, and only one of them is good news. */}
      <Show when={room.content()}>
        <Show
          when={parts().length > 0}
          fallback={<span class='chip ok'>Content ready</span>}
        >
          <Missing parts={parts()} engineVersion={props.battle.engineVersion} />
        </Show>
      </Show>
      <Show when={props.battle.layout}>
        {(layout) => (
          <span class='chip info'>
            {layout().teams} × {layout().teamSize}
          </span>
        )}
      </Show>
      <Show when={props.battle.locked}>
        <span class='chip warn'>Locked</span>
      </Show>
      <Show when={props.battle.passworded}>
        <span class='chip warn'>Passworded</span>
      </Show>
      <span class='chip'>
        {props.battle.playerCount} players · {props.battle.spectatorCount}{' '}
        spectators
      </span>
    </div>
  )
}

/**
 * What the room needs and this machine lacks, with the means to fetch it.
 *
 * The runtime starts this by itself on joining — a room whose map you do not
 * have is a room you cannot do anything in — so the button is for a retry
 * after a failure, and the stop is for when you would rather not.
 * pr-downloader ships inside an engine, so an engine we do not have is the one
 * thing it cannot fetch; that download is modlobby's own, offered here for the
 * room's version. Once it lands the runtime checks again and fetches the rest.
 */
function Missing(props: { parts: readonly string[]; engineVersion: string }) {
  const room = useRoom()
  const download = () => lobby.download
  const fetchable = () => !props.parts.includes('engine')
  const auto = () => settings()?.play.autoDownload ?? true

  async function start() {
    try {
      await room.io.downloadMissing()
    } catch (error) {
      pushNotice('warning', describeError(error))
    }
  }

  return (
    <>
      <span class='chip warn'>Missing {props.parts.join(', ')}</span>
      <Switch>
        <Match when={download().state === 'running'}>
          {(() => {
            const running = () =>
              download() as Extract<DownloadStatus, { state: 'running' }>
            const percent = () =>
              Math.round((downloadFraction(running()) ?? 0) * 100)
            // The chip is the glass here; the arrow inside carries none.
            return (
              <>
                <span
                  class='chip running sync-chip'
                  title={running().what}
                  style={{ '--fill': `${percent()}%` }}
                >
                  <SyncIcon fraction={null} running label='Downloading' />
                  {percent()}%
                </span>
                <button onClick={() => void api.stopDownload()}>Stop</button>
              </>
            )
          })()}
        </Match>
        <Match when={download().state === 'failed'}>
          <span class='chip warn'>Download failed</span>
          <button class='chip-choice' onClick={start}>
            Retry
          </button>
        </Match>
        <Match when={!fetchable()}>
          <GetEngine version={props.engineVersion} auto={auto()} />
        </Match>
        <Match when={!auto()}>
          <button class='chip-choice' onClick={start}>
            Download
          </button>
        </Match>
      </Switch>
    </>
  )
}

function Line(props: { line: ChatLine }) {
  return (
    <div
      class={`line ${props.line.kind}`}
      classList={{ named: props.line.mention }}
    >
      <span class='at'>{clock(props.line.at)}</span>
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
