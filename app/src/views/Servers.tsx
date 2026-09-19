import { For, Show, createSignal } from 'solid-js'
import type { SetStoreFunction } from 'solid-js/store'
import { LoginSheet } from '../components/LoginForm'
import { openExternal } from '../components/Linkify'
import { Row } from '../components/SettingRow'
import type { ServerEntry } from '../ipc/bindings/ServerEntry'
import type { Settings } from '../ipc/bindings/Settings'
import { api, describeError } from '../ipc/client'
import {
  forgotPasswordUrl,
  hostProblem,
  newServer,
  parsePorts,
  serverId,
  serverName,
  sessionStatus,
} from '../lib/servers'
import { pushNotice } from '../store/chat'
import { lobby } from '../store/lobby'

/** What the login sheet is open on, if anything. */
type Asking = { server: string; mode: 'login' | 'register' }

/**
 * The servers this app talks to, a card each, and a way to add another.
 *
 * Edits go into the page's draft like every other setting, and are saved the
 * same way; what needs the server — logging in, registering — opens the login
 * form over the page.
 */
export function ServerRows(props: {
  draft: Settings
  setDraft: SetStoreFunction<Settings>
  /** Saves the draft now; the login form acts on what is saved. */
  settle: () => Promise<void>
}) {
  const [asking, setAsking] = createSignal<Asking | null>(null)

  async function ask(entry: ServerEntry, mode: Asking['mode']) {
    await props.settle()
    setAsking({ server: serverId(entry.host), mode })
  }

  return (
    <>
      <For each={props.draft.servers}>
        {(entry, index) => (
          <Row>
            <ServerCard
              entry={entry}
              change={(field, value) =>
                props.setDraft('servers', index(), field, value)
              }
              remove={() =>
                props.setDraft('servers', (servers) =>
                  servers.filter((_, at) => at !== index()),
                )
              }
              ask={(mode) => void ask(entry, mode)}
            />
          </Row>
        )}
      </For>
      <Row>
        <AddServer
          listed={props.draft.servers.map((entry) => entry.host)}
          add={(host) =>
            props.setDraft('servers', (servers) => [
              ...servers,
              newServer(host),
            ])
          }
        />
      </Row>
      <Show when={asking()} keyed>
        {(open) => (
          <LoginSheet
            server={open.server}
            mode={open.mode}
            close={() => setAsking(null)}
          />
        )}
      </Show>
    </>
  )
}

/** How long a Remove waits for its second click before it stands down. */
const CONFIRM_FOR = 4000

