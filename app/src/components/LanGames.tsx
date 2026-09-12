import { For, Show, createMemo } from 'solid-js'
import type { LanGameView } from '../ipc/bindings/LanGameView'
import { api, describeError } from '../ipc/client'
import { pushNotice } from '../store/chat'
import { lobby } from '../store/lobby'

/** The port the engine listens on unless told otherwise; `recoil::script`. */
const DEFAULT_PORT = 8452

/**
 * Playing with the people in the building.
 *
 * One strip above the skirmish room, and deliberately one: a LAN game is the
 * same room with the same teams, opened on a port, so it belongs where that
 * room is rather than in a page of its own with a tab in the nav. Nothing else
 * in the app knows it exists — a guest projects into the roster as a player,
 * so the seat bar, the team cards and the drag-to-a-team gesture all draw and
 * move them without having been told there is such a thing as a guest.
 *
 * It draws as little as it can get away with. The one thing always here is the
 * switch, because a feature nobody can find is not a feature; everything else
 * — who else is on the network, what they are hosting — appears only when
 * there is some. A machine on a network with no modlobby on it sees one line.
 *
 * This matters most on macOS, where the only engine that exists may not reach
 * Beyond All Reason's servers. There, this is the whole of multiplayer.
 */
export function LanGames() {
  const open = () => lobby.skirmish?.battle.port || null
  const me = () => lobby.me ?? lobby.skirmish?.me ?? ''

  /** The people the room already expects: its members, minus you. */
  const expected = createMemo(() =>
    (lobby.skirmish?.battle.members ?? []).filter((name) => name !== me()),
  )

  const others = createMemo(() => {
    const here = new Set(expected())
    return lobby.lan.people.filter((name) => name !== me() && !here.has(name))
  })

  async function act(run: Promise<unknown>) {
    try {
      await run
    } catch (error) {
      pushNotice('warning', describeError(error))
    }
  }

  const toggle = () =>
    act(
      api.skirmishAct({
        type: 'setLan',
        port: open() ? null : DEFAULT_PORT,
      }),
    )

  /** Expects somebody. Where they sit is the room's to decide: a side of
      their own, which is the same place a new AI goes. */
  const invite = (name: string) =>
    act(api.skirmishAct({ type: 'addGuest', name }))

  const uninvite = (name: string) =>
    act(api.skirmishAct({ type: 'removeGuest', name }))

  return (
    <div class='lan'>
      <div class='lan-row'>
        <button class='chip-choice' onClick={() => void toggle()}>
          {open() ? 'Close to the network' : 'Play over the network'}
        </button>
        <Show when={open()}>
          {(port) => (
            <span class='muted lan-say'>
              Open on port {port()}. Add the people who will join — the engine
              only lets in names this room knows.
            </span>
          )}
        </Show>
        {/* Said only where it is a problem: a machine with no LAN game near it
            has no reason to hear that the socket is fine, and a machine that
            could not get one would otherwise see an empty list and read it as
            "nobody is playing". */}
        <Show when={!lobby.lan.listening && open()}>
          <span
            class='chip warn'
            title={`modlobby could not listen on UDP ${DEFAULT_PORT + 1}`}
          >
            Not announcing
          </span>
        </Show>
      </div>

      <Show when={open() && (others().length > 0 || expected().length > 0)}>
        <div class='lan-row'>
          <Show when={expected().length > 0}>
            <span class='muted lan-say'>Expecting</span>
            <For each={expected()}>
              {(name) => (
                <button
                  class='chip-choice'
                  title={`Stop expecting ${name}`}
                  onClick={() => void uninvite(name)}
                >
                  {name} ×
                </button>
              )}
            </For>
          </Show>
          {/* Picked off a list rather than spelled: a name spelled wrong is a
              guest the engine's server will not let in, and the mistake only
              shows up as a refused connection on the other machine. */}
          <Show when={others().length > 0}>
            <span class='muted lan-say'>Also here</span>
            <For each={others()}>
              {(name) => (
                <button
                  class='chip-choice'
                  title={`Expect ${name} to join`}
                  onClick={() => void invite(name)}
                >
                  + {name}
                </button>
              )}
            </For>
          </Show>
        </div>
      </Show>

      <Show when={lobby.lan.games.length > 0}>
        <ul class='lan-games'>
          <For each={lobby.lan.games}>
            {(game) => <Game game={game} me={me()} />}
          </For>
        </ul>
      </Show>
    </div>
  )
}

/** What of a game's engine, game and map this machine has not got. */
function missing(game: LanGameView): string[] {
  return [
    ['engine', game.content.engine],
    ['game', game.content.game],
    ['map', game.content.map],
  ]
    .filter(([, have]) => !have)
    .map(([what]) => what as string)
}

/**
 * One game somebody is hosting.
 *
 * Three reasons a row cannot be joined, and each is said rather than drawn as
 * a dead button: the game has started, this machine is short of the content,
 * or the host has not written your name into it. The last one is the engine's
 * own rule — its server admits a name only if the start script lists it — so
 * saying it here is the difference between "ask them to add you" and a
 * connection that is refused with nothing on screen to explain it.
 */
function Game(props: { game: LanGameView; me: string }) {
  const short = () => missing(props.game)
  const invited = () =>
    props.game.seats.length === 0 || props.game.seats.includes(props.me)
  const why = () => {
    if (props.game.running) return 'already started'
    if (short().length > 0) return `you are missing the ${short().join(', ')}`
    if (!invited()) return `${props.game.host} has not added you`
    return null
  }

  const join = async () => {
    try {
      await api.joinLanGame(props.game.id)
    } catch (error) {
      pushNotice('warning', describeError(error))
    }
  }

  return (
    <li class='lan-game'>
      <span class='lan-title'>{props.game.title}</span>
      <span class='muted'>
        {props.game.host} · {props.game.map}
      </span>
      <span
        class='muted lan-where'
        title={`${props.game.address} · ${props.game.game}`}
      >
        {props.game.address}
      </span>
      <Show
        when={why()}
        fallback={
          <button class='chip-choice' onClick={() => void join()}>
            Join
          </button>
        }
      >
        {(reason) => <span class='chip warn'>{reason()}</span>}
      </Show>
    </li>
  )
}
