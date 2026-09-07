import { listen } from '@tauri-apps/api/event'
import { Show, createEffect, createSignal, onCleanup, onMount } from 'solid-js'
import type { EngineProgress } from '../ipc/bindings/EngineProgress'
import { api, describeError } from '../ipc/client'
import { build } from '../store/build'
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
 *
 * Whether this machine has an engine to fetch at all is one local round trip
 * behind the window, so `auto` waits for that answer rather than asking on
 * mount: a reload straight into a room used to ask before it knew, and on a
 * machine Beyond All Reason publishes nothing for that is a red notice
 * carrying instructions nobody asked for. The button stays live either way — a
 * click is a question somebody meant to ask, and it gets the whole refusal
 * back rather than being quietly ignored.
 */
export function GetEngine(props: {
  version: string
  auto?: boolean
  onDone?: () => void
}) {
  const [progress, setProgress] = createSignal<EngineProgress | null>(null)
  const [busy, setBusy] = createSignal(false)
  /** Once per mount, whatever the answer turns out to be. */
  let asked = false

  onMount(() => {
    const pending = listen<EngineProgress>('engine-download', (event) =>
      setProgress(event.payload),
    )
    onCleanup(() => void pending.then((unlisten) => unlisten()))
  })

  // A version we do not have is a question the index answers 404 to, and
  // firing it on mount is what turned an empty room into a retry loop. So is
  // a machine no build is published for, which the shell answers a moment
  // after the window: waiting for it costs a frame, and asking without it
  // costs an error nobody can act on.
  createEffect(() => {
    const known = build()
    if (!known || asked || !props.auto || !props.version) return
    asked = true
    if (!known.noPublishedEngine) void get()
  })

  async function get() {
    if (!props.version) return
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

  /** Named where the room knows the version, general where it does not. */
  const heading = () =>
    props.version
      ? `Engine ${props.version} is not installed.`
      : 'The engine is not installed.'

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
        <strong>{heading()}</strong>{' '}
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
