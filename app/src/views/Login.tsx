import { useNavigate } from '@solidjs/router'
import { Show, createEffect, createSignal } from 'solid-js'
import { api, describeError } from '../ipc/client'
import { lobby } from '../store/lobby'
import { settings } from '../store/settings'

const phaseText: Record<string, string> = {
  connecting: 'connecting…',
  awaitingLogin: 'logging in…',
  loading: 'loading the lobby…',
}

/** What a stored password looks like: present, and not readable. */
const MASKED = '••••••••'

/**
 * Auto-login is attempted once per run, not once per mount — otherwise
 * returning to this view after logging out would immediately log back in.
 */
let autoLoginAttempted = false

export function Login() {
  const navigate = useNavigate()
  const [username, setUsername] = createSignal('')
  const [password, setPassword] = createSignal('')
  const [remember, setRemember] = createSignal(false)
  const [autoLogin, setAutoLogin] = createSignal(false)
  const [hasStored, setHasStored] = createSignal(false)
  const [error, setError] = createSignal<string | null>(null)
  const [busy, setBusy] = createSignal(false)

  createEffect(() => {
    const s = settings()
    if (!s || username()) return
    setUsername(s.account.username)
    setRemember(s.account.rememberPassword)
    setAutoLogin(s.account.autoLogin)
    if (s.account.rememberPassword && s.account.username) {
      void api
        .hasPassword(s.account.username)
        .then(async (stored) => {
          setHasStored(stored)
          if (stored && s.account.autoLogin) await attemptAutoLogin()
        })
        .catch(() => setHasStored(false))
    }
  })

  createEffect(() => {
    if (lobby.phase === 'ready') navigate('/battles', { replace: true })
  })

  async function attemptAutoLogin() {
    if (autoLoginAttempted || lobby.phase !== null) return
    autoLoginAttempted = true
    await login()
  }

  /** Sends the typed password, or falls back to the remembered one. */
  async function login() {
    setError(null)
    setBusy(true)
    try {
      await api.login(
        username().trim(),
        password() || null,
        remember(),
        autoLogin(),
      )
    } catch (err) {
      setError(describeError(err))
    } finally {
      setBusy(false)
    }
  }

  function submit(event: Event) {
    event.preventDefault()
    void login()
  }

  return (
    <form class='login' onSubmit={submit}>
      <h1>Log in</h1>
      <label>
        Username
        <input
          value={username()}
          onInput={(e) => setUsername(e.currentTarget.value)}
          autocomplete='username'
        />
      </label>
      <label>
        Password
        <input
          type='password'
          value={password()}
          onInput={(e) => setPassword(e.currentTarget.value)}
          placeholder={hasStored() ? MASKED : ''}
          autocomplete='current-password'
        />
      </label>
      <label class='row'>
        <input
          type='checkbox'
          checked={remember()}
          onChange={(e) => {
            setRemember(e.currentTarget.checked)
            if (!e.currentTarget.checked) setAutoLogin(false)
          }}
        />
        Remember the password (OS keyring)
      </label>
      <label class='row'>
        <input
          type='checkbox'
          checked={autoLogin()}
          disabled={!remember()}
          onChange={(e) => setAutoLogin(e.currentTarget.checked)}
        />
        Log in automatically on startup
      </label>
      <button type='submit' disabled={busy() || !username().trim()}>
        {busy() ? (phaseText[lobby.phase ?? ''] ?? 'working…') : 'Log in'}
      </button>
      <Show when={error()}>
        {(message) => <p class='error'>{message()}</p>}
      </Show>
      <p class='muted'>
        Server: {settings()?.server.host}:{settings()?.server.port}
      </p>
    </form>
  )
}
