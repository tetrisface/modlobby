import { useSearchParams } from '@solidjs/router'
import { For, Show, createEffect, createSignal, onCleanup } from 'solid-js'
import { createStore, unwrap } from 'solid-js/store'
import { PlayerFiles } from '../components/PlayerFiles'
import type { Settings } from '../ipc/bindings/Settings'
import { api, describeError } from '../ipc/client'
import { pushNotice } from '../store/chat'
import { bounds } from '../lib/scale'
import {
  applySettings,
  resetScale,
  setScale,
  settings,
  uiScale,
} from '../store/settings'
import { busy as updating, checkUpdate } from '../store/update'

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

/** Like `counted`, for a field where zero is a choice rather than a blank. */
function wholeNumber(text: string): number | null {
  const value = Number(text)
  return text.trim() === '' || !Number.isInteger(value) || value < 0
    ? null
    : value
}

type Tab = 'general' | 'notifications' | 'advanced'

export function SettingsView() {
  const [draft, setDraft] = createStore<Settings>(
    structuredClone(unwrap(settings())) ?? blankSettings(),
  )
  const [state, setState] = createSignal<'clean' | 'saving' | 'saved'>('clean')
  const [params, setParams] = useSearchParams()
  /**
   * Which tab is open, kept in the URL rather than in a signal: the corner
   * notices link straight to the notification settings, and a link has to be
   * able to say which page it means.
   */
  const tab = (): Tab => {
    const wanted = Array.isArray(params.tab) ? params.tab[0] : params.tab
    return wanted === 'notifications' || wanted === 'advanced'
      ? wanted
      : 'general'
  }
  const setTab = (key: Tab) =>
    setParams({ tab: key === 'general' ? undefined : key }, { replace: true })
  /** How far the slider goes here: a bigger screen earns a bigger range. */
  const limits = () => bounds(window.screen.width, window.screen.height)

  /**
   * The look on demand, shared with the corner of the nav. Said here as well,
   * since a button that does nothing visible looks broken.
   */
  async function checkNow() {
    await checkUpdate()
    pushNotice('info', 'Looked. The version in the corner says what was found.')
  }

  /** What is in the file, as far as we know. */
  let saved = fingerprint(unwrap(settings()) ?? blankSettings())
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

      <div class='tabs'>
        <For
          each={
            [
              ['general', 'General'],
              ['notifications', 'Notifications'],
              ['advanced', 'Advanced'],
            ] as const
          }
        >
          {([key, label]) => (
            <button
              type='button'
              class='tab'
              classList={{ on: tab() === key }}
              onClick={() => setTab(key)}
            >
              {label}
            </button>
          )}
        </For>
      </div>
      <p class='muted'>
        Stored as JSONC you can edit by hand; the app reloads it live and keeps
        your comments.{' '}
        <button type='button' onClick={() => api.openSettingsFile()}>
          Open settings file
        </button>
      </p>

      <Show when={tab() === 'general'}>
        <fieldset>
          <legend>Interface</legend>
          <label class='row'>
            Size
            <input
              type='range'
              min={limits().min}
              max={limits().max}
              step={5}
              value={uiScale()}
              onInput={(e) => setScale(Number(e.currentTarget.value))}
            />
            <output>{uiScale()}%</output>
          </label>
          <p class='muted'>
            Holding Ctrl and turning the mouse wheel does this anywhere in the
            window, and Ctrl+0 puts it back. Each screen is remembered on its
            own, so plugging in a monitor does not resize the laptop.
          </p>
          <button type='button' onClick={() => resetScale()}>
            Reset
          </button>
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
          <legend>Connection</legend>
          <label>
            Disconnect after this many minutes without a key or click (0 never)
            <input
              type='number'
              min='0'
              value={draft.connection.idleDisconnectMinutes}
              onInput={(e) => {
                const minutes = wholeNumber(e.currentTarget.value)
                if (minutes !== null)
                  setDraft('connection', 'idleDisconnectMinutes', minutes)
              }}
            />
          </label>
          <p class='muted'>
            A lost connection comes back on its own, which is right while you
            are here and wrong for a window you forgot: it keeps a seat in a
            room for nobody. Past this the connection is dropped and stays
            dropped; the window stays, one click from logging in again. A
            running game never counts as idle.
          </p>
        </fieldset>

        <fieldset>
          <legend>Updates</legend>
          <label class='row'>
            <input
              type='checkbox'
              checked={draft.updates.automatic}
              onChange={(e) =>
                setDraft('updates', 'automatic', e.currentTarget.checked)
              }
            />
            Look for a newer version once a day
          </label>
          <p class='muted'>
            One small request when the app opens, at most once a day; nothing is
            downloaded by itself. A newer version shows on the version in the
            corner of the nav, and a click there fetches and installs it. In a
            room or a game the install waits for a second click.{' '}
            <button
              type='button'
              disabled={updating()}
              onClick={() => void checkNow()}
            >
              {updating() ? 'Looking…' : 'Check now'}
            </button>
          </p>
        </fieldset>

        <fieldset>
          <legend>Overlay</legend>
          <label class='row'>
            <input
              type='checkbox'
              checked={draft.overlay.enabled}
              onChange={(e) =>
                setDraft('overlay', 'enabled', e.currentTarget.checked)
              }
            />
            Raise the lobby over a running game
          </label>
          <label>
            Shortcut
            <input
              value={draft.overlay.hotkey}
              placeholder='Alt+Shift+L'
              disabled={!draft.overlay.enabled}
              onInput={(e) =>
                setDraft('overlay', 'hotkey', e.currentTarget.value)
              }
            />
          </label>
          <p class='muted'>
            Held only while a game runs, so the lobby never owns a key while it
            sits idle. While held it does beat the game, so a combination BAR
            uses would be taken away from it. The default was picked on that
            basis: BAR binds <code>L</code> plain and with Shift, and binds only
            two Alt+Shift combinations in the whole game, neither of them this
            one.
          </p>
          <label class='row'>
            <input
              type='checkbox'
              checked={draft.overlay.returnFocusToGame}
              disabled={!draft.overlay.enabled}
              onChange={(e) =>
                setDraft(
                  'overlay',
                  'returnFocusToGame',
                  e.currentTarget.checked,
                )
              }
            />
            Put the game back in front when I dismiss it
          </label>
          <label class='row'>
            <input
              type='checkbox'
              checked={draft.overlay.inGameEscape}
              disabled={!draft.overlay.enabled}
              onChange={(e) =>
                setDraft('overlay', 'inGameEscape', e.currentTarget.checked)
              }
            />
            Escape in a game opens the lobby
          </label>
          <p class='muted'>
            The engine gives an outside program no way to see Escape, so this
            writes a small widget into <code>LuaUI/Widgets</code> in the data
            directory modlobby writes — its own, unless you have pointed{' '}
            <code>paths.dataDir</code> at another lobby's install. It draws
            nothing, it comes back out when modlobby closes, and it only takes
            the key when modlobby answers — so a game you start from Chobby
            keeps its own Escape. Escape with units selected still deselects
            them, as always.
          </p>
          <p class='muted'>
            Nothing can be drawn over an exclusive full-screen game, so if your
            engine is set that way, modlobby launches it against its own copy of
            your settings with borderless full screen instead. Your{' '}
            <code>springsettings.cfg</code> is never written to — not by
            modlobby, and not by the game it starts — so Chobby finds it exactly
            as you left it.
          </p>
        </fieldset>

        <fieldset>
          <legend>Playing</legend>
          <label class='row'>
            <input
              type='checkbox'
              checked={draft.play.autoDownload}
              onChange={(e) =>
                setDraft('play', 'autoDownload', e.currentTarget.checked)
              }
            />
            Download what a room needs automatically
          </label>
          <p class='muted'>
            Engine, game and map, as soon as you join a room that lacks them.
            Off leaves a button in the room for each, for a metered connection
            or a disk being kept small.
          </p>
        </fieldset>

        <fieldset>
          <legend>Joining</legend>
          <p class='muted'>
            Clicking a room in the list joins it. This is what that means.
          </p>
          <div
            class='choice-row'
            title='Remember last does what you did in the room before this one — take a seat and it plays, spectate and it watches.'
          >
            <span>Join rooms as</span>
            <div class='choice'>
              <For
                each={
                  [
                    ['remember', 'Remember last'],
                    ['spectator', 'Always spectator'],
                    ['player', 'Always player'],
                  ] as const
                }
              >
                {([how, label]) => (
                  <button
                    type='button'
                    classList={{ on: draft.play.joinAs === how }}
                    onClick={() => setDraft('play', 'joinAs', how)}
                  >
                    {label}
                  </button>
                )}
              </For>
            </div>
          </div>
        </fieldset>

        <fieldset>
          <legend>Paths</legend>
          <label>
            BAR data directory to write (blank = modlobby's own; the launcher's
            and bar-lobby's are always read)
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
          <PlayerFiles />
        </fieldset>
      </Show>

      <Show when={tab() === 'notifications'}>
        <fieldset>
          <legend>Notifications</legend>
          <label class='row'>
            <input
              type='checkbox'
              checked={draft.notifications.doNotDisturb}
              onChange={(e) =>
                setDraft(
                  'notifications',
                  'doNotDisturb',
                  e.currentTarget.checked,
                )
              }
            />
            Do not disturb
          </label>
          <p class='muted'>
            Silences every row below without changing any of them, for when the
            lobby is open beside something that matters more. The corner still
            shows what modlobby itself has to say — an error, a download — since
            that is an answer to something you did.
          </p>
          <p class='muted'>
            <b>In lobby</b> puts a line in the corner of this window.{' '}
            <b>Desktop</b> raises a notification from your operating system and
            flashes modlobby in the taskbar, while it is in the background — and
            nothing at all while you are looking at it, since you are already
            here. The two never both happen.
          </p>
          <For
            each={
              [
                [
                  'privateMessage',
                  'A direct message',
                  'Someone sends you a private message. Your own messages never count.',
                ],
                [
                  'mention',
                  'Someone says my name',
                  'Your name appears in a channel or in your battle room, as a word rather than inside a longer one.',
                ],
                [
                  'ring',
                  'Someone rings me',
                  'Someone in your room rings you, which is how a host says the game is waiting on you.',
                ],
                [
                  'friendOnline',
                  'A friend comes online',
                  'Someone on your friends list logs in. Never raised for the crowd that arrives when you log in yourself.',
                ],
                [
                  'vote',
                  'A vote opens in my room',
                  'A vote is called in the room you are in — a map change, a balance, a start.',
                ],
                [
                  'gameStarting',
                  "My room's game starts",
                  'The host of your room goes in-game, which is the moment you can connect to it.',
                ],
                [
                  'gameEnded',
                  "My room's game finishes",
                  'The host comes back out of the game, which is when the room starts filling for the next one.',
                ],
              ] as const
            }
          >
            {([key, label, hint]) => (
              <div class='choice-row' title={hint}>
                <span>{label}</span>
                <div class='choice'>
                  <For
                    each={
                      [
                        ['off', 'Off'],
                        ['lobby', 'In lobby'],
                        ['desktop', 'Desktop'],
                      ] as const
                    }
                  >
                    {([where, name]) => (
                      <button
                        type='button'
                        classList={{ on: draft.notifications[key] === where }}
                        onClick={() => setDraft('notifications', key, where)}
                      >
                        {name}
                      </button>
                    )}
                  </For>
                </div>
              </div>
            )}
          </For>
        </fieldset>
      </Show>

      <Show when={tab() === 'advanced'}>
        <p class='muted'>
          Things you should not need. The defaults are what the game's own
          server expects, and these are the switches behind them.
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
          <legend>Chat log</legend>
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
          <legend>Chat</legend>
          <label class='row'>
            <input
              type='checkbox'
              checked={draft.chat.filterHostChatter}
              onChange={(e) =>
                setDraft('chat', 'filterHostChatter', e.currentTarget.checked)
              }
            />
            Filter bot chatter
          </label>
          <p class='muted'>
            SPADS rides the room's state on battle chat as
            <code> BarManager|&#123;…&#125;</code>, which this reads and turns
            into the room you see. Off, those lines are shown as they arrive.
          </p>
        </fieldset>

        <fieldset>
          <legend>Playing</legend>
          <label class='row'>
            <input
              type='checkbox'
              checked={draft.play.autoLaunch}
              onChange={(e) =>
                setDraft('play', 'autoLaunch', e.currentTarget.checked)
              }
            />
            Start the game automatically
          </label>
          <p class='muted'>
            When your room's game starts, the engine starts with it — while
            spectating too, which is otherwise the case that means watching the
            room and pressing a button. Never without the content on disk.
          </p>
          <label class='row'>
            <input
              type='checkbox'
              checked={draft.play.pveStats}
              onChange={(e) =>
                setDraft('play', 'pveStats', e.currentTarget.checked)
              }
            />
            Show what a PvE room scores (pve.bar)
          </label>
          <p class='muted'>
            Asks BAR's PvE Stats service — the one the in-game widget uses — for
            a challenge score and win chance, and lists the room among the games
            being played. Sends the map, the settings and the team size; never a
            name or an account. Off hides the panel and sends nothing.
          </p>
        </fieldset>

        <fieldset>
          <legend>Logs</legend>
          <p class='muted'>
            Both the Rust side and the webview console write one JSON-per-line
            file per day, kept across restarts. What is written is the{' '}
            <code>logging.filter</code> in the settings file.
          </p>
          <button type='button' onClick={() => api.openLogDir()}>
            Open log folder
          </button>
        </fieldset>
      </Show>
    </form>
  )
}

