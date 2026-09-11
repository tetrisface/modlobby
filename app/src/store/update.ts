import { listen } from '@tauri-apps/api/event'
import { createSignal } from 'solid-js'
import type { UpdateProgress } from '../ipc/bindings/UpdateProgress'
import { api, describeError } from '../ipc/client'
import { pushNotice } from './chat'

/**
 * Where the update stands, for the corner of the nav and the settings page,
 * which show the same thing and must not disagree.
 *
 * Rust emits every step on `app-update`; the commands answer with the final
 * one as well, which is what a click waits on.
 */
export const [update, setUpdate] = createSignal<UpdateProgress | null>(null)

/** Follows Rust's steps. Returns what stops following. */
export function watchUpdates(): () => void {
  const pending = listen<UpdateProgress>('app-update', (event) =>
    setUpdate(event.payload),
  )
  return () => void pending.then((unlisten) => unlisten())
}

/** The version a look found and nothing has fetched yet. */
export function available(): string | null {
  const at = update()
  return at?.phase === 'available' ? at.version : null
}

/** The version downloaded and waiting for a restart. */
export function waiting(): string | null {
  const at = update()
  return at?.phase === 'ready' ? at.version : null
}

/** What restarting into the waiting version now would lose, while something. */
export function heldBy(): string | null {
  const at = update()
  return at?.phase === 'ready' ? at.heldBy : null
}

/** Percent downloaded, while downloading; `null` otherwise or when unknown. */
export function downloading(): number | null {
  const at = update()
  if (at?.phase !== 'downloading') return null
  return at.total > 0 ? Math.floor((at.got / at.total) * 100) : 0
}

/** A look or a download is out. */
export function busy(): boolean {
  const phase = update()?.phase
  return phase === 'checking' || phase === 'downloading'
}

export function failure(): string | null {
  const at = update()
  return at?.phase === 'failed' ? at.reason : null
}

/** Looks for a newer release; downloads nothing. */
export async function checkUpdate(): Promise<void> {
  try {
    setUpdate(await api.checkUpdate())
  } catch (error) {
    pushNotice('error', describeError(error))
  }
}

/**
 * Downloads and installs what the look found. Does not come back on a
 * successful install; comes back `ready` when a room or a game made
 * restarting now the wrong thing to do — said out loud, because a click
 * that changes nothing on screen looks like a click that did nothing.
 */
export async function installUpdate(): Promise<void> {
  try {
    const outcome = await api.installUpdate()
    setUpdate(outcome)
    if (outcome.phase === 'ready' && outcome.heldBy !== null) {
      pushNotice(
        'info',
        `version ${outcome.version} is downloaded and installs on the next start; restarting now would lose ${outcome.heldBy}`,
      )
    }
  } catch (error) {
    pushNotice('error', describeError(error))
  }
}

/**
 * Installs the download an earlier run kept, if there is one. Does not come
 * back when it installs; otherwise the start goes on — the manifest moved
 * on, or could not be reached, and either is the corner's to show.
 */
export async function resumeUpdate(): Promise<void> {
  try {
    const outcome = await api.resumeUpdate()
    if (outcome !== null) setUpdate(outcome)
  } catch (error) {
    pushNotice('warning', describeError(error))
  }
}
