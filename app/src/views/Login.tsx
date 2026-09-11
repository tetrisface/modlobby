import { useNavigate } from '@solidjs/router'
import { For, Show, createEffect, createSignal, onCleanup } from 'solid-js'
import { Linkify, openExternal } from '../components/Linkify'
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

/** What the server's own agreement asks a new account to have read. */
const PRIVACY = 'https://www.beyondallreason.info/privacy'
const CONDUCT = 'https://www.beyondallreason.info/code-of-conduct'

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
  /** Set once the account exists and the server has emailed its code. */
  const [awaitingCode, setAwaitingCode] = createSignal(false)
  const [code, setCode] = createSignal('')
  /** The agreement the server answered the new account's first login with. */
  const [agreement, setAgreement] = createSignal<string[]>([])
  const [username, setUsername] = createSignal('')
  const [password, setPassword] = createSignal('')
  /** Whether the password is being read back, rather than a confirm field. */
  const [reveal, setReveal] = createSignal(false)
  const [remember, setRemember] = createSignal(false)
  const [autoLogin, setAutoLogin] = createSignal(false)
  const [hasStored, setHasStored] = createSignal(false)
  /** Why this username cannot be had, answered without asking the server. */
  const [nameProblem, setNameProblem] = createSignal<string | null>(null)
  const [error, setError] = createSignal<string | null>(null)
  const [busy, setBusy] = createSignal(false)
  /** Seconds teiserver's login limit still needs; 0 when clear. */
  const [wait, setWait] = createSignal(0)

  // The server refuses a login within twenty seconds of the account's last,
  // and the clock is kept across restarts — a rebuild loop reaches it easily,
  // so rather than failing the login we count down and go when it lapses.
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
        .then(setHasStored)
        .catch(() => setHasStored(false))
    }
  })

  // Logging in anywhere lands here as a phase change, including the login that
  // an emailed code finishes, and this is what carries the form off screen.
  createEffect(() => {
    if (lobby.phase === 'ready') navigate('/battles', { replace: true })
  })

  /**
   * The name rules, checked when the field is left rather than per keystroke:
   * they are teiserver's own and mechanical, so the answer costs nothing, but
   * telling somebody their name is wrong while they are still typing it is
   * the kind of form that makes people give up.
   */
  async function checkName() {
    if (mode() !== 'register' || !username().trim()) {
      setNameProblem(null)
      return
    }
    setNameProblem(await api.nameProblem(username().trim()).catch(() => null))
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
      // The account is made and logged in on in one go; what comes back is the
      // agreement the server answers that login with, because a new account is
      // unverified until the emailed code says otherwise.
      setAgreement(await api.register(username().trim(), password(), email()))
      setAwaitingCode(true)
    } catch (err) {
      setError(describeError(err))
    } finally {
      setBusy(false)
    }
  }

  /**
   * A wrong code is a `DENIED`, and teiserver hangs up on those — so a second
   * attempt needs the connection back first. Logging in again puts the server
   * exactly where it was: an unverified account is answered with the agreement
   * rather than a session, which arrives here as a refusal and is the expected
   * outcome, not a failure worth showing.
   */
  async function reopen() {
    if (lobby.phase !== null) return
    await api
      .login(username().trim(), password(), remember(), autoLogin())
      .catch(() => {})
  }

  async function confirm() {
    setBusy(true)
    setError(null)
    try {
      await reopen()
      // This is the login finishing, not a step before one: teiserver verifies
      // the account and accepts it on the same connection.
      await api.confirmAgreement(
        username().trim(),
        password(),
        code(),
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

  function heading(): string {
    if (awaitingCode()) return 'Check your email'
    return mode() === 'register' ? 'Create an account' : 'Log in'
  }

  return (
    <form class='login' onSubmit={submit}>
      <h1>{heading()}</h1>
      <label>
        Username
        <input
          value={username()}
          onInput={(e) => {
            setUsername(e.currentTarget.value)
            setNameProblem(null)
          }}
          onBlur={() => void checkName()}
          disabled={awaitingCode()}
          autocomplete='username'
        />
      </label>
      <Show when={nameProblem()}>
        {(problem) => <p class='error'>{problem()}</p>}
      </Show>
      <label>
        Password
        <input
          // Read back rather than typed twice: the login that follows sends
          // whatever was typed and succeeds either way, so a confirm field
          // would not catch the typo it exists to catch — it would only be
          // found on the next launch, out of the keyring.
          type={reveal() ? 'text' : 'password'}
          value={password()}
          onInput={(e) => setPassword(e.currentTarget.value)}
          placeholder={hasStored() && mode() === 'login' ? MASKED : ''}
          disabled={awaitingCode()}
          autocomplete={
            mode() === 'register' ? 'new-password' : 'current-password'
          }
        />
      </label>
      <button
        type='button'
        class='link login-reveal'
        onClick={() => setReveal(!reveal())}
      >
        {reveal() ? 'Hide password' : 'Show password'}
      </button>
      <Show when={mode() === 'register'}>
        <label>
          Email
          <input
            type='email'
            value={email()}
            onInput={(e) => setEmail(e.currentTarget.value)}
            disabled={awaitingCode()}
            autocomplete='email'
          />
        </label>
        <p class='muted'>
          Used to send the code that activates the account, and to recover it.
        </p>
      </Show>
      <Show when={awaitingCode()}>
        {/* The server's own words, and the thing the code agrees to. */}
        <For each={agreement().filter((line) => line.trim() !== '')}>
          {(line) => (
            <p class='muted'>
              <Linkify text={line} />
            </p>
          )}
        </For>
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
      <Show when={mode() === 'register' && !awaitingCode()}>
        <p class='muted'>
          Creating an account accepts BAR's{' '}
          <a
            href={PRIVACY}
            onClick={(event) => {
              event.preventDefault()
              void openExternal(PRIVACY)
            }}
          >
            privacy policy
          </a>{' '}
          and{' '}
          <a
            href={CONDUCT}
            onClick={(event) => {
              event.preventDefault()
              void openExternal(CONDUCT)
            }}
          >
            code of conduct
          </a>
          .
        </p>
      </Show>
      {/* Above the button, because it is the reason the last press did
          nothing and reading it after pressing again is too late. */}
      <Show when={error()}>
        {(message) => <p class='error'>{message()}</p>}
      </Show>
      <button
        type='submit'
        disabled={
          busy() ||
          wait() > 0 ||
          !username().trim() ||
          (mode() === 'register' && nameProblem() !== null)
        }
      >
        {submitLabel()}
      </button>
      {/* Directly under the button: a change of mind about which form this
          is, next to the thing that submits it. */}
      <Show when={!awaitingCode()}>
        <p class='muted login-switch'>
          {mode() === 'login'
            ? "Don't have an account?"
            : 'Already have an account?'}{' '}
          <button
            type='button'
            class='link'
            onClick={() => {
              setMode(mode() === 'login' ? 'register' : 'login')
              setNameProblem(null)
              setError(null)
            }}
          >
            {mode() === 'login' ? 'Register' : 'Log in'}
          </button>
        </p>
      </Show>
      <p class='muted'>
        Server: {settings()?.server.host}:{settings()?.server.port}
      </p>
    </form>
  )
}
