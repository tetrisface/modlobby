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
  /**
   * Logging in, or creating an account.
   *
   * Same form either way: an account needs a username and a password, and
   * registering needs an email as well. Making it a second view would mean
   * typing the same two things twice.
   */
  const [mode, setMode] = createSignal<'login' | 'register'>('login')
  const [email, setEmail] = createSignal('')
  /** Set once the server has taken the registration and emailed a code. */
  const [awaitingCode, setAwaitingCode] = createSignal(false)
  const [code, setCode] = createSignal('')
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

  async function register() {
    setBusy(true)
    setError(null)
    try {
      await api.register(username().trim(), password(), email().trim())
      // The account exists but cannot log in yet; the server has emailed a
      // code, and the first login is what asks for it.
      setAwaitingCode(true)
    } catch (err) {
      setError(describeError(err))
    } finally {
      setBusy(false)
    }
  }

  async function confirm() {
    setBusy(true)
    setError(null)
    try {
      // The code is confirmed on a live connection, so a login is started and
      // the code answers the agreement the server replies with.
      await api.confirmAgreement(code().trim())
      setAwaitingCode(false)
      setMode('login')
      await login()
    } catch (err) {
      setError(describeError(err))
    } finally {
      setBusy(false)
    }
  }

  function submit(event: Event) {
    event.preventDefault()
    if (awaitingCode()) void confirm()
    else if (mode() === 'register') void register()
    else void login()
  }

  /**
   * What the one button says, which is five different things.
   *
   * In precedence order: the server's throttle outranks everything because
   * pressing the button would do nothing, then whatever is already in flight,
   * then which of the three jobs the form is currently for.
   */
  function submitLabel(): string {
    if (wait() > 0) return `throttled — ${wait()}s`
    if (busy()) return phaseText[lobby.phase ?? ''] ?? 'working…'
    if (awaitingCode()) return 'Confirm and log in'
    return mode() === 'register' ? 'Create account' : 'Log in'
  }

  return (
    <form class='login' onSubmit={submit}>
      <h1>{mode() === 'register' ? 'Create an account' : 'Log in'}</h1>
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
      <Show when={mode() === 'register'}>
        <label>
          Email
          <input
            type='email'
            value={email()}
            onInput={(e) => setEmail(e.currentTarget.value)}
            autocomplete='email'
          />
        </label>
        <p class='muted'>
          Used to send the code that activates the account, and to recover it.
        </p>
      </Show>
      <Show when={awaitingCode()}>
        <label>
          Code from the email
          <input
            value={code()}
            onInput={(e) => setCode(e.currentTarget.value)}
            autocomplete='one-time-code'
          />
        </label>
      </Show>
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
        {submitLabel()}
      </button>
      <Show when={error()}>
        {(message) => <p class='error'>{message()}</p>}
      </Show>
      <Show when={!awaitingCode()}>
        <button
          type='button'
          class='link'
          onClick={() => {
            setMode(mode() === 'login' ? 'register' : 'login')
            setError(null)
          }}
        >
          {mode() === 'login'
            ? 'No account yet? Create one'
            : 'Already have an account? Log in'}
        </button>
      </Show>
      <p class='muted'>
        Server: {settings()?.server.host}:{settings()?.server.port}
      </p>
    </form>
  )
}
