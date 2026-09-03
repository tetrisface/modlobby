import { A, HashRouter, Navigate, Route } from '@solidjs/router'
import { listen } from '@tauri-apps/api/event'
import {
  For,
  Show,
  createEffect,
  createSignal,
  onCleanup,
  onMount,
  type ParentProps,
} from 'solid-js'
import { IconSprite } from './components/icons'
import { PlayerMenu } from './components/PlayerMenu'
import { connectChannel } from './ipc/channel'
import { ACTIVITY_EVENTS, activityReporter } from './lib/activity'
import { clickLeavesOverlay, escapeLeavesOverlay } from './lib/overlay'
import { api, describeError } from './ipc/client'
import type { Settings } from './ipc/bindings/Settings'
import type { UpdateProgress } from './ipc/bindings/UpdateProgress'
import type { VersionView } from './ipc/bindings/VersionView'
import { chat, pushNotice } from './store/chat'
import { lobby } from './store/lobby'
import { applySettings } from './store/settings'
import { BattleList } from './views/BattleList'
import { Chat } from './views/Chat'
import { Login } from './views/Login'
import { Presets } from './views/Presets'
import { Replays } from './views/Replays'
import { Room } from './views/Room'
import { SettingsView } from './views/Settings'
import { Skirmish } from './views/Skirmish'

type SettingsEvent = { changed: Settings } | { invalid: string }

