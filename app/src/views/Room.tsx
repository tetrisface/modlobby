import { useNavigate } from '@solidjs/router'
import {
  For,
  Match,
  Show,
  Switch,
  createEffect,
  createMemo,
  createResource,
  createSignal,
} from 'solid-js'
import { Composer } from '../components/Composer'
import { Linkify } from '../components/Linkify'
import { showPlayerMenu } from '../components/PlayerMenu'
import { BotRow, PlayerRow, SpectatorRow } from '../components/PlayerRow'
import type { BattleView } from '../ipc/bindings/BattleView'
import type { BotView } from '../ipc/bindings/BotView'
import type { ChatLine } from '../ipc/bindings/ChatLine'
import type { DownloadStatus } from '../ipc/bindings/DownloadStatus'
import type { StartRectView } from '../ipc/bindings/StartRectView'
import type { UserView } from '../ipc/bindings/UserView'
import { api, describeError } from '../ipc/client'
import { boxSignature, centre, outline } from '../lib/boxes'
import { mapImage, sized } from '../lib/maps'
import { readSkills, teamSkill, type Skill } from '../lib/skill'
import { BATTLE_ROOM, chat, pushNotice } from '../store/chat'
import { lobby } from '../store/lobby'
import { settings } from '../store/settings'
import { HostBar } from './HostBar'
import { PveScore } from './PveScore'
import { Seat } from './Seat'
import { StartBoxes } from './StartBoxes'
import { Setup } from './Setup'
import { VoteBar } from './VoteBar'

type Team = { allyTeam: number; users: UserView[]; bots: BotView[] }