function ServerCard(props: {
  entry: ServerEntry
  change: <K extends keyof ServerEntry>(field: K, value: ServerEntry[K]) => void
  remove: () => void
  ask: (mode: 'login' | 'register') => void
}) {
  const id = () => serverId(props.entry.host)
  const session = () => lobby.servers[id()]
  const way = () => lobby.ways[id()]
  const [portsText, setPortsText] = createSignal(props.entry.ports.join(', '))
  const [removing, setRemoving] = createSignal(false)
  /**
   * Read once: open for a server that has needed it, and after that the
   * reader's to open and close — not shut under the hand that unticks a box.
   */
  const startsOpen = props.entry.allowUnencrypted

  const status = () => sessionStatus(session())
  const connected = () => (session()?.phase ?? null) !== null

  async function act(what: string, run: () => Promise<unknown>) {
    try {
      await run()
    } catch (error) {
      pushNotice('warning', `${what}: ${describeError(error)}`)
    }
  }

  function remove() {
    if (!removing()) {
      setRemoving(true)
      setTimeout(() => setRemoving(false), CONFIRM_FOR)
      return
    }
    // A server that is gone from the list is not one to stay logged in to,
    // nor one to go on keeping a password and a way in for.
    void act('remove', async () => {
      if (connected()) await api.logout(id())
      if (props.entry.username)
        await api.clearPassword(id(), props.entry.username)
      await api.forgetWay(props.entry.host)
    }).then(props.remove)
  }

  return (
    <div class='server-card'>
      <div class='server-head'>
        <input
          class='server-name'
          aria-label='Name'
          value={props.entry.name}
          placeholder={props.entry.host}
          onInput={(event) => props.change('name', event.currentTarget.value)}
        />
        <span class={`chip ${status().tone}`}>{status().text}</span>
      </div>
      <p class='muted'>
        {props.entry.host}
        <Show when={props.entry.username}>
          {(name) => <> · account {name()}</>}
        </Show>
        <Show when={way()}>
          {(found) => (
            <>
              {' · '}last way in: <b>{found()}</b>{' '}
              <button
                type='button'
                class='link'
                onClick={() =>
                  void act('re-probe', () => api.forgetWay(props.entry.host))
                }
              >
                try every way again
              </button>
            </>
          )}
        </Show>
      </p>
      {/* Set once, if ever: a server that works is never opened here. */}
      <details class='server-connection' open={startsOpen}>
        <summary>Connection</summary>
        <label>
          Ports
          <input
            value={portsText()}
            aria-invalid={parsePorts(portsText()) === null}
            onInput={(event) => {
              setPortsText(event.currentTarget.value)
              const ports = parsePorts(event.currentTarget.value)
              if (ports) props.change('ports', ports)
            }}
          />
        </label>
        <Show
          when={parsePorts(portsText()) !== null}
          fallback={
            <p class='error'>
              Port numbers, separated by commas — not saved until they are.
            </p>
          }
        >
          <p class='muted'>
            Every port is tried encrypted both ways at once, and the first to
            answer is remembered.
          </p>
        </Show>
        <label class='row'>
          <input
            type='checkbox'
            checked={props.entry.allowUnencrypted}
            onChange={(event) =>
              props.change('allowUnencrypted', event.currentTarget.checked)
            }
          />
          Allow unencrypted, once every encrypted way has failed
        </label>
        <Show when={props.entry.allowUnencrypted}>
          <p class='muted warn-text'>
            Only for a server with nothing else: your password and every message
            cross the network readable. Never raced against encryption.
          </p>
        </Show>
      </details>
      <div class='server-actions'>
        <Show
          when={connected()}
          fallback={
            <>
              <button
                type='button'
                class='primary'
                onClick={() => props.ask('login')}
              >
                Log in
              </button>
              <button type='button' onClick={() => props.ask('register')}>
                Register
              </button>
            </>
          }
        >
          <button
            type='button'
            onClick={() => void act('log out', () => api.logout(id()))}
          >
            Log out
          </button>
        </Show>
        <button
          type='button'
          onClick={() => void openExternal(forgotPasswordUrl(props.entry))}
        >
          Reset password
        </button>
        <Show when={props.entry.username}>
          {(name) => (
            <button
              type='button'
              onClick={() =>
                void act('forget the password', async () => {
                  await api.clearPassword(id(), name())
                  pushNotice('info', `forgot the password for ${name()}`)
                })
              }
            >
              Forget password
            </button>
          )}
        </Show>
        <button
          type='button'
          class='server-remove'
          classList={{ danger: removing() }}
          onClick={remove}
        >
          {removing() ? 'Really remove?' : 'Remove'}
        </button>
      </div>
    </div>
  )
}

function AddServer(props: { listed: string[]; add: (host: string) => void }) {
  const [host, setHost] = createSignal('')
  const problem = () => hostProblem(host(), props.listed)

  function add() {
    if (problem() !== null) return
    props.add(host())
    setHost('')
  }

  return (
    <>
      <div class='server-add'>
        <label>
          Add a server
          <input
            value={host()}
            placeholder='its host, e.g. server.example.com'
            onInput={(event) => setHost(event.currentTarget.value)}
            onKeyDown={(event) => {
              if (event.key !== 'Enter') return
              event.preventDefault()
              add()
            }}
          />
        </label>
        <button type='button' disabled={problem() !== null} onClick={add}>
          Add
        </button>
      </div>
      <Show when={host().trim() && problem()}>
        {(why) => <p class='error'>{why()}</p>}
      </Show>
      <p class='muted'>
        Each server has its own accounts: an account on one is nothing to
        another. Add a server once — the same one under two names would be two
        logins that keep throwing each other out.
      </p>
    </>
  )
}
