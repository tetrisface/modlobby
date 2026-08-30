import { For, Show, createEffect, createSignal, onCleanup } from 'solid-js'
import { createStore, unwrap } from 'solid-js/store'
import type { Settings } from '../ipc/bindings/Settings'
import { api, describeError } from '../ipc/client'
import { pushNotice } from '../store/chat'
import { applySettings, settings } from '../store/settings'

/** Long enough that typing a hostname is one write rather than twelve. */
const SAVE_AFTER = 600

/**
 * The draft as a string, with object keys in a fixed order.
 *
 * Used only to answer "has anything actually changed": the settings that come
 * back from Rust and the draft cloned from them agree on key order, but
 * [`blank`] need not, and a spurious first save would write the placeholder
 * over the file.
 */
function fingerprint(value: unknown): string {
  return JSON.stringify(value, (_key, held: unknown) =>
    held && typeof held === 'object' && !Array.isArray(held)
      ? Object.fromEntries(
          Object.entries(held).sort(([a], [b]) => a.localeCompare(b)),
        )
      : held,
  )
}

/**
 * A number field's value, or `null` while it is not one yet.
 *
 * `Number('')` is `0`, and with a save button that only ever showed as a
 * momentary zero on screen. Saving on its own, it would be written to the file
 * the instant you cleared the field to type a new port.
 */
function counted(text: string): number | null {
  const value = Number(text)
  return text.trim() === '' || !Number.isFinite(value) || value <= 0
    ? null
    : value
}

