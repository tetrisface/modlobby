import { A, useNavigate } from '@solidjs/router'
import { For, Show, createEffect, createMemo, createSignal } from 'solid-js'
import type { ChatLine } from '../ipc/bindings/ChatLine'
import { api, describeError } from '../ipc/client'
import { chat, pushNotice } from '../store/chat'
import { lobby } from '../store/lobby'
import { Seat } from './Seat'
import { VoteBar } from './VoteBar'

export function Room() {
  const navigate = useNavigate()
  const [text, setText] = createSignal('')
  let log: HTMLDivElement | undefined

  const battle = createMemo(() => {
    const id = lobby.myBattle?.id
    return id === undefined ? undefined : lobby.battles[id]
  })

  const members = createMemo(() => {
    const b = battle()
    if (!b) return { players: [], spectators: [] }
    const users = b.members
      .map((name) => lobby.users[name])
      .filter((u) => u !== undefined)
    return {
      players: users.filter((u) => u.battleStatus?.player),
      spectators: users.filter((u) => !u.battleStatus?.player),
    }
  })

  createEffect(() => {
    if (lobby.phase === 'ready' && !lobby.myBattle)
      navigate('/battles', { replace: true })
  })
  createEffect(() => {
    chat.lines.length
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
          <header class='room-header'>
            <div>
              <h1>{b().title}</h1>
              <p class='muted'>
                {b().mapName} · {b().gameName} · engine {b().engineVersion} ·
                host {b().founder}
              </p>
            </div>
            <div class='room-actions'>
              <A href='/room/tweaks'>Tweaks</A>
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
          <div class='room-body'>
            <aside class='members'>
              <h2>Players ({members().players.length})</h2>
              <ul>
                <For each={members().players}>
                  {(u) => (
                    <li classList={{ 'in-game': u.status.inGame }}>
                      <span class='team'>
                        {u.battleStatus?.allyTeam ?? '-'}
                      </span>
                      {u.name}
                      {u.battleStatus?.ready ? ' ✓' : ''}
                    </li>
                  )}
                </For>
                <For each={b().bots}>
                  {(bot) => (
                    <li class='bot'>
                      <span class='team'>{bot.status.allyTeam}</span>
                      {bot.name}{' '}
                      <small>
                        ({bot.ai}, {bot.owner})
                      </small>
                    </li>
                  )}
                </For>
              </ul>
              <h2>Spectators ({members().spectators.length})</h2>
              <ul>
                <For each={members().spectators}>
                  {(u) => (
                    <li
                      classList={{
                        'in-game': u.status.inGame,
                        me: u.name === lobby.me,
                      }}
                    >
                      {u.name}
                    </li>
                  )}
                </For>
              </ul>
            </aside>
            <div class='chat'>
              <div class='chat-log' ref={log}>
                <For each={chat.lines}>{(line) => <Line line={line} />}</For>
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
        </section>
      )}
    </Show>
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