/**
 * A whole settings object with nothing chosen in it.
 *
 * Exported because it is the one place the shape is written out in full, and
 * a test that needs a settings object needs this one rather than a copy that
 * drifts from it.
 */
export function blankSettings(): Settings {
  return {
    $schema: null,
    server: { host: '', port: 8201, tls: true },
    account: { username: '', rememberPassword: false, autoLogin: false },
    connection: { idleDisconnectMinutes: 60 },
    paths: { dataDir: null },
    play: {
      joinAs: 'remember',
      lastWasPlayer: true,
      autoLaunch: true,
      autoDownload: true,
      pveStats: true,
    },
    notifications: {
      privateMessage: 'desktop',
      mention: 'desktop',
      ring: 'desktop',
      friendOnline: 'lobby',
      vote: 'lobby',
      gameStarting: 'desktop',
      gameEnded: 'lobby',
      doNotDisturb: false,
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
    chat: { filterHostChatter: true, maxLines: 3000, channels: ['main'] },
    overlay: {
      enabled: true,
      hotkey: 'Alt+Shift+L',
      returnFocusToGame: true,
      inGameEscape: true,
    },
    tweaks: { styluaConfig: null, defaultSlot: 'tweakdefs1' },
    logging: { filter: 'info' },
    updates: { automatic: true },
    ui: { scale: {} },
  }
}
