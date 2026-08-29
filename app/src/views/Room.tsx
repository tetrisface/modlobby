import { useNavigate } from '@solidjs/router'
import {
  For,
  Show,
  createEffect,
  createMemo,
  createResource,
  createSignal,
} from 'solid-js'
import { BotRow, PlayerRow, SpectatorRow } from '../components/PlayerRow'
import type { BattleView } from '../ipc/bindings/BattleView'
import type { BotView } from '../ipc/bindings/BotView'
import type { ChatLine } from '../ipc/bindings/ChatLine'
import type { StartRectView } from '../ipc/bindings/StartRectView'
import type { UserView } from '../ipc/bindings/UserView'
import { api, describeError } from '../ipc/client'
import { mapImage } from '../lib/maps'
import { readSkills, teamSkill, type Skill } from '../lib/skill'
import { BATTLE_ROOM, chat, pushNotice } from '../store/chat'
import { lobby } from '../store/lobby'
import { Seat } from './Seat'
import { Setup } from './Setup'
import { VoteBar } from './VoteBar'

type Team = { allyTeam: number; users: UserView[]; bots: BotView[] }

export function Room() {
  const navigate = useNavigate()
  const [text, setText] = createSignal('')
  // Setup asks for the whole body when you are editing a tweak; the rosters
  // step aside rather than the editor being squeezed into a rail.
  const [wide, setWide] = createSignal(false)
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

  async function send(event: Event) {
    event.preventDefault()
    const line = text().trim()
    if (!line) return
    try {
      await api.sayBattle(line)
      setText('')
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
            <Minimap rects={b().startRects} mapName={b().mapName} />
            <div class='card-main'>
              <h1 title={b().title}>{b().title}</h1>
              <div class='card-meta'>
                <span>
                  Map <b>{b().mapName}</b>
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
                <button
                  class='primary'
                  disabled={lobby.engine.state === 'running'}
                  onClick={launch}
                >
                  {lobby.engine.state === 'running'
                    ? 'Engine running'
                    : 'Watch the game'}
                </button>
              </Show>
              <button onClick={() => api.leaveBattle()}>Leave</button>
            </div>
          </header>

          <VoteBar />
          <Seat />

          <div class='room-body' classList={{ wide: wide() }}>
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
                          {(bot) => <BotRow bot={bot} />}
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

              <div class='chat'>
                <div class='chat-log' ref={log}>
                  <For each={lines()}>{(line) => <Line line={line} />}</For>
                </div>
                <form class='chat-input' onSubmit={send}>
                  <input
                    value={text()}
                    onInput={(e) => setText(e.currentTarget.value)}
                    placeholder='Say something, or a !command'
                  />
                  <button type='submit'>Send</button>
                </form>
              </div>
            </div>

            <Setup wide={wide()} onWide={setWide} />
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
function Minimap(props: { rects: StartRectView[]; mapName: string }) {
  const [image] = createResource(
    () => props.mapName,
    (name) => mapImage(name),
  )
  const [broken, setBroken] = createSignal(false)

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
      <Show when={props.rects.length > 0}>
        <span class='mm-tag'>start boxes · {props.rects.length}</span>
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
            <span class='chip warn'>Missing {parts().join(', ')}</span>
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

function Line(props: { line: ChatLine }) {
  return (
    <div class={`line ${props.line.kind}`}>
      <span class='from'>{props.line.from}</span>
      <span class='text'>{props.line.text}</span>
    </div>
  )
}
