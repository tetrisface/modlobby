import { api } from '../ipc/client'
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
