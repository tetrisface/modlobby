import { api, describeError } from '../ipc/client'
import { pushNotice } from './chat'
import { applySettings, settings } from './settings'

/**
 * Which channels to rejoin next time.
 *
 * Driven by what the reader asks for — a `/join`, a `/leave`, a click in the
 * directory — rather than by watching which channels happen to be open. The
 * difference matters at startup: between asking to join and being let in,
 * nothing is open, and a watcher would faithfully save that as "no channels".
 *
 * The command hands back the settings it wrote, which are applied here, so the
 * next call builds on what the file actually says. Our own write raises no
 * change event — the store recognises it by hash — so this is the only way the
 * front end learns of it.
 */
export async function rememberChannel(
  name: string,
  joined: boolean,
): Promise<void> {
  const saved = settings()?.chat.channels ?? []
  const next = joined
    ? saved.includes(name)
      ? saved
      : [...saved, name]
    : saved.filter((channel) => channel !== name)
  if (next.length === saved.length && next.every((c, i) => c === saved[i]))
    return
  applySettings(await api.rememberChannels(next))
}

/** Whether a room is kept out of the Chat tab's unread count. */
export function isMuted(room: string): boolean {
  return settings()?.chat.muted.includes(room) ?? false
}

/** Keeps a room out of the Chat tab's unread count, or lets it back in. */
export async function toggleMute(room: string): Promise<void> {
  const current = settings()
  if (!current) return
  const saved = current.chat.muted
  const next = saved.includes(room)
    ? saved.filter((key) => key !== room)
    : [...saved, room]
  try {
    applySettings(
      await api.updateSettings({
        ...current,
        chat: { ...current.chat, muted: next },
      }),
    )
  } catch (error) {
    pushNotice('warning', `mute: ${describeError(error)}`)
  }
}
