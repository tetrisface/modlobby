import { Show, createEffect, createSignal, onCleanup } from 'solid-js'
import type { PasteStatus } from '../ipc/bindings/PasteStatus'
import { api, describeError } from '../ipc/client'
import { pushNotice } from '../store/chat'
import { lobby } from '../store/lobby'

/** How long a finished paste stays up before the banner folds away. */
const LINGER_MS = 6000

type Running = Extract<PasteStatus, { state: 'running' }>
type Done = Extract<PasteStatus, { state: 'done' }>

/**
 * A multi-line paste on its way out, above the setup tabs so it pushes them
 * down: paced under SPADS's counters a paste can take a minute, and a pane
 * that sits still for a minute reads as nothing happening.
 *
 * The bar follows the host's answers, weighted by how much each command
 * asked of it, since a tweak blob takes the host far longer than a short
 * setting. There is no time estimate: the host's pace is not ours to know.
 * A finished paste lingers long enough to be read, then goes.
 */
export function PasteBanner() {
  const [shown, setShown] = createSignal(false)

  createEffect(() => {
    const paste = lobby.paste
    if (paste.state === 'idle') return setShown(false)
    setShown(true)
    if (paste.state === 'done') {
      const timer = setTimeout(() => setShown(false), LINGER_MS)
      onCleanup(() => clearTimeout(timer))
    }
  })

  /** Answered bytes over command bytes; a paste of plain chat falls back to what left. */
  const percent = (paste: Running) => {
    if (paste.work > 0) return Math.round((paste.done / paste.work) * 100)
    if (paste.total === 0) return 100
    return Math.round((paste.sent / paste.total) * 100)
  }

  const skipped = (n: number) => (n === 0 ? '' : ` · ${n} already set, skipped`)

  const running = (paste: Running) =>
    `Applying commands ${paste.applied}/${paste.commands}` +
    skipped(paste.skipped)

  const done = (paste: Done) => {
    if (paste.cancelled)
      return `Paste cancelled: the host answered ${paste.applied} of ${paste.commands} commands before the rest was dropped`
    if (paste.total === 0)
      return `Nothing to send: the room already has all ${paste.skipped} settings`
    if (paste.applied >= paste.commands)
      return `Paste applied: ${paste.commands} commands${skipped(paste.skipped)}`
    return (
      `Paste sent: the host answered ${paste.applied} of ${paste.commands} commands` +
      skipped(paste.skipped)
    )
  }

  async function cancel() {
    try {
      await api.cancelPaste()
    } catch (error) {
      pushNotice('warning', `cancel: ${describeError(error)}`)
    }
  }

  return (
    <Show when={shown()}>
      <div
        class='paste-banner'
        classList={{ done: lobby.paste.state === 'done' }}
        role='status'
      >
        <Show when={lobby.paste.state === 'running'}>
          <div class='paste-head'>
            <div class='paste-text'>{running(lobby.paste as Running)}</div>
            <button type='button' class='paste-cancel' onClick={cancel}>
              Cancel
            </button>
          </div>
          <div class='paste-bar'>
            <div style={{ width: `${percent(lobby.paste as Running)}%` }} />
          </div>
        </Show>
        <Show when={lobby.paste.state === 'done'}>
          <div class='paste-text'>{done(lobby.paste as Done)}</div>
        </Show>
      </div>
    </Show>
  )
}