export function SettingsView() {
  const [draft, setDraft] = createStore<Settings>(
    structuredClone(unwrap(settings())) ?? blank(),
  )
  const [state, setState] = createSignal<'clean' | 'saving' | 'saved'>('clean')

  /** What is in the file, as far as we know. */
  let saved = fingerprint(unwrap(settings()) ?? blank())
  let pending: ReturnType<typeof setTimeout> | undefined

  createEffect(() => {
    const current = settings()
    if (!current) return
    // An edit made in the file, or our own write coming back. Either way this
    // is now what the file says, so it is neither a change to save nor one to
    // draw attention to.
    saved = fingerprint(current)
    setDraft(structuredClone(unwrap(current)))
  })

  // Reading the whole draft is what subscribes this to every field in it.
  createEffect(() => {
    const now = fingerprint(draft)
    if (now === saved) return
    clearTimeout(pending)
    pending = setTimeout(() => void save(), SAVE_AFTER)
  })

  onCleanup(() => clearTimeout(pending))

  async function save() {
    setState('saving')
    try {
      const written = await api.updateSettings(structuredClone(unwrap(draft)))
      saved = fingerprint(written)
      applySettings(written)
      setState('saved')
    } catch (error) {
      // The draft keeps the rejected value so it can be corrected rather than
      // silently reverted; the file still holds the last good one.
      setState('clean')
      pushNotice('error', describeError(error))
    }
  }

  return (
    <form class='settings' onSubmit={(event) => event.preventDefault()}>
      <h1>
        Settings
        <Show when={state() !== 'clean'}>
          <span class='saved-mark'>
            {state() === 'saving' ? 'saving…' : 'saved'}
          </span>
        </Show>
      </h1>
      <p class='muted'>
        Stored as JSONC you can edit by hand; the app reloads it live and keeps
        your comments.{' '}
        <button type='button' onClick={() => api.openSettingsFile()}>
          Open settings file
        </button>
      </p>

      <fieldset>
        <legend>Server</legend>
        <label>
          Host
          <input
            value={draft.server.host}
            onInput={(e) => setDraft('server', 'host', e.currentTarget.value)}
          />
        </label>
        <label>
          Port
          <input
            type='number'
            value={draft.server.port}
            onInput={(e) => {
              const port = counted(e.currentTarget.value)
              if (port !== null) setDraft('server', 'port', port)
            }}
          />
        </label>
      </fieldset>

      <fieldset>
        <legend>Account</legend>
        <label class='row'>
          <input
            type='checkbox'
            checked={draft.account.rememberPassword}
            onChange={(e) => {
              setDraft('account', 'rememberPassword', e.currentTarget.checked)
              if (!e.currentTarget.checked)
                setDraft('account', 'autoLogin', false)
            }}
          />
          Remember the password (OS keyring)
        </label>
        <label class='row'>
          <input
            type='checkbox'
            checked={draft.account.autoLogin}
            disabled={!draft.account.rememberPassword}
            onChange={(e) =>
              setDraft('account', 'autoLogin', e.currentTarget.checked)
            }
          />
          Log in automatically on startup
        </label>
        <Show when={draft.account.username}>
          <button
            type='button'
            onClick={() =>
              void api.clearPassword(draft.account.username).then(() => {
                setDraft('account', 'rememberPassword', false)
                setDraft('account', 'autoLogin', false)
                pushNotice('info', 'forgot the stored password')
              })
            }
          >
            Forget the stored password
          </button>
        </Show>
      </fieldset>

      <fieldset>
        <legend>Playing</legend>
        <label class='row'>
          <input
            type='checkbox'
            checked={draft.play.inPublicRooms}
            onChange={(e) =>
              setDraft('play', 'inPublicRooms', e.currentTarget.checked)
            }
          />
          Let me take a seat in public rooms
        </label>
        <p class='muted'>
          Off by default. A slot in a public room belongs to someone waiting for
          a game; a room of your own never needs this.
        </p>
      </fieldset>

      <fieldset>
        <legend>Notifications</legend>
        <p class='muted'>Raised only while the window is in the background.</p>
        <label class='row'>
          <input
            type='checkbox'
            checked={draft.notifications.enabled}
            onChange={(e) =>
              setDraft('notifications', 'enabled', e.currentTarget.checked)
            }
          />
          Notify me
        </label>
        <For
          each={
            [
              ['privateMessage', 'A direct message'],
              ['mention', 'Someone says my name'],
              ['friendOnline', 'A friend comes online'],
              ['ring', 'Someone rings me'],
              ['vote', 'A vote opens in my room'],
              ['gameStarting', "My room's game starts"],
            ] as const
          }
        >
          {([key, label]) => (
            <label class='row'>
              <input
                type='checkbox'
                disabled={!draft.notifications.enabled}
                checked={draft.notifications[key]}
                onChange={(e) =>
                  setDraft('notifications', key, e.currentTarget.checked)
                }
              />
              {label}
            </label>
          )}
        </For>
      </fieldset>

      <fieldset>
        <legend>Chat</legend>
        <label>
          Lines kept
          <input
            type='number'
            value={draft.chat.maxLines}
            onInput={(e) => {
              const lines = counted(e.currentTarget.value)
              if (lines !== null) setDraft('chat', 'maxLines', lines)
            }}
          />
        </label>
      </fieldset>

      <fieldset>
        <legend>Paths</legend>
        <label>
          BAR data directory (blank = the launcher's)
          <input
            value={draft.paths.dataDir ?? ''}
            onInput={(e) =>
              setDraft('paths', 'dataDir', e.currentTarget.value || null)
            }
          />
        </label>
        <button type='button' onClick={() => api.openDataDir()}>
          Open data directory
        </button>
      </fieldset>

      <fieldset>
        <legend>Logging</legend>
        <label>
          Filter (a `tracing` filter, e.g. `info,spring::rx=trace`)
          <input
            value={draft.logging.filter}
            onInput={(e) =>
              setDraft('logging', 'filter', e.currentTarget.value)
            }
          />
        </label>
        <p class='muted'>
          Both the Rust side and the webview console write one JSON-per-line
          file per day, kept across restarts. Applies on the next start.
        </p>
        <button type='button' onClick={() => api.openLogDir()}>
          Open log folder
        </button>
      </fieldset>
    </form>
  )
}

function blank(): Settings {
  return {
    $schema: null,
    server: { host: '', port: 8201, tls: true },
    account: { username: '', rememberPassword: false, autoLogin: false },
    paths: { dataDir: null },
    play: { inPublicRooms: false },
    notifications: {
      enabled: true,
      privateMessage: true,
      mention: true,
      friendOnline: true,
      vote: true,
      gameStarting: true,
      ring: true,
    },
    battleList: {
      showPassworded: true,
      showLocked: true,
      showEmpty: true,
      showRunning: true,
      friendsOnly: false,
      mode: 'all',
      sort: 'relevance',
      sortDescending: false,
    },
    chat: { maxLines: 500, channels: ['main'] },
    tweaks: { styluaConfig: null, defaultSlot: 'tweakdefs1' },
    logging: { filter: 'info' },
  }
}
