import { useSearchParams } from '@solidjs/router'
import {
  For,
  Show,
  createEffect,
  createSignal,
  onCleanup,
  type ParentProps,
} from 'solid-js'
import { createStore, reconcile, unwrap } from 'solid-js/store'
import { PlayerFiles } from '../components/PlayerFiles'
import { SearchBox } from '../components/SearchBox'
import { Heading, Query, Row } from '../components/SettingRow'
import type { Settings } from '../ipc/bindings/Settings'
import { api, describeError } from '../ipc/client'
import { build } from '../store/build'
import { pushNotice } from '../store/chat'
import { bounds } from '../lib/scale'
import {
  applySettings,
  resetScale,
  setScale,
  settings,
  uiScale,
} from '../store/settings'
import { busy as updating, checkUpdate, checking } from '../store/update'
import { ServerRows } from './Servers'

/** Long enough that typing a hostname is one write rather than twelve. */
const SAVE_AFTER = 600

/**
 * Every section, in the order the page shows them, by the name its heading,
 * the overview and a `?section=` link know it by; the overview is drawn
 * in this order, so it has to match the page's.
 */
const SECTIONS = {
  account: 'Account',
  notifications: 'Notifications',
  room: 'Room',
  interface: 'Interface',
  overlay: 'Overlay',
  updates: 'Updates',
  servers: 'Servers',
  connection: 'Connection',
  files: 'Files and logs',
  chat: 'Chat',
} as const

type SectionId = keyof typeof SECTIONS

const SECTION_IDS = Object.keys(SECTIONS) as SectionId[]

/** Where the page opens, and what the overview marks until you scroll. */
const TOP: SectionId = 'account'

/** A section's element id, which is also what `?section=` names. */
const anchor = (id: SectionId) => `settings-${id}`

/**
 * How far below the top of the page a section's heading has to have scrolled
 * for the overview to call that section the one being read.
 */
const READ_LINE = 48

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

/**
 * One titled part of the page: its heading out on the left over a rule
 * across the page, and its rows set in beneath. No box: the rule is what
 * parts one section from the next, and the heading is what the eye runs
 * down the page by.
 */
function Section(props: ParentProps<{ id: SectionId }>) {
  return (
    <Heading.Provider value={SECTIONS[props.id]}>
      <section id={anchor(props.id)} class='set-section'>
        <h2>{SECTIONS[props.id]}</h2>
        <div class='set-rows'>{props.children}</div>
      </section>
    </Heading.Provider>
  )
}

