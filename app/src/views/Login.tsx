import { useNavigate } from '@solidjs/router'
import { Show, createEffect, createSignal, onCleanup } from 'solid-js'
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
  /** Seconds teiserver's login limit still needs; 0 when clear. */
  const [wait, setWait] = createSignal(0)

  // The limit is three logins per ten seconds, counted per account and kept
  // across restarts — a rebuild loop reaches it easily, so rather than failing
  // the login we count down and go when it lapses.
  createEffect(() => {
    void api
      .loginWait()
      .then(setWait)
      .catch(() => setWait(0))
  })
  createEffect(() => {
    if (wait() <= 0) return
    const timer = setTimeout(() => setWait((seconds) => seconds - 1), 1000)
    onCleanup(() => clearTimeout(timer))
  })

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
    const held = await api.loginWait().catch(() => 0)
    if (held > 0) {
      setWait(held)
      // Waiting it out beats failing: the rebuild that caused this is over.
      setTimeout(() => void login(), (held + 1) * 1000)
      return
    }
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
      void api
        .loginWait()
        .then(setWait)
        .catch(() => {})
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
      <button
        type='submit'
        disabled={busy() || wait() > 0 || !username().trim()}
      >
        {wait() > 0
          ? `throttled — ${wait()}s`
          : busy()
            ? (phaseText[lobby.phase ?? ''] ?? 'working…')
            : 'Log in'}
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
