import { useNavigate } from '@solidjs/router'
import { For, Show, createSignal } from 'solid-js'
import { LoginForm } from '../components/LoginForm'
import { Select } from '../components/Select'
import { serverId, serverName } from '../lib/servers'
import { mainServer } from '../store/lobby'
import { settings } from '../store/settings'

/**
 * The way in while there is no session: the login form, for whichever server
 * is picked — the one the lobby already means, else the first listed.
 */
export function Login() {
  const navigate = useNavigate()
  const servers = () => settings()?.servers ?? []
  const [picked, setPicked] = createSignal<string | null>(null)
  const server = () =>
    picked() ??
    mainServer() ??
    (servers()[0] ? serverId(servers()[0]!.host) : null)

  return (
    <div class='login-page'>
      <Show when={servers().length > 1}>
        <label>
          Server
          <Select
            value={server() ?? ''}
            onChange={(event) => setPicked(event.currentTarget.value)}
          >
            <For each={servers()}>
              {(entry) => (
                <option value={serverId(entry.host)}>
                  {serverName(entry)}
                </option>
              )}
            </For>
          </Select>
        </label>
      </Show>
      <Show
        when={server()}
        keyed
        fallback={
          <p class='muted'>
            No server is set up. Add one in Settings, under Servers.
          </p>
        }
      >
        {(id) => (
          <LoginForm
            server={id}
            onDone={() => navigate('/battles', { replace: true })}
          />
        )}
      </Show>
    </div>
  )
}
