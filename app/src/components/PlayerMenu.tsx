import { For, Show, createSignal, onCleanup } from 'solid-js'
import { api, describeError } from '../ipc/client'
import { ensureRoom, privateRoom, pushNotice } from '../store/chat'
import { lobby } from '../store/lobby'

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
