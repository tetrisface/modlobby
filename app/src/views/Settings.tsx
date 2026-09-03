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

/** Like `counted`, for a field where zero is a choice rather than a blank. */
function wholeNumber(text: string): number | null {
  const value = Number(text)
  return text.trim() === '' || !Number.isInteger(value) || value < 0
    ? null
    : value
}

export function SettingsView() {
  const [draft, setDraft] = createStore<Settings>(
    structuredClone(unwrap(settings())) ?? blank(),
  )
  const [state, setState] = createSignal<'clean' | 'saving' | 'saved'>('clean')
  const [tab, setTab] = createSignal<'general' | 'advanced'>('general')
  const [checking, setChecking] = createSignal(false)

  /**
   * The update check on demand. A newer version found with nothing to lose
   * installs and restarts before this returns; otherwise say what happened,
   * since a button that does nothing visible looks broken.
   */
  async function checkNow() {
    setChecking(true)
    try {
      const outcome = await api.checkUpdate()
      pushNotice(
        'info',
        outcome.phase === 'ready'
          ? `Version ${outcome.version} is downloaded; the corner of the nav restarts into it.`
          : 'This is the newest version.',
      )
    } catch (error) {
      pushNotice('error', describeError(error))
    } finally {
      setChecking(false)
    }
  }

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

      <div class='tabs'>
        <For
          each={
            [
              ['general', 'General'],
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
            Update on startup
          </label>
          <p class='muted'>
            Fetches the newest release when the app opens and restarts into it
            before you have logged in. Found while you are in a room or a game,
            it waits in the corner of the nav for a click instead.{' '}
            <button
              type='button'
              disabled={checking()}
              onClick={() => void checkNow()}
            >
              {checking() ? 'Looking…' : 'Check now'}
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
            The engine gives an outside program no way to see Escape, so this is
            the one thing that puts a file of modlobby's in your Beyond All
            Reason folder: a small widget in <code>LuaUI/Widgets</code>. It
            draws nothing, it comes back out when modlobby closes, and it only
            takes the key when modlobby answers — so a game you start from
            Chobby keeps its own Escape. Escape with units selected still
            deselects them, as always.
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
          <legend>Notifications</legend>
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
      </Show>

      <Show when={tab() === 'advanced'}>
        <p class='muted'>
          Things you should not need. The defaults are what the game's own
          server expects, and a seat in a public room is a real player's game.
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
          <legend>Playing</legend>
          <label class='row'>
            <input
              type='checkbox'
              checked={draft.play.pveStats}
              onChange={(e) =>
                setDraft('play', 'pveStats', e.currentTarget.checked)
              }
            />
            Show what a PvE room scores
          </label>
          <p class='muted'>
            Asks BAR's PvE Stats service — the one the in-game widget uses — for
            a challenge score and win chance. Sends the map, the settings and
            the team size; never a name or an account.
          </p>

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
            On, because this is a lobby. Turn it off to watch only — a room of
            your own is yours to sit in either way.
          </p>
        </fieldset>
      </Show>
    </form>
  )
}

function blank(): Settings {
  return {
    $schema: null,
    server: { host: '', port: 8201, tls: true },
    account: { username: '', rememberPassword: false, autoLogin: false },
    connection: { idleDisconnectMinutes: 60 },
    paths: { dataDir: null },
    play: {
      inPublicRooms: true,
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
  }
}
