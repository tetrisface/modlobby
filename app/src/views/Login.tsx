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

export function Login() {
  const navigate = useNavigate()
  const [username, setUsername] = createSignal('')
  const [password, setPassword] = createSignal('')
  const [remember, setRemember] = createSignal(false)
  const [hasStored, setHasStored] = createSignal(false)
  const [error, setError] = createSignal<string | null>(null)
  const [busy, setBusy] = createSignal(false)

  createEffect(() => {
    const s = settings()
    if (!s || username()) return
    setUsername(s.account.username)
    setRemember(s.account.rememberPassword)
    if (s.account.rememberPassword && s.account.username) {
      api
        .hasPassword(s.account.username)
        .then(setHasStored)
        .catch(() => setHasStored(false))
    }
  })

  createEffect(() => {
    if (lobby.phase === 'ready') navigate('/battles', { replace: true })
  })

  async function submit(event: Event) {
    event.preventDefault()
    setError(null)
    setBusy(true)
    try {
      await api.login(username().trim(), password() || null, remember())
    } catch (err) {
      setError(describeError(err))
    } finally {
      setBusy(false)
    }
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
          placeholder={hasStored() ? 'remembered' : ''}
          autocomplete='current-password'
        />
      </label>
      <label class='row'>
        <input
          type='checkbox'
          checked={remember()}
          onChange={(e) => setRemember(e.currentTarget.checked)}
        />
        Remember the password (OS keyring)
      </label>
      <button type='submit' disabled={busy() || !username().trim()}>
        {busy() ? (phaseText[lobby.phase ?? ''] ?? 'working…') : 'Log in'}
      </button>
      <Show when={error()}>
        {(message) => <p class='error'>{message()}</p>}
      </Show>
      <p class='muted'>
        Server: {settings()?.server.host}:{settings()?.server.port}
        {settings()?.server.tls ? ' (TLS)' : ' (plain)'}
      </p>
    </form>
  )
}
