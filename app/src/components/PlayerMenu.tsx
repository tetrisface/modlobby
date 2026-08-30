import { For, Show, createSignal, onCleanup } from 'solid-js'
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

const [openFor, setOpenFor] = createSignal<{
  name: string
  x: number
  y: number
} | null>(null)

/** Opens the menu for a name at the pointer. */
export function showPlayerMenu(name: string, event: MouseEvent): void {
  event.preventDefault()
  event.stopPropagation()
  setOpenFor({ name, x: event.clientX, y: event.clientY })
}

export function PlayerMenu() {
  const close = () => setOpenFor(null)

  // Any click elsewhere, or Escape, dismisses it — the usual bargain for
  // something that floats above everything.
  const onDown = () => close()
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
        const name = () => target().name
        const isFriend = () => lobby.friends.friends.includes(name())
        const isIgnored = () => lobby.friends.ignored.includes(name())
        const isMe = () => name() === lobby.me

        const user = () => lobby.users[name()]
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

        const items = () => {
          const entries: Array<[string, () => Promise<void> | void]> = [
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
            class='player-menu'
            style={{ left: `${target().x}px`, top: `${target().y}px` }}
            onMouseDown={(event) => event.stopPropagation()}
          >
            <div class='player-menu-name'>{name()}</div>
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
            <For each={items()}>
              {([label, run]) => (
                <button
                  onClick={() =>
                    void act(
                      label.toLowerCase(),
                      async () => void (await run()),
                    )
                  }
                >
                  {label}
                </button>
              )}
            </For>
          </div>
        )
      }}
    </Show>
  )
}
