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
import { clickLeavesOverlay, escapeLeavesOverlay } from './lib/overlay'
import { api, describeError } from './ipc/client'
import type { Settings } from './ipc/bindings/Settings'
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
    onCleanup(() => {
      window.removeEventListener('keydown', keys)
      window.removeEventListener('mousedown', clicks)
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
      <nav class='nav'>
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
        <span class='spacer' />
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
      </nav>
      <main class='main'>{props.children}</main>
      <Notices />
      <PlayerMenu />
      <Show when={over()}>
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