export function SettingsView() {
  const [draft, setDraft] = createStore<Settings>(
    structuredClone(unwrap(settings())) ?? blankSettings(),
  )
  const [state, setState] = createSignal<'clean' | 'saving' | 'saved'>('clean')
  const [params] = useSearchParams()
  const [query, setQuery] = createSignal('')
  const searching = () => query().trim() !== ''
  const [reading, setReading] = createSignal<SectionId>(TOP)
  let page: HTMLFormElement | undefined

  const sectionOf = (id: SectionId) =>
    page?.querySelector<HTMLElement>(`#${anchor(id)}`)

  /**
   * The section being read: the last one whose heading has scrolled past the
   * read line, or the last of all once the page will not scroll further — a
   * short final section never reaches the line.
   */
  function spy() {
    if (!page) return
    const atEnd = page.scrollTop + page.clientHeight >= page.scrollHeight - 1
    const line = page.getBoundingClientRect().top + READ_LINE
    const passed = SECTION_IDS.filter((id) => {
      const top = sectionOf(id)?.getBoundingClientRect().top
      return top !== undefined && (atEnd || top <= line)
    })
    setReading(passed.at(-1) ?? TOP)
  }

  /**
   * Set while the page scrolls to where the overview sent it. That scroll is
   * not the reader's, and near the foot of the page it stops short of putting
   * the section at the top, so it would otherwise mark a neighbour instead of
   * the entry that was clicked. The reader's own wheel takes over at once.
   */
  let jumping = false

  /**
   * The section under the pointer while it is over the page. What a hand is
   * resting on is what is being read, and it says so before any scroll does;
   * off the page, the scroll position answers instead.
   */
  const [pointed, setPointed] = createSignal<SectionId | null>(null)
  const marked = () => pointed() ?? reading()

  function point(event: MouseEvent) {
    const id = (event.target as Element)
      .closest('.set-section')
      ?.id.replace(/^settings-/, '')
    // Between two sections, or over the page's own title: still where it
    // was. Letting go there would flash the scrolled-to section on every
    // move from one section to the next.
    if (id === undefined || !(id in SECTIONS)) return
    setPointed(id as SectionId)
  }

  function jump(id: SectionId, behavior: ScrollBehavior) {
    jumping = true
    sectionOf(id)?.scrollIntoView({ block: 'start', behavior })
    setReading(id)
  }
  /** How far the slider goes here: a bigger screen earns a bigger range. */
  const limits = () => bounds(window.screen.width, window.screen.height)

  /**
   * The corner notices link straight to a section, so a link has to be able
   * to say which one it means.
   */
  createEffect(() => {
    const wanted = Array.isArray(params.section)
      ? params.section[0]
      : params.section
    if (wanted && wanted in SECTIONS) jump(wanted as SectionId, 'instant')
  })

  /**
   * The look on demand, shared with the corner of the nav. Said here as well,
   * since a button that does nothing visible looks broken.
   */
  async function checkNow() {
    await checkUpdate()
    pushNotice(
      'info',
      'Looked. If there is a newer version, the nav offers it.',
    )
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
    // Reconciled, not replaced: our own save comes back through here while
    // its field is still being typed in, and a server card made anew for a
    // list that only looks new takes the focus and the half-typed ports away.
    setDraft(reconcile(structuredClone(unwrap(current))))
  })

  // Reading the whole draft is what subscribes this to every field in it.
  createEffect(() => {
    const now = fingerprint(draft)
    if (now === saved) return
    clearTimeout(pending)
    pending = setTimeout(() => void save(), SAVE_AFTER)
  })

  onCleanup(() => clearTimeout(pending))

  /**
   * Writes what is waiting to be written, now. For what cannot act on a
   * draft: logging in to a server added a moment ago asks Rust about a
   * server the file does not have yet.
   */
  async function settle() {
    clearTimeout(pending)
    if (fingerprint(draft) !== saved) await save()
  }

  async function save() {
    setState('saving')
    try {
      const written = await api.updateSettings(structuredClone(unwrap(draft)))
      saved = fingerprint(written)
      applySettings(written)
      setState('saved')
    } catch (error) {
      // The draft keeps the rejected value so it can be corrected rather than
      // silently reverted; the file still holds the last good one. A warning:
      // a value typed wrong is not the app's bug.
      setState('clean')
      pushNotice('warning', describeError(error))
    }
  }

  return (
    <form
      ref={page}
      class='settings'
      onSubmit={(event) => event.preventDefault()}
      onScroll={() => jumping || spy()}
      onScrollEnd={() => (jumping = false)}
      onWheel={() => (jumping = false)}
    >
      {/* The whole page at a glance, out in the margin: every section, the
          one being read marked, and so how far there is still to go. While
          searching, the page itself is the list of what matched. */}
      <Show when={!searching()}>
        <nav class='settings-overview' aria-label='Settings sections'>
          <For each={SECTION_IDS}>
            {(id) => (
              <button
                type='button'
                classList={{ on: marked() === id }}
                aria-current={marked() === id ? 'location' : undefined}
                onClick={() => jump(id, 'smooth')}
              >
                {SECTIONS[id]}
              </button>
            )}
          </For>
        </nav>
      </Show>
      <Query.Provider value={query}>
        <div
          class='settings-body'
          onMouseOver={point}
          onMouseLeave={() => setPointed(null)}
        >
          <header class='settings-head'>
            <div>
              <h1>
                Settings
                <Show when={state() !== 'clean'}>
                  <span class='saved-mark'>
                    {state() === 'saving' ? 'saving…' : 'saved'}
                  </span>
                </Show>
              </h1>
              <p class='muted'>
                Stored as JSONC you can edit by hand; the app reloads it live
                and keeps your comments.{' '}
                <button type='button' onClick={() => api.openSettingsFile()}>
                  Open settings file
                </button>
              </p>
            </div>
            <SearchBox
              placeholder='Search settings'
              value={query()}
              onInput={setQuery}
            />
          </header>
          <p class='muted settings-none'>Nothing here matches that.</p>

          <Section id='account'>
            <Row>
              <label class='row'>
                <input
                  type='checkbox'
                  checked={draft.account.rememberPassword}
                  onChange={(e) => {
                    setDraft(
                      'account',
                      'rememberPassword',
                      e.currentTarget.checked,
                    )
                    if (!e.currentTarget.checked)
                      setDraft('account', 'autoLogin', false)
                  }}
                />
                Remember passwords (OS keyring)
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
              <p class='muted'>
                For every server at once: each keeps its own account and
                password, and these decide whether any of them are kept and used
                at startup. A password never goes in the settings file.
                Forgettable on the server's setting.
              </p>
            </Row>
          </Section>

          <Section id='notifications'>
            <Row>
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
                Silences every row below without changing any of them, for when
                the lobby is open beside something that matters more. The corner
                still shows what modlobby itself has to say — an error, a
                download — since that is an answer to something you did.
              </p>
            </Row>
            <Row>
              <p class='muted'>
                <b>In lobby</b> puts a line in the corner of this window.{' '}
                <b>Desktop</b> raises a notification from your operating system
                and flashes modlobby in the taskbar, while it is in the
                background — and nothing at all while you are looking at it,
                since you are already here. The two never both happen.
              </p>
            </Row>
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
                <Row>
                  <div class='choice-row'>
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
                            classList={{
                              on: draft.notifications[key] === where,
                            }}
                            onClick={() =>
                              setDraft('notifications', key, where)
                            }
                          >
                            {name}
                          </button>
                        )}
                      </For>
                    </div>
                  </div>
                  <p class='muted'>{hint}</p>
                </Row>
              )}
            </For>
          </Section>

          <Section id='room'>
            <Row>
              <div class='choice-row'>
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
              <p class='muted'>
                Clicking a room in the list joins it, and this is what that
                means. Remember last does what you did in the room before this
                one — take a seat and it plays, spectate and it watches.
              </p>
            </Row>
            <Row>
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
                Engine, game and map, as soon as you join a room that lacks
                them. Off leaves a button in the room for each, for a metered
                connection or a disk being kept small.
              </p>
            </Row>
            <Row>
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
                spectating too, which is otherwise the case that means watching
                the room and pressing a button. Never without the content on
                disk.
              </p>
            </Row>
            <Row>
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
                Asks the pve.bar stats service — the one the in-game widget uses
                — for a challenge score and win chance, and lists the room among
                the games being played. Sends the map, the settings and the team
                size; never a name or an account. Off hides the panel and sends
                nothing.
              </p>
            </Row>
          </Section>

          <Section id='interface'>
            <Row>
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
                Holding Ctrl and turning the mouse wheel does this anywhere in
                the window, and Ctrl+0 puts it back. Each screen is remembered
                on its own, so plugging in a monitor does not resize the laptop.
              </p>
              <button type='button' onClick={() => resetScale()}>
                Reset
              </button>
            </Row>
          </Section>

          <Section id='overlay'>
            <Row>
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
                Held only while a game runs, so the lobby never owns a key while
                it sits idle. While held it does beat the game, so a combination
                BAR uses would be taken away from it. The default was picked on
                that basis: BAR binds <code>L</code> plain and with Shift, and
                binds only two Alt+Shift combinations in the whole game, neither
                of them this one.
              </p>
              <p class='muted'>
                Nothing can be drawn over an exclusive full-screen game, so if
                your engine is set that way, modlobby launches it against its
                own copy of your settings with borderless full screen instead.
                Your <code>springsettings.cfg</code> is never written to — not
                by modlobby, and not by the game it starts — so Chobby finds it
                exactly as you left it.
              </p>
            </Row>
            <Row>
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
            </Row>
            <Row>
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
                The engine gives an outside program no way to see Escape, so
                this writes a small widget into <code>LuaUI/Widgets</code> in
                the data directory modlobby writes — its own, unless you have
                pointed <code>paths.dataDir</code> at another lobby's install.
                It draws nothing, it comes back out when modlobby closes, and it
                only takes the key when modlobby answers — so a game you start
                from Chobby keeps its own Escape. Escape with units selected
                still deselects them, as always.
              </p>
            </Row>
          </Section>

          <Section id='updates'>
            <Row>
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
              <label class='row'>
                <input
                  type='checkbox'
                  checked={draft.updates.download}
                  onChange={(e) =>
                    setDraft('updates', 'download', e.currentTarget.checked)
                  }
                />
                Download a newer version as soon as one is found
              </label>
              <p class='muted'>
                One small request when the app opens, at most once a day. A
                newer version puts a button beside the version in the nav. With
                downloading on it is fetched in the background and kept, nothing
                is installed, and the button restarts into it; the next start
                installs it if you do not. Off, nothing is fetched until you
                click, and the click fetches first. In a room or a game the
                install waits for a second click. After a session that ended
                badly the look comes round more often for a while — hourly at
                first, easing back to daily — so a fix reaches you sooner;
                nothing about the failure is sent anywhere.{' '}
                <button
                  type='button'
                  disabled={updating()}
                  onClick={() => void checkNow()}
                >
                  {checking() ? 'Looking…' : 'Check now'}
                </button>
              </p>
            </Row>
          </Section>

          <Section id='servers'>
            <ServerRows draft={draft} setDraft={setDraft} settle={settle} />
          </Section>

          <Section id='connection'>
            <Row>
              <label>
                Disconnect after this many minutes without a key or click (0
                never)
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
                A lost connection comes back on its own, which is right while
                you are here and wrong for a window you forgot: it keeps a seat
                in a room for nobody. Past this the connection is dropped and
                stays dropped; the window stays, one click from logging in
                again. A running game never counts as idle.
              </p>
            </Row>
          </Section>

          <Section id='files'>
            <Row>
              <label>
                BAR data directory to write (blank = modlobby's own; the
                launcher's and bar-lobby's are always read)
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
            </Row>
            <Row>
              <PlayerFiles />
            </Row>
            <Row>
              <p class='muted'>
                Logs: both the Rust side and the webview console write one
                JSON-per-line file per day, kept across restarts. What is
                written is the <code>logging.filter</code> in the settings file.
              </p>
              <button type='button' onClick={() => api.openLogDir()}>
                Open log folder
              </button>
            </Row>
          </Section>

          <Section id='chat'>
            <Row>
              <label class='row'>
                <input
                  type='checkbox'
                  checked={draft.chat.filterHostChatter}
                  onChange={(e) =>
                    setDraft(
                      'chat',
                      'filterHostChatter',
                      e.currentTarget.checked,
                    )
                  }
                />
                Filter bot chatter
              </label>
              <p class='muted'>
                SPADS rides the room's state on battle chat as
                <code> BarManager|&#123;…&#125;</code>, which this reads and
                turns into the room you see. Off, those lines are shown as they
                arrive.
              </p>
            </Row>
            <Row>
              <label>
                Lines kept in each chat log
                <input
                  type='number'
                  value={draft.chat.maxLines}
                  onInput={(e) => {
                    const lines = counted(e.currentTarget.value)
                    if (lines !== null) setDraft('chat', 'maxLines', lines)
                  }}
                />
              </label>
            </Row>
          </Section>

          {/* The build, at the foot of the page: the one fact a bug report needs. */}
          <Show when={build()}>
            {(found) => (
              <p class='build-stamp'>
                modlobby {found().version}+{found().commit}
              </p>
            )}
          </Show>
        </div>
      </Query.Provider>
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
    servers: [],
    account: { rememberPassword: false, autoLogin: false },
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
      friendOnline: 'off',
      vote: 'lobby',
      gameStarting: 'desktop',
      gameEnded: 'desktop',
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
    chat: {
      filterHostChatter: true,
      maxLines: 3000,
      muted: ['main'],
    },
    overlay: {
      enabled: true,
      hotkey: 'Alt+Shift+L',
      returnFocusToGame: true,
      inGameEscape: true,
    },
    tweaks: { styluaConfig: null, defaultSlot: 'tweakdefs1' },
    logging: { filter: 'info' },
    updates: { automatic: true, download: true },
    ui: { scale: {} },
  }
}
