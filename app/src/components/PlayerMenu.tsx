import { For, Show, createSignal, onCleanup } from 'solid-js'
import type { BotView } from '../ipc/bindings/BotView'
import { api, describeError } from '../ipc/client'
import { ensureRoom, privateRoom, pushNotice } from '../store/chat'
import { lobby } from '../store/lobby'
import { Flag, RankIcon } from './icons'

/**
 * What you can do about a person, reachable from wherever their name appears.
 *
 * Until this existed, messaging, friending and ignoring were all things you
 * could only do by typing someone's name exactly — which is no use when the
 * name you want is `[Crd]XxStormKittyxX`.
 */

/**
 * Where a row may be sent, handed in by the room rather than worked out here.
 *
 * This menu knows about people, not about rooms: which teams exist, and what
 * moving somebody costs, are the room's questions -- and a skirmish answers
 * them differently from a room on the server. Absent where a row cannot move.
 */
export type Moves = {
  /** Ally team indices to offer, in the order the room draws them. */
  teams: number[]
  /** Where they are now, so it is not offered as somewhere to go. */
  on: number | null
  to: (allyTeam: number) => Promise<void>
  /** Present only where a bonus can be set: an AI of ours, or a boss's word. */
  bonus?: (percent: number) => Promise<void>
  bonusNow?: number
}

type Target =
  | { kind: 'user'; name: string; moves?: Moves; x: number; y: number }
  /** One of our own AIs; `remove` is the one thing there is to do about it. */
  | {
      kind: 'bot'
      bot: BotView
      remove: () => Promise<void>
      moves?: Moves
      x: number
      y: number
    }

const [openFor, setOpenFor] = createSignal<Target | null>(null)

/** Opens the menu for a name at the pointer. */
export function showPlayerMenu(
  name: string,
  event: MouseEvent,
  moves?: Moves,
): void {
  event.preventDefault()
  event.stopPropagation()
  setOpenFor({ kind: 'user', name, moves, x: event.clientX, y: event.clientY })
}

/**
 * Opens the menu for an AI of ours at the pointer. Only ours: the server
 * refuses `REMOVEBOT` from anyone but the owner, the host and moderators, so
 * for another player's AI there would be nothing in the menu.
 */
export function showBotMenu(
  bot: BotView,
  remove: () => Promise<void>,
  event: MouseEvent,
  moves?: Moves,
): void {
  event.preventDefault()
  event.stopPropagation()
  setOpenFor({
    kind: 'bot',
    bot,
    remove,
    moves,
    x: event.clientX,
    y: event.clientY,
  })
}

