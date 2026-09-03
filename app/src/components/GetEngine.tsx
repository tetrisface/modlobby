import { listen } from '@tauri-apps/api/event'
import { Show, createSignal, onCleanup, onMount } from 'solid-js'
import type { EngineProgress } from '../ipc/bindings/EngineProgress'
import { api, describeError } from '../ipc/client'
import { pushNotice } from '../store/chat'

/** Bytes as something a person reads, which for this is always whole MB. */
function mb(bytes: number): string {
  return `${Math.round(bytes / 1_000_000)} MB`
}

/**
 * Getting the first engine onto a machine that has none.
 *
 * This is the one download modlobby does itself. Everything else goes through
 * pr-downloader, which is the right tool — but pr-downloader ships inside an
 * engine, so it cannot be what fetches one.
 *
 * Shown only where an engine is required and missing. With `auto` it starts
 * by itself, which is what a room does: joining one whose engine you lack is
 * a request for that engine, as bar-lobby and the launcher both treat it.
 * Without `auto` it says the size and waits for the click.
 */
export function GetEngine(props: {
  version: string
  auto?: boolean
  onDone?: () => void
}) {
  const [progress, setProgress] = createSignal<EngineProgress | null>(null)
  const [busy, setBusy] = createSignal(false)

  onMount(() => {
    const pending = listen<EngineProgress>('engine-download', (event) =>
      setProgress(event.payload),
    )
    onCleanup(() => void pending.then((unlisten) => unlisten()))
    if (props.auto) void get()
  })

  async function get() {
    setBusy(true)
    try {
      await api.downloadEngine(props.version)
      props.onDone?.()
    } catch (error) {
      pushNotice('error', describeError(error))
    } finally {
      setBusy(false)
    }
  }

  const said = () => {
    const at = progress()
    if (!at) return null
    switch (at.phase) {
      case 'finding':
        return 'looking it up…'
      case 'downloading':
        return at.total > 0 ? `${mb(at.got)} of ${mb(at.total)}` : mb(at.got)
      case 'extracting':
        return 'unpacking…'
      case 'done':
        return `engine ${at.version} installed`
      case 'failed':
        return at.reason
    }
  }

  const fraction = () => {
    const at = progress()
    return at?.phase === 'downloading' && at.total > 0
      ? at.got / at.total
      : null
  }

  return (
    <div class='get-engine'>
      <div class='get-engine-say'>
        <strong>Engine {props.version} is not installed.</strong>{' '}
        <span class='muted'>
          It is a few hundred megabytes and only needs fetching once.
        </span>
      </div>
      <Show when={said()}>
        {(text) => (
          <div class='get-engine-progress'>
            <div class='bar'>
              <div
                class='fill'
                classList={{ indeterminate: fraction() === null }}
                style={
                  fraction() === null
                    ? undefined
                    : { width: `${(fraction() ?? 0) * 100}%` }
                }
              />
            </div>
            <span class='muted'>{text()}</span>
          </div>
        )}
      </Show>
      <Show when={!busy()}>
        <button class='primary' onClick={() => void get()}>
          {progress()?.phase === 'failed' ? 'Try again' : 'Download the engine'}
        </button>
      </Show>
    </div>
  )
}
