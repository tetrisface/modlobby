import { Show, createEffect, createSignal } from 'solid-js'
import { createStore, unwrap } from 'solid-js/store'
import type { Settings } from '../ipc/bindings/Settings'
import { api, describeError } from '../ipc/client'
import { pushNotice } from '../store/chat'
import { applySettings, settings } from '../store/settings'

export function SettingsView() {
  const [draft, setDraft] = createStore<Settings>(
    structuredClone(unwrap(settings())) ?? blank(),
  )
  const [saving, setSaving] = createSignal(false)

  createEffect(() => {
    const current = settings()
    if (current) setDraft(structuredClone(unwrap(current)))
  })

  async function save(event: Event) {
    event.preventDefault()
    setSaving(true)
    try {
      applySettings(await api.updateSettings(structuredClone(unwrap(draft))))
      pushNotice('info', 'settings saved')
    } catch (error) {
      pushNotice('error', describeError(error))
    } finally {
      setSaving(false)
    }
  }

  return (
    <form class='settings' onSubmit={save}>
      <h1>Settings</h1>
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
            onInput={(e) =>
              setDraft('server', 'port', Number(e.currentTarget.value))
            }
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
        <legend>Chat</legend>
        <label>
          Lines kept
          <input
            type='number'
            value={draft.chat.maxLines}
            onInput={(e) =>
              setDraft('chat', 'maxLines', Number(e.currentTarget.value))
            }
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

      <button type='submit' class='primary' disabled={saving()}>
        Save
      </button>
    </form>
  )
}

function blank(): Settings {
  return {
    $schema: null,
    server: { host: '', port: 8201, tls: true },
    account: { username: '', rememberPassword: false, autoLogin: false },
    paths: { dataDir: null },
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
    chat: { maxLines: 500 },
    tweaks: { styluaConfig: null, defaultSlot: 'tweakdefs1' },
    logging: { filter: 'info' },
  }
}
