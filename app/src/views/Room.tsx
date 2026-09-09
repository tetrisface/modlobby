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
import { GameActions } from '../components/GameActions'
import { GetEngine } from '../components/GetEngine'
import { Linkify } from '../components/Linkify'
import { MapEditor } from '../components/MapEditor'
import { MapPicker, VersionPicker, picked } from '../components/MapPicker'
import { BotOptions } from '../components/BotOptions'
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
import {
  type Roster,
  arrange,
  emptySeats,
  freeTeam,
  unusedBotName,
} from '../lib/roster'
import { readSkills, teamSkill, type Skill } from '../lib/skill'
import { noPublishedEngine } from '../store/build'
import { chat, pushNotice } from '../store/chat'
import { joinMilestone } from '../store/join'
import { lobby } from '../store/lobby'
import { over } from '../store/overlay'
import { settings } from '../store/settings'
import { HostBar } from './HostBar'
import { PveScore } from './PveScore'
import { RoomTitle } from './RoomTitle'
import { useRoom, type RoomModel } from './room/model'
import { dragging } from '../lib/drag'
import { Seat, sitOn } from './Seat'
import { movable, moveTo, setBonus, type Target } from './room/move'
import { StartBoxes } from './StartBoxes'
import { Setup } from './Setup'
import { VoteBar } from './VoteBar'

/** `startpostype`, by the names Chobby gives the three. */
const START_POS = [
  { id: 2, label: 'In the boxes' },
  { id: 0, label: "The map's own" },
  { id: 1, label: 'Random' },
]