export function Room() {
  const navigate = useNavigate()
  let log: HTMLDivElement | undefined

  const battle = createMemo(() => {
    const id = lobby.myBattle?.id
    return id === undefined ? undefined : lobby.battles[id]
  })

  const friends = createMemo(() => new Set(lobby.friends.friends))
  const isFriend = (name: string) => friends().has(name)

  /** SPADS keys its player tags by lowercased name. */
  const skills = createMemo(() => readSkills(lobby.myBattle?.scriptTags))
  const skillOf = (name: string): Skill | null =>
    skills()[name.toLowerCase()] ?? null

  const occupants = createMemo(() => {
    const b = battle()
    if (!b) return { teams: [] as Team[], spectators: [] as UserView[] }

    const teams = new Map<number, Team>()
    const team = (allyTeam: number) => {
      const found = teams.get(allyTeam) ?? { allyTeam, users: [], bots: [] }
      teams.set(allyTeam, found)
      return found
    }

    const spectators: UserView[] = []
    // `members` arrives sorted by name; keeping that order means a skill tag
    // landing late never reshuffles the list under the reader's eyes.
    for (const name of b.members) {
      const user = lobby.users[name]
      if (!user) continue
      if (user.battleStatus?.player)
        team(user.battleStatus.allyTeam).users.push(user)
      else spectators.push(user)
    }
    for (const bot of b.bots) team(bot.status.allyTeam).bots.push(bot)

    return {
      teams: [...teams.values()].sort((a, b) => a.allyTeam - b.allyTeam),
      spectators,
    }
  })

  createEffect(() => {
    if (lobby.phase === 'ready' && !lobby.myBattle)
      navigate('/battles', { replace: true })
  })
  const lines = () => chat.rooms[BATTLE_ROOM] ?? []
  createEffect(() => {
    lines().length
    log?.scrollTo({ top: log.scrollHeight })
  })

  async function send(line: string) {
    try {
      await api.sayBattle(line.trim())
    } catch (error) {
      pushNotice('warning', describeError(error))
    }
  }

  async function launch() {
    try {
      await api.launch()
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
            />
            <div class='card-main'>
              <h1 title={b().title}>{b().title}</h1>
              <div class='card-meta'>
                <span>
                  Map{' '}
                  {/* The page Chobby opens for a map, so the link lands where
                      people already expect it to. */}
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
              <Show when={lobby.gameRunning}>
                {/* While our own engine runs, the useful button is not another
                    launch — it is the way back to the game the lobby is
                    sitting on top of. */}
                <Show
                  when={lobby.engine.state === 'running'}
                  fallback={
                    <button class='primary' onClick={launch}>
                      Watch the game
                    </button>
                  }
                >
                  <button
                    class='primary'
                    title={`Or press ${settings()?.overlay.hotkey ?? 'the overlay shortcut'}`}
                    onClick={() => void api.overlayToggle()}
                  >
                    Back to game
                  </button>
                </Show>
              </Show>
              <button onClick={() => api.leaveBattle()}>Leave</button>
            </div>
          </header>

          <PveScore />
          <VoteBar teams={Math.max(occupants().teams.length, 2)} />
          <StartBoxes
            teams={Math.max(occupants().teams.length, 2)}
            mapName={b().mapName}
          />
          <HostBar />
          <Seat />

          <div class='room-body'>
            <div class='room-main'>
              <div class='rosters'>
                <div class='teams'>
                  <For each={occupants().teams}>
                    {(team) => (
                      <section class='team'>
                        <header class='team-head'>
                          <span class='name'>Team {team.allyTeam + 1}</span>
                          <span class='count'>
                            {team.users.length + team.bots.length}
                          </span>
                          <Show when={team.users.length > 0}>
                            <span class='os'>
                              Σ{' '}
                              {teamSkill(
                                team.users.map((u) => skillOf(u.name)),
                              ).toFixed(1)}
                            </span>
                          </Show>
                        </header>
                        <For each={team.users}>
                          {(user) => (
                            <PlayerRow
                              user={user}
                              skill={skillOf(user.name)}
                              me={user.name === lobby.me}
                              friend={isFriend(user.name)}
                            />
                          )}
                        </For>
                        <For each={team.bots}>
                          {(bot) => (
                            <BotRow
                              bot={bot}
                              onRemove={
                                // The server would refuse anyone else; not
                                // drawing the button beats a silent refusal.
                                bot.owner === lobby.me ||
                                lobby.myBattle?.boss === lobby.me
                                  ? () =>
                                      void api
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
                      </section>
                    )}
                  </For>
                </div>

                <section class='spectators'>
                  <header class='team-head'>
                    <span class='name'>Spectators</span>
                    <span class='count'>{occupants().spectators.length}</span>
                  </header>
                  <div class='spectator-list'>
                    <For each={occupants().spectators}>
                      {(user) => (
                        <SpectatorRow
                          user={user}
                          me={user.name === lobby.me}
                          friend={isFriend(user.name)}
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
                  placeholder='Say something, or a !command'
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
}) {
  const [image] = createResource(
    () => props.mapName,
    async (name) => {
      const url = await mapImage(name)
      // The minimap column is 132px square; ask for it at that scale rather
      // than downscaling a 1024px picture into it.
      return url === null ? null : sized(url, 384)
    },
  )
  const [broken, setBroken] = createSignal(false)

  // A room changes map, so a picture that failed must not condemn the next one.
  createEffect(() => {
    void props.mapName
    setBroken(false)
  })

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
        boxSignature(lobby.myBattle?.scriptTags),
      ] as const,
    ([teams]) => api.startBoxes(teams).catch(() => null),
  )

  return (
    <div class='minimap'>
      <Show when={broken() ? undefined : image()}>
        {(url) => (
          <img
            class='mm-photo'
            src={url()}
            alt=''
            onError={() => setBroken(true)}
          />
        )}
      </Show>
      <svg viewBox='0 0 200 200' role='img'>
        <title>Start boxes</title>
        <Show when={broken() || !image()}>
          <rect width='200' height='200' class='mm-ground' />
          {/* A quarter grid, so an empty square reads as a schematic waiting
              for a map rather than as a panel that failed to load. */}
          <path
            class='mm-grid'
            d='M50 0V200M100 0V200M150 0V200M0 50H200M0 100H200M0 150H200'
          />
        </Show>
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

function Chips(props: { battle: BattleView }) {
  const missing = createMemo(() => {
    const content = lobby.content
    if (!content) return null
    return (['engine', 'game', 'map'] as const).filter((part) => !content[part])
  })

  return (
    <div class='chips'>
      <Show when={lobby.gameRunning}>
        <span class='chip running'>In game</span>
      </Show>
      <Show when={missing()}>
        {(parts) => (
          <Show
            when={parts().length > 0}
            fallback={<span class='chip ok'>Content ready</span>}
          >
            <Missing parts={parts()} />
          </Show>
        )}
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
 * thing this cannot fix — it says so rather than offering a button that fails.
 */
function Missing(props: { parts: string[] }) {
  const download = () => lobby.download
  const fetchable = () => props.parts.some((part) => part !== 'engine')

  async function start() {
    try {
      await api.downloadMissing()
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
              running().total > 0
                ? Math.round((running().current / running().total) * 100)
                : 0
            return (
              <>
                <span class='chip info' title={running().what}>
                  Downloading {percent()}%
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
        <Match when={fetchable()}>
          <button class='chip-choice' onClick={start}>
            Download
          </button>
        </Match>
        <Match when={!fetchable()}>
          <span class='chip'>Install an engine to fetch content</span>
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