function Layout(props: ParentProps) {
  /**
   * Everything unread, anywhere. A notification is only raised while the
   * window is in the background, so without this a message that arrives while
   * you are reading the battle list leaves no mark at all.
   */
  const unread = () =>
    Object.values(chat.unread).reduce((total, count) => total + count, 0)
  const named = () => Object.values(chat.named).some(Boolean)

  /**
   * The window controls in the nav's corner.
   *
   * They earn their place because the OS title bar is not a given here — a
   * transparent window on Windows can lose its frame, and fullscreen has no
   * chrome at all — and a window that cannot be un-fullscreened or closed from
   * inside itself is a trap.
   */
  const [fullscreen, setFullscreen] = createSignal(false)
  onMount(
    () =>
      void api
        .isFullscreen()
        .then(setFullscreen)
        .catch(() => {}),
  )

  /**
   * Which build this is, in the corner where the brand is. It becomes the
   * offer to restart into a newer one when an update was downloaded while a
   * room or a game made installing it at once the wrong thing to do.
   */
  const [version, setVersion] = createSignal<VersionView | null>(null)
  const [update, setUpdate] = createSignal<UpdateProgress | null>(null)
  onMount(() => {
    void api
      .appVersion()
      .then(setVersion)
      .catch(() => {})
    const pending = listen<UpdateProgress>('app-update', (event) =>
      setUpdate(event.payload),
    )
    onCleanup(() => void pending.then((unlisten) => unlisten()))
  })
  const waiting = () => {
    const at = update()
    return at?.phase === 'ready' ? at.version : null
  }
  const fetching = () => {
    const at = update()
    return at?.phase === 'checking' || at?.phase === 'downloading'
  }
  async function installUpdate() {
    try {
      await api.installUpdate()
    } catch (error) {
      pushNotice('error', describeError(error))
    }
  }

  /** What the server says about us, which is what everyone else can see. */
  const away = () => (lobby.me ? lobby.users[lobby.me]?.status.away : false)

  /**
   * Whether the window is sitting over a running game.
   *
   * Over a game the page dresses as a modal: a centred card on a see-through
   * scrim, and every way out a modal has — Esc, a click on the scrim, an X —
   * hands the game back. Seeded by asking, because a webview that reloads
   * mid-overlay was not there for the event.
   */
  const [over, setOver] = createSignal(false)
  // Quitting a game by mis-clicking once would be unforgivable, so the
  // button asks again. The doubt resets whenever the overlay comes or goes.
  const [confirming, setConfirming] = createSignal<'leave' | 'quit' | null>(
    null,
  )
  createEffect(() => {
    over()
    setConfirming(null)
  })

  /**
   * Two clicks for anything that ends a game in progress.
   *
   * The first arms it and the second does it, and arming one disarms the
   * other — so a mis-click never ends a match, and never quits the lobby.
   */
  function guarded(which: 'leave' | 'quit', run: () => void) {
    if (confirming() !== which) {
      setConfirming(which)
      return
    }
    setConfirming(null)
    run()
  }
  onMount(() => {
    void api.overlayActive().then(setOver)
    const pending = listen<boolean>('overlay', (event) =>
      setOver(event.payload),
    )
    onCleanup(() => void pending.then((unlisten) => unlisten()))

    const keys = (event: KeyboardEvent) => {
      if (over() && escapeLeavesOverlay(event)) void api.overlayToggle()
    }
    const clicks = (event: MouseEvent) => {
      if (over() && clickLeavesOverlay(event.target)) void api.overlayToggle()
    }
    window.addEventListener('keydown', keys)
    window.addEventListener('mousedown', clicks)
    // The idle disconnect counts from the last of these. Passive: none of
    // them is prevented, and a scroll must not wait on IPC.
    const touch = activityReporter(() => void api.activity().catch(() => {}))
    for (const event of ACTIVITY_EVENTS)
      window.addEventListener(event, touch, { passive: true })
    onCleanup(() => {
      window.removeEventListener('keydown', keys)
      window.removeEventListener('mousedown', clicks)
      for (const event of ACTIVITY_EVENTS)
        window.removeEventListener(event, touch)
    })
  })
  createEffect(() =>
    document.documentElement.classList.toggle('overlay', over()),
  )

  /**
   * Channels are rejoined once a session.
   *
   * The server forgets your channels the moment you disconnect, so a client
   * that does not remember them leaves you rejoining `#main` by hand on every
   * launch. The list is read from the file here rather than from the settings
   * signal: this runs the instant the lobby is ready, and a signal that has
   * not arrived yet looks exactly like a user with no channels.
   *
   * What gets written back is driven by joining and leaving, never by a diff
   * of what happens to be open — that would save an empty list in the moment
   * between asking to join and being let in, which is precisely when this
   * effect fires.
   */
  let restored = false

  createEffect(() => {
    if (lobby.phase !== 'ready' || restored) return
    restored = true
    void (async () => {
      try {
        const saved = await api.getSettings()
        for (const name of saved.chat.channels) {
          if (!(name in chat.channels)) await api.joinChannel(name, null)
        }
      } catch (error) {
        pushNotice('warning', describeError(error))
      }
    })()
  })

  onMount(async () => {
    try {
      applySettings(await api.getSettings())
      await connectChannel()
    } catch (error) {
      pushNotice('error', describeError(error))
    }
  })
  onMount(() => {
    // `listen` resolves after a round trip. Registering the cleanup on the
    // promise keeps it inside this component's owner — awaiting first would
    // leave the owner behind and the listener would never be removed.
    const pending = listen<SettingsEvent>('settings', (event) => {
      if ('changed' in event.payload) {
        applySettings(event.payload.changed)
        pushNotice('info', 'settings reloaded')
      } else {
        pushNotice(
          'warning',
          `settings file not applied: ${event.payload.invalid}`,
        )
      }
    })
    onCleanup(() => void pending.then((unlisten) => unlisten()))
  })

  return (
    <div class='shell'>
      <IconSprite />
      {/* The nav doubles as the title bar, because the transparent window
          has none: empty nav space drags the window, double-click maximizes.
          Only in an ordinary window — a fullscreen or overlaid one is not a
          thing to drag around. */}
      <nav
        class='nav'
        data-tauri-drag-region={!fullscreen() && !over() ? true : undefined}
      >
        <span class='brand'>modlobby</span>
        <A href='/skirmish'>Skirmish</A>
        <Show when={lobby.phase === 'ready'}>
          <A href='/battles'>Battles</A>
          {/* Always in this slot, whether or not there is a room to go to.
              Rendering it only when in one made every link to its right jump
              sideways the moment you joined. */}
          <Show
            when={lobby.myBattle}
            fallback={<span class='nav-absent'>Room</span>}
          >
            <A href='/room'>Room</A>
          </Show>
          <A href='/chat'>
            Chat
            <Show when={unread() > 0}>
              <span class='badge' classList={{ named: named() }}>
                {unread()}
              </span>
            </Show>
          </A>
          <A href='/replays'>Replays</A>
          <A href='/presets'>Presets</A>
        </Show>
        <A href='/settings'>Settings</A>
        <span
          class='spacer'
          data-tauri-drag-region={!fullscreen() && !over() ? true : undefined}
        />
        {/* Small and out of the way, level with the account: the
            build is worth a glance, not a place in the row. */}
        <span class='version-slot'>
          <Show when={version()}>
            {(build) => (
              <Show
                when={waiting()}
                fallback={
                  <span
                    class='version'
                    title={
                      fetching()
                        ? 'Looking for a newer version…'
                        : 'Version, and the commit it was built from'
                    }
                  >
                    {build().version}+{build().commit}
                    {fetching() ? ' ↓' : ''}
                  </span>
                }
              >
                {(next) => (
                  <button
                    type='button'
                    class='version waiting'
                    title={`Version ${next()} is downloaded. Restart into it.`}
                    onClick={() => void installUpdate()}
                  >
                    {build().version} → {next()} ⟳
                  </button>
                )}
              </Show>
            )}
          </Show>
        </span>
        <Show
          when={lobby.me}
          fallback={<span class='muted'>not logged in</span>}
        >
          <span>{lobby.me}</span>
          {/* The server keeps this bit, so what it says is what everyone else
              sees — no local guess to drift out of step with it. */}
          <button
            class='chip-choice'
            classList={{ on: away() }}
            title={
              away()
                ? 'Everyone sees you as away'
                : 'Tell everyone you have stepped out'
            }
            onClick={() => void api.setAway(!away())}
          >
            Away
          </button>
          <button onClick={() => api.logout()}>Log out</button>
        </Show>
        <div class='win-controls'>
          <button
            class='win-btn'
            title={fullscreen() ? 'Windowed' : 'Full screen'}
            aria-label={fullscreen() ? 'Windowed' : 'Full screen'}
            onClick={() =>
              void api
                .toggleFullscreen()
                .then(setFullscreen)
                .catch((error) => pushNotice('warning', describeError(error)))
            }
          >
            <svg viewBox='0 0 12 12' aria-hidden='true'>
              <Show
                when={fullscreen()}
                fallback={
                  // Corners pointing out: take the whole screen.
                  <path d='M1 4V1h3M8 1h3v3M11 8v3H8M4 11H1V8' />
                }
              >
                {/* Corners pointing in: back to a window. */}
                <path d='M4 1v3H1M11 4H8V1M8 11V8h3M1 8h3v3' />
              </Show>
            </svg>
          </button>
          <button
            class='win-btn close'
            title='Close modlobby (a running game keeps going)'
            aria-label='Close modlobby'
            onClick={() => void api.shutdown()}
          >
            <svg viewBox='0 0 12 12' aria-hidden='true'>
              <path d='M2 2l8 8M10 2l-8 8' />
            </svg>
          </button>
        </div>
      </nav>
      <main class='main'>{props.children}</main>
      <Notices />
      <PlayerMenu />
      <Show when={over()}>
        {/* The three things you can do standing here, in the order you are
            likely to want them: carry on in the lobby (already true, so no
            button), go back, or stop playing. */}
        <div class='overlay-chrome'>
          <button
            class='quit'
            title='Ends the game and leaves you here in the lobby'
            onClick={() =>
              guarded(
                'leave',
                () =>
                  void api
                    .stopGame()
                    .catch((error) =>
                      pushNotice('warning', describeError(error)),
                    ),
              )
            }
          >
            {confirming() === 'leave' ? 'End the game?' : 'Leave game'}
          </button>
          <button
            class='quit'
            title='Ends the game and closes modlobby'
            onClick={() => guarded('quit', () => void api.quitAll())}
          >
            {confirming() === 'quit' ? 'Quit everything?' : 'Quit'}
          </button>
          <button
            class='primary'
            title='Or press Escape'
            onClick={() => void api.overlayToggle()}
          >
            Back to game
          </button>
        </div>
        <button
          class='overlay-close'
          title='Back to game (Esc)'
          aria-label='Back to game'
          onClick={() => void api.overlayToggle()}
        >
          ×
        </button>
      </Show>
    </div>
  )
}

function Notices() {
  return (
    <div class='notices'>
      <For each={chat.notices.slice(-3)}>
        {(notice) => <div class={`notice ${notice.level}`}>{notice.text}</div>}
      </For>
    </div>
  )
}

function Home() {
  createEffect(() => lobby.phase)
  return (
    <Show when={lobby.phase === 'ready'} fallback={<Login />}>
      <Navigate href='/battles' />
    </Show>
  )
}

export function App() {
  return (
    <HashRouter root={Layout}>
      <Route path='/' component={Home} />
      <Route path='/login' component={Login} />
      <Route path='/battles' component={BattleList} />
      <Route path='/chat' component={Chat} />
      <Route path='/replays' component={Replays} />
      <Route path='/presets' component={Presets} />
      <Route path='/skirmish' component={Skirmish} />
      <Route path='/room' component={Room} />
      {/* The tweak editor lives inside the room's setup pane, which is where
          the slots it edits are listed. It was reachable here as well, drawing
          the same component with none of that around it. */}
      <Route path='/room/tweaks' component={Room} />
      <Route path='/settings' component={SettingsView} />
    </HashRouter>
  )
}