/** Where the only Beyond All Reason engine for Apple Silicon is published. */
const APPLE_ENGINE =
  'https://github.com/Vandomas/RecoilEngine-AppleSilicon/releases'

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
    // A room has a name because it is listed for other people to read. Where
    // it is not listed there is nobody to name it for, so the pen stays away
    // rather than offering to change a label only you will ever see.
    if (!room.caps.spads) return false
    const me = room.me()
    if (me === null) return false
    if (room.my()?.boss === me) return true
    return room.users()[me]?.battleStatus?.player ?? false
  })

  /** Every team drawn, which is where a dragged row may be dropped. */
  const allyTeams = () => occupants().teams.map((team) => team.allyTeam)

  /**
   * What a row may do about where it sits, or nothing where it may do nothing.
   *
   * Handed to the row rather than worked out there: which teams exist is the
   * room's question, and the answer is the same for the drag and for the menu.
   */
  const movesFor = (target: Target, on: number | null) => {
    if (!movable(room, target)) return undefined
    const say = (work: Promise<void>) =>
      work.catch((error) => pushNotice('warning', describeError(error)))
    return {
      teams: allyTeams(),
      on,
      to: (ally: number) => say(moveTo(room, target, ally)),
      ...(target.kind === 'bot'
        ? {
            bonus: (percent: number) =>
              say(setBonus(room, target, percent, on ?? 0)),
            bonusNow: target.handicap,
          }
        : {}),
    }
  }

  /** A team's header offers a seat on it unless we already hold one there. */
  const canJoin = (allyTeam: number): boolean => {
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
  /** The AI whose own options are open over the room, if any. */
  const [botOptions, setBotOptions] = createSignal<string | null>(null)
  /** Which of the room's three choices is being made, if any. */
  const [picking, setPicking] = createSignal<'map' | 'game' | 'engine' | null>(
    null,
  )

  const lines = () => chat.rooms[room.log] ?? []
  createEffect(() => {
    lines().length
    log?.scrollTo({ top: log.scrollHeight })
  })

  /** The AI a name points at, while it is still in the room. */
  const botOf = (name: string | null) =>
    name === null
      ? undefined
      : room.battle()?.bots.find((bot) => bot.name === name)

  /** Where players start, out of the room's script tags. */
  const startPos = () => Number(room.my()?.scriptTags['game/startpostype'] ?? 2)

  /**
   * A version, as a thing to change where it is ours to change and as a plain
   * statement of fact where it is not.
   *
   * A room that has not been told which it wants shows the invitation rather
   * than nothing: an empty link is a click target with no width, which reads
   * as dead text beside its label. The same hole opens on the other side — a
   * fact with no value is a label with nothing after it — which is what
   * `absent` fills where the caller has something to put there. The two do not
   * share one placeholder, because one of them is an invitation and the other
   * is news.
   *
   * `picks` is asked for rather than read here, because the room's permission
   * is only half of it: an engine this machine can only be given by hand is a
   * fact about the machine, and a list of the one engine on the disk is not a
   * choice.
   */
  const Choice = (props: {
    what: 'game' | 'engine'
    shown: string
    picks: boolean
    absent?: string
  }) => (
    <Show when={props.picks} fallback={<b>{props.shown || props.absent}</b>}>
      <b
        class='chat-link'
        title={`Play a different ${props.what}`}
        onClick={() => setPicking(props.what)}
      >
        {props.shown || 'choose one'}
      </b>
    </Show>
  )

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
            <Show when={botOf(botOptions())}>
              {(bot) => (
                <BotOptions bot={bot()} onClose={() => setBotOptions(null)} />
              )}
            </Show>
            <Show when={picking() === 'map'}>
              <MapPicker
                current={b().mapName}
                onPick={(name) => {
                  setPicking(null)
                  void picked(room.io.setMap, 'map', name)
                }}
                onClose={() => setPicking(null)}
              />
            </Show>
            <Show when={picking() === 'game' || picking() === 'engine'}>
              <VersionPicker
                what={picking() === 'game' ? 'Game' : 'Engine'}
                current={
                  picking() === 'game' ? b().gameName : b().engineVersion
                }
                onPick={(version) => {
                  const what = picking()
                  setPicking(null)
                  if (what === null) return
                  void picked(
                    (name) => room.io.sayBattle(`!${what} ${name}`),
                    what,
                    version,
                  )
                }}
                onClose={() => setPicking(null)}
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
                      onClick={() => setPicking('map')}
                    >
                      {b().mapName || 'choose one'}
                    </b>
                  </Show>
                </span>
                {/* Whose room it is, which is only worth saying when it is
                    somebody else's. */}
                <Show when={room.caps.spads}>
                  <span>
                    Host <b>{b().founder}</b>
                  </span>
                </Show>
                <span>
                  Engine{' '}
                  {/* Where nothing publishes an engine for this machine, the
                      one on the disk is not a choice: it is the only one there
                      is, and somebody put it there by hand. Said as a fact
                      instead of offered as a list of one. */}
                  <Choice
                    what='engine'
                    shown={b().engineVersion}
                    picks={room.caps.picksContent && !noPublishedEngine()}
                    absent='none installed'
                  />
                </span>
                <span>
                  Game{' '}
                  <Choice
                    what='game'
                    shown={b().gameName}
                    picks={room.caps.picksContent}
                  />
                </span>
                {/* A `[game]` key rather than a modoption, so it has no row in
                    the settings table and belongs here with the rest of what
                    the game is played on. */}
                <Show when={room.caps.picksContent}>
                  <label class='card-choice'>
                    Start
                    <select
                      value={startPos()}
                      onChange={(event) =>
                        void room.io
                          .setStartPos(Number(event.currentTarget.value))
                          .catch((error) =>
                            pushNotice('warning', describeError(error)),
                          )
                      }
                    >
                      <For each={START_POS}>
                        {(kind) => (
                          <option value={String(kind.id)}>{kind.label}</option>
                        )}
                      </For>
                    </select>
                  </label>
                </Show>
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
                    title={
                      over()
                        ? 'Or press Escape'
                        : `Or press ${settings()?.overlay.hotkey ?? 'the overlay shortcut'}`
                    }
                    onClick={() => void api.overlayToggle()}
                  >
                    Back to game
                  </button>
                </Match>
                <Match when={room.running() && room.caps.plays}>
                  <button class='primary' onClick={launch}>
                    Watch the game
                  </button>
                </Match>
                {/* The game is on, and this machine has no engine allowed to
                    join it. Said rather than left as an empty corner. */}
                <Match when={room.running()}>
                  <span class='muted'>Game in progress</span>
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
                <button onClick={() => room.io.leaveBattle()}>
                  Leave room
                </button>
              </Show>
              {/* Ending the game belongs beside leaving the room: one column
                  for every way out, rather than two corners of the window
                  offering much the same thing. */}
              <Show when={lobby.engine.state === 'running'}>
                <GameActions />
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
                <div class='teams' classList={{ dropping: dragging() }}>
                  {/* By position, not by object: the memo builds new team
                      objects on every status line, and a `For` keyed on
                      them rebuilt every row each time. The users inside are
                      the store's own objects, so their rows do survive. */}
                  <Index each={occupants().teams}>
                    {(team) => (
                      <section class='team' data-ally={team().allyTeam}>
                        <header class='team-head'>
                          <span class='name'>Team {team().allyTeam + 1}</span>
                          <span class='count'>{team().expected}</span>
                          {/* Nothing to say where nobody is rated, which is
                              every skirmish and any room whose skills have
                              not arrived yet. A sum of zero is not a fact
                              about the team. */}
                          <Show
                            when={teamSkill(
                              team().users.map((u) => skillOf(u.name)),
                            )}
                          >
                            {(sum) => (
                              <span class='os'>Σ {sum().toFixed(1)}</span>
                            )}
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
                              moves={movesFor(
                                user.name === room.me()
                                  ? { kind: 'me' }
                                  : { kind: 'player', name: user.name },
                                team().allyTeam,
                              )}
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
                              onOptions={
                                room.caps.picksContent
                                  ? () => setBotOptions(bot.name)
                                  : undefined
                              }
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
                              onClone={
                                bot.owner === room.me()
                                  ? () =>
                                      room.io
                                        .addBot(
                                          unusedBotName(room.battle(), bot.ai),
                                          bot.ai,
                                          freeTeam(
                                            b(),
                                            room.users(),
                                            room.me(),
                                          ),
                                          bot.status.allyTeam,
                                          bot.teamColour,
                                        )
                                        .catch((error) =>
                                          pushNotice(
                                            'warning',
                                            describeError(error),
                                          ),
                                        )
                                  : undefined
                              }
                              moves={movesFor(
                                {
                                  kind: 'bot',
                                  name: bot.name,
                                  mine: bot.owner === room.me(),
                                  team: bot.status.team,
                                  handicap: bot.status.handicap,
                                  colour: bot.teamColour,
                                },
                                team().allyTeam,
                              )}
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

/** `1 player`, `2 players` — a count with its word, agreeing with it. */
function count(n: number, what: string): string {
  return `${n} ${what}${n === 1 ? '' : 's'}`
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
        {count(props.battle.playerCount, 'player')} ·{' '}
        {count(props.battle.spectatorCount, 'spectator')}
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
      {/* The fallback is the point: with auto-download on and the engine
          already here, none of the arms matched and the row offered nothing
          at all — a dead end one second after the engine landed. */}
      <Switch
        fallback={
          <button class='chip-choice' onClick={start}>
            Download
          </button>
        }
      >
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
          <Switch
            fallback={<GetEngine version={props.engineVersion} auto={auto()} />}
          >
            {/* An index with no build for this machine is not a failure to
                retry: it is the answer, and the way past it is not in this
                app. Ahead of the empty-version arm because a first run here
                has neither a version nor an engine, and "this room names no
                engine" would blame the room for a fact about the machine.
                Three buttons because the instruction has three steps and each
                of them is a thing this app can do. */}
            <Match when={noPublishedEngine()}>
              {(why) => (
                <>
                  <button
                    class='chip-choice'
                    onClick={() =>
                      void api
                        .openUrl(APPLE_ENGINE)
                        .catch((error) =>
                          pushNotice('warning', describeError(error)),
                        )
                    }
                  >
                    Get one
                  </button>
                  <button
                    class='chip-choice'
                    onClick={() =>
                      void api
                        .openEngineDir()
                        .catch((error) =>
                          pushNotice('warning', describeError(error)),
                        )
                    }
                  >
                    Engine folder
                  </button>
                  {/* What is installed is cached against the room's three
                      names, and dropping a bundle into the folder changes none
                      of them — so an engine that arrived from outside this app
                      is found only by being asked for. */}
                  <button
                    class='chip-choice'
                    onClick={() =>
                      void api
                        .recheckContent()
                        .catch((error) =>
                          pushNotice('warning', describeError(error)),
                        )
                    }
                  >
                    Look again
                  </button>
                  <span class='muted chip-say'>{why()}</span>
                </>
              )}
            </Match>
            {/* Asking BAR's index for the engine called "" answers 404, and a
                retry asks the same question again. Saying so is the honest end
                of that road until the room is given a version. */}
            <Match when={!props.engineVersion}>
              <span class='chip warn'>This room names no engine to fetch</span>
            </Match>
          </Switch>
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