export function PlayerMenu() {
  /** Whether the menu shows the bonus panel in place of its entries. */
  const [picking, setPicking] = createSignal(false)
  const close = () => {
    setOpenFor(null)
    setPicking(false)
  }

  let root: HTMLDivElement | undefined

  // Any press elsewhere, or Escape, dismisses it — the usual bargain for
  // something that floats above everything. "Elsewhere" is checked here, on
  // the document, rather than stopped at the menu: Solid delegates
  // `onMouseDown` to the document too, and stopping propagation there does
  // not reach a listener on the same node.
  const onDown = (event: MouseEvent) => {
    if (root?.contains(event.target as Node)) return
    close()
  }
  const onKey = (event: KeyboardEvent) => {
    if (event.key === 'Escape') close()
  }
  document.addEventListener('mousedown', onDown)
  document.addEventListener('keydown', onKey)
  onCleanup(() => {
    document.removeEventListener('mousedown', onDown)
    document.removeEventListener('keydown', onKey)
  })

  /** A SPADS command, sent the way anyone would type it into the room. */
  const say = (command: string) => api.sayBattle(command)

  async function act(what: string, run: () => Promise<void>) {
    close()
    try {
      await run()
    } catch (error) {
      pushNotice('warning', `${what}: ${describeError(error)}`)
    }
  }

  return (
    <Show when={openFor()}>
      {(target) => {
        const bot = () => {
          const t = target()
          return t.kind === 'bot' ? t : undefined
        }
        const name = () => {
          const t = target()
          return t.kind === 'bot' ? t.bot.name : t.name
        }
        const isFriend = () => lobby.friends.friends.includes(name())
        const isIgnored = () => lobby.friends.ignored.includes(name())
        const isMe = () => name() === lobby.me

        const user = () => (bot() ? undefined : lobby.users[name()])
        /**
         * The room they are in, when it is one we can see and not the one we
         * are already standing in — where they are is only news if it is
         * somewhere else.
         */
        const theirRoom = () => {
          const id = user()?.battleId
          if (id === null || id === undefined || id === lobby.myBattle?.id)
            return undefined
          return lobby.battles[id]
        }

        /** Whether SPADS would take our word for it in this room. */
        const bossing = () =>
          lobby.myBattle?.boss !== null && lobby.myBattle?.boss === lobby.me

        /** `stay`: the entry opens something in the menu, so it stays. */
        type Entry = [string, () => Promise<void> | void, 'stay'?]

        /**
         * Where this row can be sent, as words.
         *
         * The same rows a drag produces, said out loud: dragging is quicker
         * once you know it is there, and nothing tells you that it is.
         */
        const placings = (moves: Moves | undefined): Entry[] => {
          if (!moves) return []
          const rows: Entry[] = moves.teams
            .filter((ally) => ally !== moves.on)
            .map((ally) => [`Move to team ${ally + 1}`, () => moves.to(ally)])
          if (moves.bonus)
            rows.push([
              'Bonus',
              () => {
                setPicking(true)
              },
              'stay',
            ])
          return rows
        }

        const items = () => {
          const ai = bot()
          if (ai) {
            return [
              ...placings(openFor()?.moves),
              ['Remove', ai.remove] as Entry,
            ]
          }
          const entries: Entry[] = [
            [
              'Message',
              () => {
                ensureRoom(privateRoom(name()))
                location.hash = '#/chat'
              },
            ],
          ]
          // In the same room, and it is ours to run: SPADS takes these as
          // chat, so they need nothing but the words a host would type.
          const together =
            user()?.battleId !== null &&
            user()?.battleId === lobby.myBattle?.id &&
            !isMe()
          if (together) {
            entries.push(['Ring', () => api.ring(name())])
          }
          if (together) entries.push(...placings(openFor()?.moves))
          if (together && bossing()) {
            entries.push(['Move to spectators', () => say(`!spec ${name()}`)])
            entries.push(['Kick from the room', () => say(`!kick ${name()}`)])
          }

          const room = theirRoom()
          if (room) {
            entries.push([
              'Go to their room',
              async () => {
                // Passworded rooms are the host's business; the list is where
                // you get asked for one.
                if (room.passworded) {
                  pushNotice('info', `${room.title} needs a password`)
                  location.hash = '#/battles'
                  return
                }
                await api.joinBattle(room.id, null)
                location.hash = '#/room'
              },
            ])
          }
          if (isMe()) return entries
          entries.push(
            isFriend()
              ? ['Remove friend', () => api.friendAction('remove', name())]
              : ['Add friend', () => api.friendAction('request', name())],
          )
          entries.push(
            isIgnored()
              ? ['Stop ignoring', () => api.friendAction('unignore', name())]
              : ['Ignore', () => api.friendAction('ignore', name())],
          )
          return entries
        }

        return (
          <div
            ref={root}
            class='player-menu'
            style={{ left: `${target().x}px`, top: `${target().y}px` }}
          >
            <div class='player-menu-name'>{name()}</div>
            <Show when={bot()}>
              {(ai) => (
                <div class='player-menu-about muted'>
                  {ai().bot.ai} · {ai().bot.owner}
                </div>
              )}
            </Show>
            <Show when={user()}>
              {(who) => (
                <div class='player-menu-about'>
                  <Flag country={who().country} />
                  <RankIcon status={who().status} />
                  <Show when={who().status.inGame}>
                    <span class='chip warn'>in game</span>
                  </Show>
                  <Show when={who().status.away}>
                    <span class='chip'>away</span>
                  </Show>
                </div>
              )}
            </Show>
            <Show when={theirRoom()}>
              {(room) => (
                <div class='player-menu-about muted' title={room().title}>
                  in {room().title}
                </div>
              )}
            </Show>
            <Show
              when={!picking()}
              fallback={
                <BonusPanel
                  now={target().moves?.bonusNow ?? 0}
                  apply={(percent) => {
                    // Taken before `act` closes the menu: once it has, the
                    // target is gone and there is nothing to read it off.
                    const bonus = target().moves?.bonus
                    void act('bonus', async () => {
                      await bonus?.(percent)
                    })
                  }}
                  cancel={close}
                />
              }
            >
              <For each={items()}>
                {([label, run, stay]) => (
                  <button
                    onClick={() =>
                      stay
                        ? void run()
                        : void act(
                            label.toLowerCase(),
                            async () => void (await run()),
                          )
                    }
                  >
                    {label}
                  </button>
                )}
              </For>
            </Show>
          </div>
        )
      }}
    </Show>
  )
}

/**
 * A resource bonus, to the percent, in the menu's place: 100% is double the
 * normal income, and SPADS takes nothing higher.
 */
function BonusPanel(props: {
  now: number
  apply: (percent: number) => void
  cancel: () => void
}) {
  const [value, setValue] = createSignal(props.now)
  const take = (text: string) => {
    const n = Number(text)
    if (Number.isFinite(n)) setValue(Math.max(0, Math.min(100, Math.round(n))))
  }
  return (
    <form
      class='bonus-pick'
      onSubmit={(event) => {
        event.preventDefault()
        props.apply(value())
      }}
    >
      <label>
        Bonus
        <input
          type='range'
          min={0}
          max={100}
          step={1}
          value={value()}
          onInput={(e) => take(e.currentTarget.value)}
        />
        <input
          type='number'
          min={0}
          max={100}
          value={value()}
          onInput={(e) => take(e.currentTarget.value)}
        />
        %
      </label>
      <div class='sheet-actions'>
        <button type='button' onClick={props.cancel}>
          Cancel
        </button>
        <button type='submit' class='primary'>
          Set
        </button>
      </div>
    </form>
  )
}
